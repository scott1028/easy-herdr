//! `easy-herdr cluster --yolo / --yolo-herdr`: answers known dialogs that keep the first agent blocked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::YoloMode;

use crate::api::client::{parse_response_value, ApiClient, ApiClientError};
use crate::api::schema::{
    AgentReadParams, AgentSendKeysParams, AgentStatus, AgentTarget, Method, PaneTarget, ReadFormat,
    ReadSource, Request, ResponseResult,
};

#[path = "yolo_policy.rs"]
mod yolo_policy;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long after sending keys the dialog is checked again.
const RECHECK_DELAY: Duration = Duration::from_secs(3);
/// Key presses per blocked episode: the first one and two retries.
const MAX_ATTEMPTS: usize = 3;
/// Key presses across all dialogs in one blocked stretch.
const MAX_PRESSES: usize = 10;
/// Consecutive failed polls that mean the server is gone.
const MAX_FAILURES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WatchEnd {
    LeaderGone,
    ServerGone,
    Stopped,
}

/// One unchanged dialog within a blocked stretch.
struct Episode {
    since: Instant,
    attempts: usize,
    /// Agent kind, rule id and detection text without timer/spinner noise.
    identity: (String, String, String),
    keys: &'static [&'static str],
    /// Set after an unknown dialog or the last retry: no more keys until the episode ends.
    is_finished: bool,
}

/// Starts the watch of the first agent's pane in its own thread.
pub(super) fn spawn_watch(pane_id: String, mode: YoloMode, grace: Duration, stop: Arc<AtomicBool>) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-yolo", move || {
        let mode_name = match mode {
            YoloMode::Agent => "agent",
            YoloMode::Herdr => "herdr",
        };
        tracing::info!(pane = %pane_id, mode = mode_name, ?grace, "cluster yolo watch started");
        let client = ApiClient::local();
        let end = watch(
            &pane_id,
            mode,
            grace,
            |method| {
                client
                    .request_value_with_timeout(
                        &Request {
                            id: "cluster:yolo".into(),
                            method,
                        },
                        Duration::from_secs(5),
                    )
                    .and_then(parse_response_value)
                    .map(|response| response.result)
            },
            Instant::now,
            std::thread::sleep,
            || stop.load(Ordering::SeqCst),
        );
        tracing::info!(pane = %pane_id, ?end, "cluster yolo watch ended");
    }) {
        tracing::warn!(%err, "cannot start cluster yolo watch");
    }
}

/// Polls the first agent until its pane or the server is gone, or `is_stopped`.
/// A known dialog still blocking it after `grace` gets its policy keys.
pub(super) fn watch(
    pane_id: &str,
    mode: YoloMode,
    grace: Duration,
    mut request: impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
    is_stopped: impl Fn() -> bool,
) -> WatchEnd {
    let mut episode: Option<Episode> = None;
    let mut failures = 0;
    let mut presses = 0;
    let mut is_press_limit_logged = false;
    loop {
        if is_stopped() {
            return WatchEnd::Stopped;
        }
        let mut delay = POLL_INTERVAL;
        match request(Method::AgentGet(AgentTarget {
            target: pane_id.into(),
        })) {
            Ok(ResponseResult::AgentInfo { agent }) => {
                failures = 0;
                if agent.agent_status == AgentStatus::Blocked {
                    if let Some((kind, rule, screen)) =
                        read_dialog(pane_id, &mut request, &is_stopped)
                    {
                        let normalized = screen
                            .chars()
                            .filter(|ch| {
                                !ch.is_ascii_digit() && !('\u{2800}'..='\u{28ff}').contains(ch)
                            })
                            .collect();
                        let identity = (kind, rule, normalized);
                        if episode.as_ref().is_none_or(|old| old.identity != identity) {
                            if let Some(old) = &episode {
                                log_answered(pane_id, old);
                            }
                            tracing::debug!(
                                pane = pane_id,
                                kind = identity.0,
                                rule = identity.1,
                                "cluster yolo dialog changed"
                            );
                            episode = Some(Episode {
                                since: now(),
                                attempts: 0,
                                identity,
                                keys: &[],
                                is_finished: false,
                            });
                        }
                        if let Some(episode) = episode.as_mut().filter(|episode| {
                            !episode.is_finished
                                && now().saturating_duration_since(episode.since) >= grace
                        }) {
                            if presses == MAX_PRESSES {
                                if !is_press_limit_logged {
                                    tracing::warn!(
                                        pane = pane_id,
                                        result = "press_limit",
                                        "cluster yolo press limit reached"
                                    );
                                    is_press_limit_logged = true;
                                }
                                episode.is_finished = true;
                            } else if answer(
                                pane_id,
                                mode,
                                episode,
                                &screen,
                                &mut request,
                                &is_stopped,
                            ) {
                                presses += 1;
                                delay = RECHECK_DELAY;
                            }
                        }
                    }
                } else {
                    if let Some(old) = episode.take() {
                        log_answered(pane_id, &old);
                    }
                    presses = 0;
                    is_press_limit_logged = false;
                }
            }
            // A pane whose agent is not detected is not listed as an agent. Its next
            // block is a new dialog, so it starts a new episode.
            Err(ApiClientError::ErrorResponse(response))
                if response.error.code == "agent_not_found" =>
            {
                failures = 0;
                episode = None;
                presses = 0;
                is_press_limit_logged = false;
                if is_stopped() {
                    return WatchEnd::Stopped;
                }
                let pane_id = pane_id.to_owned();
                if matches!(
                    request(Method::PaneGet(PaneTarget { pane_id })),
                    Err(ApiClientError::ErrorResponse(response)) if response.error.code == "pane_not_found"
                ) {
                    return WatchEnd::LeaderGone;
                }
            }
            result => {
                if let Err(error) = result {
                    tracing::debug!(pane = pane_id, %error, "cannot poll the cluster yolo agent");
                }
                failures += 1;
                if failures == MAX_FAILURES {
                    return WatchEnd::ServerGone;
                }
            }
        }
        sleep(delay);
    }
}

