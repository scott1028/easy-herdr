//! Group supervisor for `easy-herdr cluster --exit-when-done`.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::api::client::ApiClientError;
use crate::api::schema::{
    AgentStatus, AgentTarget, EmptyParams, Method, PaneReadParams, PaneTarget, ReadFormat,
    ReadIntent, ReadSource, ResponseResult,
};

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// Consecutive failed polls that mean the server is gone.
const MAX_FAILURES: usize = 3;
const TIMELINE_CAP: usize = 1000;

#[derive(Debug, Clone)]
pub(super) struct WatchConfig {
    /// Members write their pane id to `<n>.ready` files here.
    pub(super) ready_dir: PathBuf,
    /// The cluster's own pane: watched, but never closed by the supervisor.
    pub(super) leader_pane: Option<String>,
    pub(super) timeout: Duration,
    pub(super) idle_exit: Duration,
    /// Background mode: snapshot live members before closing them on a timeout.
    pub(super) capture_on_timeout: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EndReason {
    Done,
    Timeout,
    AgentsExited,
    ServerGone,
    /// A stop signal reached the background supervisor.
    Stopped,
}

#[derive(Debug)]
pub(super) struct Outcome {
    pub(super) reason: EndReason,
    /// Agent panes closed at the end, the leader included.
    pub(super) closed: usize,
    /// Member pane ids in join order.
    pub(super) members: Vec<String>,
    pub(super) timeline: VecDeque<TimelineEntry>,
    /// Live members at a captured timeout, in member order.
    pub(super) snapshots: Vec<Snapshot>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Snapshot {
    pub(super) pane_id: String,
    pub(super) name: Option<String>,
    pub(super) kind: Option<String>,
    pub(super) status: Option<AgentStatus>,
    /// The last 200 lines as text.
    pub(super) screen: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct TimelineEntry {
    /// Time since the supervisor started.
    pub(super) elapsed: Duration,
    pub(super) event: String,
}

/// One member in the latest `agent.list`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MemberState {
    /// Not listed yet: still starting.
    Pending,
    /// Listed before, gone now: its agent exited.
    Gone,
    Listed {
        detected: bool,
        status: AgentStatus,
    },
}

/// Decides when every member has been idle long enough after some work.
struct IdleTracker {
    idle_exit: Duration,
    armed: bool,
    idle_since: Option<Instant>,
}

impl IdleTracker {
    fn new(idle_exit: Duration) -> Self {
        Self {
            idle_exit,
            armed: false,
            idle_since: None,
        }
    }

    /// Starts the idle period again, for example when a member joins.
    fn restart(&mut self) {
        self.idle_since = None;
    }

    /// Counts work that started and finished between two polls.
    fn arm(&mut self) {
        self.armed = true;
    }

