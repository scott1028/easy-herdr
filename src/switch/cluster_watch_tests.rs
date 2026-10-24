use std::cell::Cell;
use std::sync::atomic::{AtomicBool, Ordering};

use super::*;
use crate::api::schema::AgentStatus::{Blocked, Done, Idle, Unknown, Working};
use crate::api::schema::{AgentInfo, ReadSource};

fn listed(status: AgentStatus) -> MemberState {
    MemberState::Listed {
        detected: true,
        status,
    }
}

fn agent(pane_id: &str, status: AgentStatus) -> AgentInfo {
    serde_json::from_value(serde_json::json!({
        "terminal_id": format!("term_{pane_id}"), "agent": "claude", "agent_status": status,
        "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": pane_id, "focused": false,
        "revision": 1,
    }))
    .unwrap()
}

#[test]
fn idle_tracker_waits_for_work_then_a_full_idle_period() {
    let start = Instant::now();
    let at = |secs| start + Duration::from_secs(secs);
    let mut tracker = IdleTracker::new(Duration::from_secs(5));
    assert!(!tracker.observe(&[listed(Idle)], at(0)));
    assert!(!tracker.observe(&[listed(Done)], at(60)));
    assert!(!tracker.observe(&[listed(Working), listed(Idle)], at(61)));
    assert!(!tracker.observe(&[listed(Idle), listed(Done)], at(62)));
    assert!(!tracker.observe(&[listed(Idle), listed(Done)], at(66)));
    assert!(tracker.observe(&[listed(Idle), listed(Done)], at(67)));
}

#[test]
fn idle_tracker_restarts_for_busy_unknown_undetected_or_pending_members() {
    let start = Instant::now();
    let at = |secs| start + Duration::from_secs(secs);
    for interruption in [
        listed(Working),
        listed(Blocked),
        listed(Unknown),
        MemberState::Listed {
            detected: false,
            status: Idle,
        },
        MemberState::Pending,
    ] {
        let mut tracker = IdleTracker::new(Duration::from_secs(5));
        let idle = [listed(Idle), listed(Idle)];
        assert!(!tracker.observe(&[listed(Blocked), listed(Idle)], at(0)));
        assert!(!tracker.observe(&idle, at(1)));
        assert!(!tracker.observe(&[listed(Idle), interruption], at(4)));
        assert!(!tracker.observe(&idle, at(5)));
        assert!(!tracker.observe(&idle, at(9)), "{interruption:?}");
        assert!(tracker.observe(&idle, at(10)), "{interruption:?}");
    }
}

#[test]
fn idle_tracker_waits_for_late_members_and_skips_gone_ones() {
    let start = Instant::now();
    let at = |secs| start + Duration::from_secs(secs);
    let mut tracker = IdleTracker::new(Duration::from_secs(5));
    assert!(!tracker.observe(&[listed(Working)], at(0)));
    assert!(!tracker.observe(&[listed(Idle)], at(1)));
    assert!(!tracker.observe(&[listed(Idle), MemberState::Pending], at(6)));
    assert!(!tracker.observe(&[listed(Idle), listed(Idle)], at(7)));
    assert!(tracker.observe(&[listed(Idle), MemberState::Gone], at(12)));
    assert!(!tracker.observe(&[MemberState::Gone], at(30)));
}

/// A fresh ready directory holding `<n>.ready` files for the given panes.
fn ready_dir(name: &str, panes: &[&str]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "herdr-cluster-watch-{name}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    for (index, pane) in panes.iter().enumerate() {
        add_ready_file(&dir, index + 1, pane);
    }
    dir
}

fn add_ready_file(dir: &Path, index: usize, pane: &str) {
    std::fs::write(dir.join(format!("{index}.ready")), format!("{pane}\n")).unwrap();
    // A half-written file from write_ready_file must be ignored.
    std::fs::write(dir.join(format!("{index}.ready.tmp")), "w1:p99\n").unwrap();
}

/// What a fake `pane.get` finds for a member missing from `agent.list`.
#[derive(Clone, Copy)]
enum PaneLookup {
    Live,
    NotFound,
    Fails,
}

/// For runs where every member stays listed, so no poll may call `pane.get`.
fn no_lookup(poll: usize, pane: &str) -> PaneLookup {
    panic!("poll {poll} looked up listed pane {pane}");
}

struct Run {
    outcome: Outcome,
    polls: usize,
    closes: Vec<String>,
    /// `agent.get`, `pane.read` and `pane.close` calls in order.
    calls: Vec<String>,
    lookups: Vec<(usize, String)>,
    elapsed: Duration,
}