/// Reads both rule and screen on every blocked poll, including the grace wait.
fn read_dialog(
    pane_id: &str,
    request: &mut impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
    is_stopped: &impl Fn() -> bool,
) -> Option<(String, String, String)> {
    if is_stopped() {
        return None;
    }
    let explain = match request(Method::AgentExplain(AgentTarget {
        target: pane_id.into(),
    })) {
        Ok(ResponseResult::AgentExplain { explain }) => explain,
        result => {
            tracing::debug!(
                pane = pane_id,
                ?result,
                "cannot explain the cluster yolo agent"
            );
            return None;
        }
    };
    if is_stopped() {
        return None;
    }
    let screen = match request(Method::AgentRead(AgentReadParams {
        target: pane_id.into(),
        source: ReadSource::Detection,
        lines: None,
        format: ReadFormat::Text,
        strip_ansi: true,
    })) {
        Ok(ResponseResult::PaneRead { read }) => read.text,
        result => {
            tracing::debug!(
                pane = pane_id,
                ?result,
                "cannot read the cluster yolo agent"
            );
            return None;
        }
    };
    Some((
        explain["agent"].as_str().unwrap_or("-").into(),
        explain["matched_rule"]["id"].as_str().unwrap_or("-").into(),
        screen,
    ))
}

fn log_answered(pane_id: &str, episode: &Episode) {
    if episode.attempts > 0 && !episode.is_finished {
        tracing::info!(
            pane = pane_id,
            kind = episode.identity.0,
            rule = episode.identity.1,
            keys = ?episode.keys,
            attempt = episode.attempts,
            result = "answered",
            "cluster yolo dialog answered"
        );
    }
}

/// Sends policy keys for the current dialog; returns whether a send was attempted.
/// A stop seen before any request ends the attempt, so no keys follow a stop.
fn answer(
    pane_id: &str,
    mode: YoloMode,
    episode: &mut Episode,
    screen: &str,
    request: &mut impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
    is_stopped: &impl Fn() -> bool,
) -> bool {
    let (kind, rule, _) = &episode.identity;
    let answer = yolo_policy::get_answer(kind, rule, mode);
    if episode.attempts == MAX_ATTEMPTS {
        tracing::warn!(
            pane = pane_id,
            kind,
            rule,
            attempt = episode.attempts,
            result = "gave_up",
            "cluster yolo dialog finished"
        );
        episode.is_finished = true;
        return false;
    }
    let result = if let Some(answer) = answer {
        let screen = screen.to_lowercase();
        (!answer.gates.is_empty()
            && !answer
                .gates
                .iter()
                .any(|gate| screen.contains(&gate.to_lowercase())))
        .then_some("gate_mismatch")
    } else {
        Some("unknown")
    };
    if let Some(result) = result {
        tracing::info!(
            pane = pane_id,
            kind,
            rule,
            attempt = episode.attempts,
            result,
            "cluster yolo dialog finished"
        );
        episode.is_finished = true;
        return false;
    }
    let Some(answer) = answer else {
        return false;
    };
    let keys = answer.keys;
    if is_stopped() {
        return false;
    }
    episode.attempts += 1;
    let attempt = episode.attempts;
    match request(Method::AgentSendKeys(AgentSendKeysParams {
        target: pane_id.into(),
        keys: keys.iter().map(|key| (*key).to_owned()).collect(),
    })) {
        Ok(_) => {
            tracing::info!(
                pane = pane_id,
                kind,
                rule,
                ?keys,
                attempt,
                result = "sent",
                "cluster yolo sent keys to a dialog"
            );
        }
        Err(error) => {
            tracing::warn!(pane = pane_id, kind, rule, ?keys, attempt, result = "send_failed", %error, "cluster yolo cannot send keys");
        }
    }
    episode.keys = keys;
    true
}

#[cfg(test)]
#[path = "cluster_yolo_tests.rs"]
mod tests;