    /// Gone members are skipped; any other member must be a detected idle or done agent.
    fn observe(&mut self, states: &[MemberState], now: Instant) -> bool {
        let is_listed = |state: &MemberState| matches!(state, MemberState::Listed { .. });
        self.armed |= states.iter().any(|state| {
            matches!(
                state,
                MemberState::Listed {
                    status: AgentStatus::Working | AgentStatus::Blocked,
                    ..
                }
            )
        });
        let is_idle = states.iter().any(is_listed)
            && states.iter().all(|state| {
                matches!(
                    state,
                    MemberState::Gone
                        | MemberState::Listed {
                            detected: true,
                            status: AgentStatus::Idle | AgentStatus::Done,
                        }
                )
            });
        if !(self.armed && is_idle) {
            self.idle_since = None;
            return false;
        }
        now.saturating_duration_since(*self.idle_since.get_or_insert(now)) >= self.idle_exit
    }
}

struct Member {
    pane_id: String,
    state: MemberState,
    /// Last recorded name and status, for timeline changes.
    last: Option<(Option<String>, AgentStatus)>,
}

/// Polls the group until it is done, times out or disappears, then closes member panes.
/// `interrupt` is checked before every poll and can end the run with its own reason.
pub(super) fn supervise(
    config: &WatchConfig,
    mut request: impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
    mut interrupt: impl FnMut() -> Option<EndReason>,
) -> Outcome {
    let start = now();
    // An overflowing timeout means no deadline.
    let deadline = start.checked_add(config.timeout);
    let mut tracker = IdleTracker::new(config.idle_exit);
    let mut members = Vec::<Member>::new();
    let mut timeline = VecDeque::new();
    let record = |timeline: &mut VecDeque<TimelineEntry>, event: String| {
        if timeline.len() == TIMELINE_CAP {
            timeline.pop_front();
        }
        timeline.push_back(TimelineEntry {
            elapsed: now().saturating_duration_since(start),
            event,
        });
    };
    let mut failures = 0;
    let reason = loop {
        if let Some(reason) = interrupt() {
            break reason;
        }
        for pane_id in config
            .leader_pane
            .iter()
            .cloned()
            .chain(read_ready_panes(&config.ready_dir))
        {
            if !members.iter().any(|member| member.pane_id == pane_id) {
                record(&mut timeline, format!("{pane_id} joined"));
                tracker.restart();
                members.push(Member {
                    pane_id,
                    state: MemberState::Pending,
                    last: None,
                });
            }
        }
        match request(Method::AgentList(EmptyParams {})) {
            Ok(ResponseResult::AgentList { agents }) => {
                failures = 0;
                for member in &mut members {
                    let event = match agents.iter().find(|agent| agent.pane_id == member.pane_id) {
                        Some(agent) => {
                            // A completed task shows the agent worked, even unseen by a poll.
                            if agent.completion_seq.is_some() {
                                tracker.arm();
                            }
                            set_listed(
                                member,
                                agent.agent.is_some(),
                                agent.name.clone(),
                                agent.agent_status,
                            )
                        }
                        None => check_unlisted(member, &mut request),
                    };
                    if let Some(event) = event {
                        record(&mut timeline, event);
                    }
                }
                let states = members
                    .iter()
                    .map(|member| member.state)
                    .collect::<Vec<_>>();
                if !states.is_empty() && states.iter().all(|state| *state == MemberState::Gone) {
                    break EndReason::AgentsExited;
                }
                if tracker.observe(&states, now()) {
                    break EndReason::Done;
                }
            }
            result => {
                if let Err(error) = result {
                    tracing::debug!(%error, "cannot poll cluster agents");
                }
                failures += 1;
                if failures == MAX_FAILURES {
                    break EndReason::ServerGone;
                }
            }
        }
        if deadline.is_some_and(|deadline| now() >= deadline) {
            break EndReason::Timeout;
        }
        sleep(POLL_INTERVAL);
    };
    let mut snapshots = Vec::new();
    if reason == EndReason::Timeout && config.capture_on_timeout {
        for member in members
            .iter()
            .filter(|member| member.state != MemberState::Gone)
        {
            snapshots.push(capture(&member.pane_id, &mut request));
        }
    }
    let mut closed = 0;
    if matches!(
        reason,
        EndReason::Done | EndReason::Timeout | EndReason::Stopped
    ) {
        closed = usize::from(config.leader_pane.is_some());
        for member in &members {
            if config.leader_pane.as_ref() == Some(&member.pane_id)
                || member.state == MemberState::Gone
            {
                continue;
            }
            let pane_id = member.pane_id.clone();
            match request(Method::PaneClose(PaneTarget { pane_id })) {
                Ok(_) => closed += 1,
                Err(error) => {
                    tracing::warn!(pane_id = %member.pane_id, %error, "cannot close cluster pane");
                }
            }
        }
    }
    Outcome {
        reason,
        closed,
        members: members.into_iter().map(|member| member.pane_id).collect(),
        timeline,
        snapshots,
    }
}

/// Reads a member's agent and recent screen; a failed request leaves its fields empty.
fn capture(
    pane_id: &str,
    request: &mut impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
) -> Snapshot {
    let agent = match request(Method::AgentGet(AgentTarget {
        target: pane_id.into(),
    })) {
        Ok(ResponseResult::AgentInfo { agent }) => Some(agent),
        _ => None,
    };
    let screen = match request(Method::PaneRead(PaneReadParams {
        pane_id: pane_id.into(),
        source: ReadSource::Recent,
        lines: Some(200),
        format: ReadFormat::Text,
        strip_ansi: true,
        intent: ReadIntent::Passive,
    })) {
        Ok(ResponseResult::PaneRead { read }) => Some(read.text),
        _ => None,
    };
    Snapshot {
        pane_id: pane_id.into(),
        name: agent.as_ref().and_then(|agent| agent.name.clone()),
        kind: agent.as_ref().and_then(|agent| agent.agent.clone()),
        status: agent.map(|agent| agent.agent_status),
        screen,
    }
}

/// Marks a member listed; returns a timeline event when its name or status changed.
fn set_listed(
    member: &mut Member,
    detected: bool,
    name: Option<String>,
    status: AgentStatus,
) -> Option<String> {
    member.state = MemberState::Listed { detected, status };
    let current = (name, status);
    if member.last.as_ref() == Some(&current) {
        return None;
    }
    let role = current.0.as_deref().unwrap_or("-");
    let status_name = format!("{status:?}").to_lowercase();
    let event = format!("{} {role} {status_name}", member.pane_id);
    member.last = Some(current);
    Some(event)
}

/// `agent.list` skips live panes without a detected agent, so `pane.get` decides
/// whether an unlisted member is gone. A failed lookup keeps the member's state.
fn check_unlisted(
    member: &mut Member,
    request: &mut impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
) -> Option<String> {
    if member.state == MemberState::Gone {
        return None;
    }
    let pane_id = member.pane_id.clone();
    match request(Method::PaneGet(PaneTarget { pane_id })) {
        Ok(ResponseResult::PaneInfo { .. }) if member.state == MemberState::Pending => None,
        Ok(ResponseResult::PaneInfo { .. }) => {
            let name = member.last.as_ref().and_then(|(name, _)| name.clone());
            set_listed(member, false, name, AgentStatus::Unknown)
        }
        Err(ApiClientError::ErrorResponse(response)) if response.error.code == "pane_not_found" => {
            member.state = MemberState::Gone;
            member.last = None;
            Some(format!("{} gone", member.pane_id))
        }
        Ok(_) => {
            tracing::debug!(pane_id = %member.pane_id, "unexpected cluster pane response");
            None
        }
        Err(error) => {
            tracing::debug!(pane_id = %member.pane_id, %error, "cannot look up cluster pane");
            None
        }
    }
}

/// Pane ids from complete `<n>.ready` files, in file name order.
fn read_ready_panes(dir: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut paths = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "ready")
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|text| text.trim().to_owned())
        .filter(|pane_id| !pane_id.is_empty())
        .collect()
}

/// Sets `stop` once the leader process is gone.
pub(super) fn spawn_leader_watch(group: u32, stop: std::sync::Arc<AtomicBool>) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-leader", move || {
        watch_leader(group, &stop, crate::platform::process_exists, || {
            std::thread::sleep(Duration::from_secs(1));
        });
    }) {
        tracing::warn!(%err, "cannot start cluster leader watch");
    }
}

fn watch_leader(
    group: u32,
    stop: &AtomicBool,
    mut leader_exists: impl FnMut(u32) -> bool,
    mut sleep: impl FnMut(),
) {
    while !stop.load(Ordering::SeqCst) && leader_exists(group) {
        sleep();
    }
    stop.store(true, Ordering::SeqCst);
}

#[cfg(test)]
#[path = "cluster_watch_tdd.rs"]
mod tdd;