fn run_supervise(
    config: &WatchConfig,
    failing_close: Option<&str>,
    list: impl FnMut(usize) -> Result<Vec<AgentInfo>, ()>,
    lookup: impl FnMut(usize, &str) -> PaneLookup,
) -> Run {
    run_interrupted(config, failing_close, list, lookup, || None)
}

/// Runs `supervise` with a fake clock. `list` answers each `agent.list` poll by number,
/// `lookup` answers `pane.get`, and the `failing_close` pane's `pane.close` fails.
fn run_interrupted(
    config: &WatchConfig,
    failing_close: Option<&str>,
    mut list: impl FnMut(usize) -> Result<Vec<AgentInfo>, ()>,
    mut lookup: impl FnMut(usize, &str) -> PaneLookup,
    interrupt: impl FnMut() -> Option<EndReason>,
) -> Run {
    use crate::api::schema::{ErrorBody, ErrorResponse};

    let start = Instant::now();
    let offset = Cell::new(Duration::ZERO);
    let mut polls = 0;
    let mut closes = Vec::new();
    let mut calls = Vec::new();
    let mut lookups = Vec::new();
    let outcome = supervise(
        config,
        |method| match method {
            Method::AgentList(_) => {
                polls += 1;
                list(polls)
                    .map(|agents| ResponseResult::AgentList { agents })
                    .map_err(|()| std::io::Error::other("server down").into())
            }
            Method::PaneGet(target) => {
                lookups.push((polls, target.pane_id.clone()));
                match lookup(polls, &target.pane_id) {
                    PaneLookup::Live => Ok(ResponseResult::PaneInfo {
                        pane: serde_json::from_value(serde_json::json!({
                            "pane_id": target.pane_id, "terminal_id": "term_1",
                            "workspace_id": "w1", "tab_id": "w1:t1", "focused": false,
                            "agent_status": "unknown", "revision": 0,
                        }))
                        .unwrap(),
                    }),
                    PaneLookup::NotFound => Err(ApiClientError::ErrorResponse(ErrorResponse {
                        id: "test".into(),
                        error: ErrorBody {
                            code: "pane_not_found".into(),
                            message: "closed".into(),
                        },
                    })),
                    PaneLookup::Fails => Err(std::io::Error::other("timed out").into()),
                }
            }
            Method::AgentGet(target) => {
                calls.push(format!("agent.get {}", target.target));
                Ok(ResponseResult::AgentInfo {
                    agent: agent(&target.target, Idle),
                })
            }
            Method::PaneRead(params) => {
                calls.push(format!("pane.read {}", params.pane_id));
                assert_eq!(
                    (params.source, params.lines),
                    (ReadSource::Recent, Some(200))
                );
                Ok(ResponseResult::PaneRead {
                    read: serde_json::from_value(serde_json::json!({
                        "pane_id": params.pane_id, "workspace_id": "w1", "tab_id": "w1:t1",
                        "source": "recent", "format": "text", "revision": 0, "truncated": false,
                        "text": format!("screen of {}", params.pane_id),
                    }))
                    .unwrap(),
                })
            }
            Method::PaneClose(target) => {
                calls.push(format!("pane.close {}", target.pane_id));
                closes.push(target.pane_id.clone());
                if failing_close == Some(target.pane_id.as_str()) {
                    Err(std::io::Error::other("close failed").into())
                } else {
                    Ok(ResponseResult::Ok {})
                }
            }
            _ => panic!("unexpected API method"),
        },
        || start + offset.get(),
        |duration| offset.set(offset.get() + duration),
        interrupt,
    );
    std::fs::remove_dir_all(&config.ready_dir).unwrap();
    Run {
        outcome,
        polls,
        closes,
        calls,
        lookups,
        elapsed: offset.get(),
    }
}

fn config(dir: PathBuf, leader: Option<&str>, timeout: Duration) -> WatchConfig {
    WatchConfig {
        ready_dir: dir,
        leader_pane: leader.map(str::to_owned),
        timeout: Some(timeout),
        idle_exit: Duration::from_secs(4),
        capture_on_timeout: false,
    }
}

#[test]
fn supervise_counts_work_that_finished_between_two_polls() {
    // No poll sees `working`, but the agent reports a completed task.
    for (has_completion, reason) in [(true, EndReason::Done), (false, EndReason::Timeout)] {
        let dir = ready_dir("between-polls", &["w1:p2"]);
        let watch = config(dir, None, Duration::from_secs(20));
        let run = run_supervise(
            &watch,
            None,
            |poll| {
                let mut finished = agent("w1:p2", Idle);
                finished.completion_seq = (has_completion && poll > 1).then_some(1);
                Ok(vec![finished])
            },
            no_lookup,
        );
        assert_eq!(run.outcome.reason, reason, "completion {has_completion}");
    }
}

#[test]
fn supervise_ends_with_the_reason_an_interrupt_returns() {
    for (reason, closes) in [
        (EndReason::AgentsExited, &[][..]),
        (EndReason::Stopped, &["pane.close w1:p2"][..]),
    ] {
        let dir = ready_dir("interrupt", &["w1:p2"]);
        let mut watch = config(dir, None, Duration::from_secs(60));
        watch.capture_on_timeout = true;
        let mut checks = 0;
        let run = run_interrupted(
            &watch,
            None,
            |_| Ok(vec![agent("w1:p2", Working)]),
            no_lookup,
            || {
                checks += 1;
                (checks == 3).then_some(reason)
            },
        );
        assert_eq!((run.outcome.reason, run.polls), (reason, 2));
        assert_eq!(run.calls, closes, "{reason:?}");
    }
}

#[test]
fn supervise_captures_members_before_closing_only_on_a_background_timeout() {
    let captured_then_closed = [
        "agent.get w1:p2",
        "pane.read w1:p2",
        "agent.get w1:p3",
        "pane.read w1:p3",
        "pane.close w1:p2",
        "pane.close w1:p3",
    ];
    let closed = ["pane.close w1:p2", "pane.close w1:p3"];
    for (capture, timeout, reason, calls) in [
        (true, 3, EndReason::Timeout, &captured_then_closed[..]),
        (false, 3, EndReason::Timeout, &closed[..]),
        (true, 60, EndReason::Done, &closed[..]),
    ] {
        let dir = ready_dir("capture", &["w1:p2", "w1:p3"]);
        let mut watch = config(dir, None, Duration::from_secs(timeout));
        watch.capture_on_timeout = capture;
        let run = run_supervise(
            &watch,
            None,
            |poll| {
                let status = if poll == 1 { Working } else { Idle };
                Ok(vec![agent("w1:p2", status), agent("w1:p3", status)])
            },
            no_lookup,
        );
        assert_eq!(run.outcome.reason, reason);
        assert_eq!(run.calls, calls, "capture {capture}, {reason:?}");
        // Each snapshot is one agent.get and one pane.read.
        assert_eq!(
            run.outcome.snapshots.len(),
            (calls.len() - closed.len()) / 2
        );
    }
    let dir = ready_dir("snapshot", &["w1:p2"]);
    let mut watch = config(dir, None, Duration::from_secs(1));
    watch.capture_on_timeout = true;
    let run = run_supervise(
        &watch,
        None,
        |_| Ok(vec![agent("w1:p2", Working)]),
        no_lookup,
    );
    assert_eq!(
        run.outcome.snapshots,
        [Snapshot {
            pane_id: "w1:p2".into(),
            name: None,
            kind: Some("claude".into()),
            status: Some(Idle),
            screen: Some("screen of w1:p2".into()),
        }]
    );
}

#[test]
fn supervise_closes_members_but_not_the_leader_once_all_finish() {
    // An overflowing timeout runs without a deadline.
    let dir = ready_dir("done", &["w1:p2", "w1:p3"]);
    let watch = config(dir, Some("w1:p1"), Duration::MAX);
    let run = run_supervise(
        &watch,
        Some("w1:p3"),
        |poll| {
            let leader = if poll == 1 { Working } else { Idle };
            // w1:p7 is not a member, so its work never delays the cluster.
            Ok(vec![
                agent("w1:p1", leader),
                agent("w1:p2", Idle),
                agent("w1:p3", Done),
                agent("w1:p7", Working),
            ])
        },
        no_lookup,
    );
    assert_eq!(run.outcome.reason, EndReason::Done);
    assert_eq!(run.closes, ["w1:p2", "w1:p3"]);
    // w1:p2 and the leader; the failed close of w1:p3 is logged and skipped.
    assert_eq!(run.outcome.closed, 2);
    assert_eq!(run.outcome.members, ["w1:p1", "w1:p2", "w1:p3"]);
    assert_eq!((run.polls, run.elapsed), (4, Duration::from_secs(6)));
}

#[test]
fn supervise_closes_members_when_the_timeout_elapses() {
    let dir = ready_dir("timeout", &["w1:p2"]);
    let watch = config(dir, Some("w1:p1"), Duration::from_secs(5));
    let run = run_supervise(
        &watch,
        None,
        |_| Ok(vec![agent("w1:p1", Working), agent("w1:p2", Working)]),
        no_lookup,
    );
    assert_eq!(run.outcome.reason, EndReason::Timeout);
    assert_eq!(run.closes, ["w1:p2"]);
    assert_eq!(run.outcome.closed, 2);
    assert_eq!(run.elapsed, Duration::from_secs(6));
}

#[test]
fn supervise_without_a_timeout_never_reaches_a_deadline() {
    let dir = ready_dir("no-deadline", &["w1:p2"]);
    let mut watch = config(dir, Some("w1:p1"), Duration::from_secs(5));
    // `--timeout -1`: the working agents must run past any deadline.
    watch.timeout = None;
    let mut checks = 0;
    let run = run_interrupted(
        &watch,
        None,
        |_| Ok(vec![agent("w1:p1", Working), agent("w1:p2", Working)]),
        no_lookup,
        || {
            checks += 1;
            (checks == 5).then_some(EndReason::Stopped)
        },
    );
    // Four 2 s polls; with a 5 s deadline the fourth would have ended in Timeout.
    assert_eq!(run.elapsed, Duration::from_secs(8));
    assert_eq!(run.outcome.reason, EndReason::Stopped);
}

#[test]
fn supervise_ends_without_closing_when_agents_exit_or_the_server_is_gone() {
    // w1:p2 is listed, then confirmed gone; pending w1:p3 is alive, unknown, then gone.
    let dir = ready_dir("exited", &["w1:p2", "w1:p3"]);
    let watch = config(dir, None, Duration::from_secs(60));
    let run = run_supervise(
        &watch,
        None,
        |poll| {
            Ok(if poll < 3 {
                vec![agent("w1:p2", Working)]
            } else {
                vec![]
            })
        },
        |poll, pane| match (pane, poll) {
            ("w1:p3", ..4) => PaneLookup::Live,
            ("w1:p3", 4) => PaneLookup::Fails,
            _ => PaneLookup::NotFound,
        },
    );
    assert_eq!(run.outcome.reason, EndReason::AgentsExited);
    assert_eq!(run.polls, 5);
    assert_eq!(run.lookups[..2], [(1, "w1:p3".into()), (2, "w1:p3".into())]);
    assert!(run.closes.is_empty());
    assert_eq!(run.outcome.closed, 0);

    let dir = ready_dir("server-gone", &["w1:p2"]);
    let watch = config(dir, None, Duration::from_secs(60));
    let run = run_supervise(
        &watch,
        None,
        |poll| {
            if matches!(poll, 1 | 4) {
                Ok(vec![agent("w1:p2", Working)])
            } else {
                Err(())
            }
        },
        no_lookup,
    );
    assert_eq!(run.outcome.reason, EndReason::ServerGone);
    assert_eq!(run.polls, 7);
    assert!(run.closes.is_empty());
}

#[test]
fn supervise_keeps_waiting_for_a_member_that_is_alive_but_unlisted() {
    let dir = ready_dir("undetected", &["w1:p2"]);
    let watch = config(dir, Some("w1:p1"), Duration::from_secs(20));
    let run = run_supervise(
        &watch,
        None,
        |poll| {
            Ok(if poll == 1 {
                vec![agent("w1:p1", Working), agent("w1:p2", Working)]
            } else {
                vec![agent("w1:p1", Idle)]
            })
        },
        |_, _| PaneLookup::Live,
    );
    assert_eq!(run.outcome.reason, EndReason::Timeout);
    assert_eq!(run.closes, ["w1:p2"]);
    assert_eq!(
        run.lookups,
        (2..=11)
            .map(|poll| (poll, "w1:p2".into()))
            .collect::<Vec<_>>()
    );
    let events = run
        .outcome
        .timeline
        .iter()
        .map(|entry| entry.event.as_str());
    assert_eq!(
        events.filter(|event| *event == "w1:p2 - unknown").count(),
        1
    );
}

#[test]
fn supervise_waits_for_pending_or_undetected_members_after_another_is_gone() {
    for is_listed_first in [false, true] {
        let dir = ready_dir("one-gone", &["w1:p2", "w1:p3"]);
        let watch = config(dir, None, Duration::from_secs(10));
        let run = run_supervise(
            &watch,
            None,
            |poll| {
                let mut agents = vec![agent("w1:p2", Working)];
                if is_listed_first {
                    agents.push(agent("w1:p3", Working));
                }
                Ok(if poll == 1 { agents } else { vec![] })
            },
            |_, pane| {
                if pane == "w1:p2" {
                    PaneLookup::NotFound
                } else {
                    PaneLookup::Live
                }
            },
        );
        assert_eq!(run.outcome.reason, EndReason::Timeout, "{is_listed_first}");
        assert_eq!(run.closes, ["w1:p3"], "{is_listed_first}");
    }
}

#[test]
fn supervise_restarts_the_idle_period_when_a_member_joins() {
    let dir = ready_dir("join-idle", &[]);
    let late_dir = dir.clone();
    let watch = config(dir, Some("w1:p1"), Duration::from_secs(60));
    let run = run_supervise(
        &watch,
        None,
        |poll| {
            if poll == 3 {
                add_ready_file(&late_dir, 1, "w1:p2");
            }
            let leader = agent("w1:p1", if poll == 1 { Working } else { Idle });
            Ok(if poll < 4 {
                vec![leader]
            } else {
                vec![leader, agent("w1:p2", Idle)]
            })
        },
        no_lookup,
    );
    assert_eq!(run.outcome.reason, EndReason::Done);
    assert_eq!(run.elapsed, Duration::from_secs(10));
    assert_eq!(run.closes, ["w1:p2"]);
}

#[test]
fn supervise_adds_late_ready_files_and_records_the_timeline() {
    let dir = ready_dir("late", &["w1:p2"]);
    let late_dir = dir.clone();
    let watch = config(dir, None, Duration::from_secs(60));
    let run = run_supervise(
        &watch,
        None,
        |poll| {
            if poll == 2 {
                add_ready_file(&late_dir, 2, "w1:p4");
            }
            let mut second = agent("w1:p4", if poll < 4 { Working } else { Idle });
            second.name = Some("reviewer".into());
            Ok(match poll {
                1 | 2 => vec![agent("w1:p2", Working)],
                _ => vec![agent("w1:p2", Idle), second],
            })
        },
        no_lookup,
    );
    assert_eq!(run.outcome.reason, EndReason::Done);
    assert_eq!(run.outcome.members, ["w1:p2", "w1:p4"]);
    assert_eq!(run.closes, ["w1:p2", "w1:p4"]);
    let timeline = run
        .outcome
        .timeline
        .iter()
        .map(|entry| (entry.elapsed.as_secs(), entry.event.as_str()))
        .collect::<Vec<_>>();
    assert_eq!(
        timeline,
        [
            (0, "w1:p2 joined"),
            (0, "w1:p2 - working"),
            (4, "w1:p4 joined"),
            (4, "w1:p2 - idle"),
            (4, "w1:p4 reviewer working"),
            (6, "w1:p4 reviewer idle"),
        ]
    );
}

#[test]
fn supervise_keeps_only_the_newest_timeline_entries() {
    let dir = ready_dir("cap", &["w1:p2"]);
    let watch = config(dir, None, Duration::from_secs(2400));
    let run = run_supervise(
        &watch,
        None,
        |poll| {
            let status = if poll % 2 == 0 { Working } else { Blocked };
            Ok(vec![agent("w1:p2", status)])
        },
        no_lookup,
    );
    let timeline = &run.outcome.timeline;
    assert_eq!(run.outcome.reason, EndReason::Timeout);
    assert_eq!(timeline.len(), TIMELINE_CAP);
    assert_ne!(timeline[0].event, "w1:p2 joined");
    assert_eq!(
        timeline.back().map(|entry| entry.elapsed),
        Some(Duration::from_secs(2400))
    );
}

#[test]
fn leader_watch_sets_stop_only_after_the_leader_is_gone() {
    let stop = AtomicBool::new(false);
    let mut checks = 0;
    let mut sleeps = 0;
    watch_leader(
        42,
        &stop,
        |pid| {
            assert_eq!(pid, 42);
            assert!(!stop.load(Ordering::SeqCst));
            checks += 1;
            checks < 3
        },
        || sleeps += 1,
    );
    assert!(stop.load(Ordering::SeqCst));
    assert_eq!((checks, sleeps), (3, 2));

    let stop = AtomicBool::new(true);
    watch_leader(42, &stop, |_| panic!("already stopped"), || panic!());
}
