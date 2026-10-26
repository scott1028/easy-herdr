use std::cell::{Cell, RefCell};

use super::*;
use crate::api::schema::AgentStatus::{Blocked, Idle};
use crate::api::schema::{AgentInfo, ErrorBody, ErrorResponse, PaneReadResult};

/// The current codex trust dialog, which shows the `trust_directory` gate text.
const TRUST_SCREEN: &str =
    "Trust this folder?\n› 1. Trust and continue\n  2. Quit\n  enter continue · esc quit\n";

/// What the fake server reports for the first agent's pane.
#[derive(Clone, Copy)]
enum Leader {
    Status(AgentStatus),
    /// The pane lives, but no agent is detected in it yet.
    Undetected,
    Gone,
    Unreachable,
}

/// Which reply of the fake server sets the stop flag.
#[derive(Clone, Copy, PartialEq, Eq)]
enum StopAt {
    Never,
    AgentGet,
    AgentExplain,
    AgentRead,
}

struct Run {
    end: WatchEnd,
    /// Seconds since the start of each `agent.explain`.
    explains: Vec<u64>,
    /// Seconds since the start of each `agent.read`.
    reads: Vec<u64>,
    /// Seconds since the start of each `agent.send_keys`.
    presses: Vec<u64>,
}

fn error(code: &str) -> ApiClientError {
    ApiClientError::ErrorResponse(ErrorResponse {
        id: "test".into(),
        error: ErrorBody {
            code: code.into(),
            message: code.into(),
        },
    })
}

fn run_watch(grace: u64, kind: &str, rule: &str, leader: impl Fn(u64, usize) -> Leader) -> Run {
    run_watch_with(grace, kind, rule, TRUST_SCREEN, StopAt::Never, leader)
}

/// Runs `watch` with a fake clock. `leader` gets the seconds since the start and the
/// key presses so far; the dialog is `kind` and `rule`, and `agent.read` returns
/// `screen`. The stop flag is set by the `stop_at` reply, or after 600 s.
fn run_watch_with(
    grace: u64,
    kind: &str,
    rule: &str,
    screen: &str,
    stop_at: StopAt,
    leader: impl Fn(u64, usize) -> Leader,
) -> Run {
    run_watch_dialog(YoloMode::Agent, grace, stop_at, leader, |_, _| {
        (kind.into(), rule.into(), screen.into())
    })
}

fn run_watch_dialog(
    mode: YoloMode,
    grace: u64,
    stop_at: StopAt,
    leader: impl Fn(u64, usize) -> Leader,
    dialog: impl Fn(u64, usize) -> (String, String, String),
) -> Run {
    let start = Instant::now();
    let clock = Cell::new(start);
    let elapsed = || clock.get().duration_since(start).as_secs();
    let stop = Cell::new(false);
    let explains = RefCell::new(Vec::new());
    let reads = RefCell::new(Vec::new());
    let presses = RefCell::new(Vec::new());
    let end = watch(
        "w1:p1",
        mode,
        Duration::from_secs(grace),
        |method| {
            let count = presses.borrow().len();
            let state = leader(elapsed(), count);
            let (kind, rule, screen) = dialog(elapsed(), count);
            match method {
                Method::AgentGet(target) => {
                    assert_eq!(target.target, "w1:p1");
                    stop.set(stop.get() || stop_at == StopAt::AgentGet);
                    match state {
                        Leader::Status(status) => Ok(ResponseResult::AgentInfo {
                            agent: serde_json::from_value::<AgentInfo>(serde_json::json!({
                                "terminal_id": "term_1", "agent": kind, "agent_status": status,
                                "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p1",
                                "focused": false, "revision": 1,
                            }))
                            .unwrap(),
                        }),
                        Leader::Undetected | Leader::Gone => Err(error("agent_not_found")),
                        Leader::Unreachable => Err(std::io::Error::other("no server").into()),
                    }
                }
                Method::PaneGet(target) => {
                    assert_eq!(target.pane_id, "w1:p1");
                    match state {
                        Leader::Gone => Err(error("pane_not_found")),
                        _ => Ok(ResponseResult::Ok {}),
                    }
                }
                Method::AgentExplain(target) => {
                    assert_eq!(target.target, "w1:p1");
                    explains.borrow_mut().push(elapsed());
                    stop.set(stop.get() || stop_at == StopAt::AgentExplain);
                    Ok(ResponseResult::AgentExplain {
                        explain: serde_json::json!({
                            "agent": kind,
                            "state": "blocked",
                            "matched_rule": { "id": rule, "state": "blocked" },
                        }),
                    })
                }
                Method::AgentRead(params) => {
                    assert_eq!(params.target, "w1:p1");
                    assert_eq!(params.source, ReadSource::Detection);
                    reads.borrow_mut().push(elapsed());
                    stop.set(stop.get() || stop_at == StopAt::AgentRead);
                    Ok(ResponseResult::PaneRead {
                        read: PaneReadResult {
                            pane_id: "w1:p1".into(),
                            workspace_id: "w1".into(),
                            tab_id: "w1:t1".into(),
                            source: ReadSource::Detection,
                            format: ReadFormat::Text,
                            text: screen,
                            revision: 1,
                            truncated: false,
                        },
                    })
                }
                Method::AgentSendKeys(params) => {
                    assert_eq!(params.target, "w1:p1");
                    assert_eq!(params.keys, ["Enter"]);
                    presses.borrow_mut().push(elapsed());
                    Ok(ResponseResult::Ok {})
                }
                _ => panic!("unexpected API method"),
            }
        },
        || clock.get(),
        |delay| clock.set(clock.get() + delay),
        || stop.get() || elapsed() >= 600,
    );
    Run {
        end,
        explains: explains.into_inner(),
        reads: reads.into_inner(),
        presses: presses.into_inner(),
    }
}

/// Blocked until the first key press, idle for a while, then its pane closes.
fn answered_by_one_press(t: u64, presses: usize) -> Leader {
    match (t, presses) {
        (_, 0) => Leader::Status(Blocked),
        (..100, _) => Leader::Status(Idle),
        _ => Leader::Gone,
    }
}

#[test]
fn yolo_waits_out_the_grace_period_then_answers_a_known_dialog() {
    for (kind, rule) in [
        ("claude", "bash_permission_prompt"),
        ("claude", "generic_permission_prompt"),
        ("codex", "trust_directory"),
        ("opencode", "permission_required"),
    ] {
        let run = run_watch(60, kind, rule, answered_by_one_press);
        // Polls run every 2 s, so the poll at 60 s is the first one past the grace period.
        assert_eq!(
            run.explains,
            (0..=60).step_by(2).collect::<Vec<_>>(),
            "{kind} {rule}"
        );
        assert_eq!(run.presses, [60], "{kind} {rule}");
        assert_eq!(run.end, WatchEnd::LeaderGone);
    }
}

#[test]
fn yolo_timeout_zero_answers_on_the_first_blocked_poll() {
    let run = run_watch(0, "codex", "trust_directory", answered_by_one_press);
    assert_eq!(run.explains, [0]);
    assert_eq!(run.presses, [0]);
}

#[test]
fn yolo_leaves_a_dialog_a_human_answers_within_the_grace_period() {
    let run = run_watch(60, "claude", "bash_permission_prompt", |t, _| match t {
        ..30 => Leader::Status(Blocked),
        30..100 => Leader::Status(Idle),
        _ => Leader::Gone,
    });
    assert_eq!(run.explains, (0..30).step_by(2).collect::<Vec<_>>());
    assert!(run.presses.is_empty());
    assert_eq!(run.end, WatchEnd::LeaderGone);
}

#[test]
fn yolo_never_presses_keys_on_unknown_dialogs_or_claude_question_forms() {
    for (kind, rule) in [
        ("claude", "live_blocked_form"),
        ("claude", "live_blocked_form_named_session"),
        ("claude", "legacy_no_prompt_blocker"),
        ("claude", "-"),
        ("codex", "live_strong_blocker"),
        ("kimi", "bash_permission_prompt"),
    ] {
        let run = run_watch(0, kind, rule, |t, _| match t {
            ..20 => Leader::Status(Blocked),
            _ => Leader::Gone,
        });
        // Still inspected each poll so a replacement dialog can be recognized.
        assert_eq!(
            run.explains,
            (0..20).step_by(2).collect::<Vec<_>>(),
            "{kind} {rule}"
        );
        assert!(run.presses.is_empty(), "{kind} {rule}");
    }
}

#[test]
fn yolo_gives_up_after_two_retries() {
    let run = run_watch(0, "claude", "bash_permission_prompt", |t, _| match t {
        ..60 => Leader::Status(Blocked),
        _ => Leader::Gone,
    });
    // Each press is checked again 3 s later; inspection continues after giving up.
    assert_eq!(run.presses, [0, 3, 6]);
    assert!(run.explains.starts_with(&[0, 3, 6, 9]));
    assert_eq!(run.end, WatchEnd::LeaderGone);
}

#[test]
fn yolo_starts_every_blocked_episode_afresh() {
    let run = run_watch(4, "claude", "bash_permission_prompt", |t, presses| {
        match (t, presses) {
            (..20, _) => Leader::Status(Blocked),
            (..30, _) => Leader::Status(Idle),
            (..50, ..4) => Leader::Status(Blocked),
            (..80, _) => Leader::Status(Idle),
            _ => Leader::Gone,
        }
    });
    // Episode one gives up after three presses. Episode two starts at the 31 s poll,
    // waits its own 4 s and gets a fresh first press.
    assert_eq!(run.presses, [4, 7, 10, 35]);
    assert_eq!(run.end, WatchEnd::LeaderGone);
}

#[test]
fn yolo_ends_when_the_pane_or_server_goes_or_it_is_stopped() {
    let run = run_watch(0, "claude", "bash_permission_prompt", |_, _| Leader::Gone);
    assert_eq!(run.end, WatchEnd::LeaderGone);
    let run = run_watch(0, "claude", "bash_permission_prompt", |_, _| {
        Leader::Unreachable
    });
    assert_eq!(run.end, WatchEnd::ServerGone);
    // An undetected agent in a live pane keeps the watch running.
    let run = run_watch(0, "claude", "bash_permission_prompt", |t, _| match t {
        ..10 => Leader::Undetected,
        _ => Leader::Status(Idle),
    });
    assert_eq!(run.end, WatchEnd::Stopped);
    assert!(run.presses.is_empty());
}

#[test]
fn yolo_presses_a_gated_dialog_only_while_its_gate_text_shows() {
    let on_quit =
        "Trust this folder?\n  1. Trust and continue\n› 2. Quit\n  enter continue · esc quit\n";
    let legacy =
        "Do you trust the contents of this directory?\n› 1. Yes, continue\n  2. No, quit\n";
    for (screen, presses) in [(TRUST_SCREEN, vec![0]), (on_quit, vec![]), (legacy, vec![])] {
        let run = run_watch_with(
            0,
            "codex",
            "trust_directory",
            screen,
            StopAt::Never,
            answered_by_one_press,
        );
        // A missing gate prevents all presses, while inspection continues.
        assert_eq!(run.reads[0], 0, "{screen}");
        assert_eq!(run.reads, run.explains, "{screen}");
        assert_eq!(run.presses, presses, "{screen}");
    }
    // Entries without a gate also read the screen to track dialog changes.
    let run = run_watch(0, "claude", "bash_permission_prompt", answered_by_one_press);
    assert_eq!(run.reads, [0]);
    assert_eq!(run.presses, [0]);
}

#[test]
fn yolo_sends_no_keys_once_stopped_during_a_reply() {
    for (stop_at, explains) in [
        (StopAt::AgentGet, vec![]),
        (StopAt::AgentExplain, vec![0]),
        (StopAt::AgentRead, vec![0]),
    ] {
        let run = run_watch_with(
            0,
            "codex",
            "trust_directory",
            TRUST_SCREEN,
            stop_at,
            |_, _| Leader::Status(Blocked),
        );
        assert_eq!(run.end, WatchEnd::Stopped);
        assert_eq!(run.explains, explains);
        assert_eq!(run.reads.len(), usize::from(stop_at == StopAt::AgentRead));
        assert!(run.presses.is_empty());
    }
}

#[test]
fn yolo_starts_a_new_episode_after_the_agent_was_undetected() {
    let run = run_watch(
        10,
        "claude",
        "bash_permission_prompt",
        |t, presses| match (t, presses) {
            (4..8, _) => Leader::Undetected,
            (_, 0) => Leader::Status(Blocked),
            (..60, _) => Leader::Status(Idle),
            _ => Leader::Gone,
        },
    );
    // Blocked at 0 s and 2 s, undetected at 4 s and 6 s, blocked again from 8 s:
    // the new block waits its own 10 s instead of answering at 10 s.
    assert_eq!(run.explains, [0, 2, 8, 10, 12, 14, 16, 18]);
    assert_eq!(run.presses, [18]);
}

#[test]
fn yolo_unknown_then_known_dialog_waits_its_own_grace() {
    for mode in [YoloMode::Agent, YoloMode::Herdr] {
        let run = run_watch_dialog(
            mode,
            4,
            StopAt::Never,
            |t, presses| {
                if presses == 0 && t < 40 {
                    Leader::Status(Blocked)
                } else {
                    Leader::Gone
                }
            },
            |t, _| {
                (
                    "claude".into(),
                    if t < 6 {
                        "unknown"
                    } else {
                        "bash_permission_prompt"
                    }
                    .into(),
                    "dialog".into(),
                )
            },
        );
        assert_eq!(run.presses, [10]);
    }
}

#[test]
fn yolo_changed_dialog_resets_attempts_and_grace() {
    for mode in [YoloMode::Agent, YoloMode::Herdr] {
        let run = run_watch_dialog(
            mode,
            4,
            StopAt::Never,
            |t, _| {
                if t < 32 {
                    Leader::Status(Blocked)
                } else {
                    Leader::Gone
                }
            },
            |_, presses| {
                (
                    "claude".into(),
                    "bash_permission_prompt".into(),
                    if presses == 0 {
                        "first".into()
                    } else {
                        "second".into()
                    },
                )
            },
        );
        assert_eq!(run.presses, [4, 11, 14, 17]);
        // A replacement during the initial wait also starts a fresh wait.
        let run = run_watch_dialog(
            mode,
            6,
            StopAt::Never,
            |t, presses| {
                if t < 30 && presses == 0 {
                    Leader::Status(Blocked)
                } else {
                    Leader::Gone
                }
            },
            |t, _| {
                (
                    "claude".into(),
                    "bash_permission_prompt".into(),
                    if t < 4 {
                        "first".into()
                    } else {
                        "second".into()
                    },
                )
            },
        );
        assert_eq!(run.presses, [10]);
    }
}

#[test]
fn yolo_timers_and_braille_spinners_do_not_reset_the_dialog() {
    for mode in [YoloMode::Agent, YoloMode::Herdr] {
        let run = run_watch_dialog(
            mode,
            4,
            StopAt::Never,
            |t, _| {
                if t < 40 {
                    Leader::Status(Blocked)
                } else {
                    Leader::Gone
                }
            },
            |t, _| {
                (
                    "claude".into(),
                    "bash_permission_prompt".into(),
                    format!(
                        "Waiting {} {}",
                        100 - t,
                        char::from_u32(0x2800 + t as u32).unwrap()
                    ),
                )
            },
        );
        assert_eq!(run.presses, [4, 7, 10]);
    }
}

#[test]
fn yolo_caps_changing_dialogs_and_resets_after_a_nonblocked_poll() {
    for mode in [YoloMode::Agent, YoloMode::Herdr] {
        let output = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = super::super::tests::LogWriter(output.clone());
        let subscriber = tracing_subscriber::fmt()
            .without_time()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        let run = tracing::subscriber::with_default(subscriber, || {
            run_watch_dialog(
                mode,
                0,
                StopAt::Never,
                |t, _| match t {
                    40..44 => Leader::Status(Idle),
                    90.. => Leader::Gone,
                    _ => Leader::Status(Blocked),
                },
                |t, presses| {
                    (
                        "claude".into(),
                        "bash_permission_prompt".into(),
                        // Letters ensure each press changes identity after normalization.
                        format!(
                            "dialog {} {}",
                            (b'a' + presses as u8) as char,
                            if t >= 44 { "new" } else { "old" }
                        ),
                    )
                },
            )
        });
        assert_eq!(run.presses.len(), 2 * MAX_PRESSES);
        assert!(run.presses[..MAX_PRESSES].iter().all(|t| *t < 40));
        assert!(run.presses[MAX_PRESSES..].iter().all(|t| *t >= 44));
        let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
        assert_eq!(logs.matches("result=\"press_limit\"").count(), 2, "{logs}");
    }
}

#[test]
fn herdr_answers_each_question_and_review_after_its_own_grace() {
    let run = run_watch_dialog(
        YoloMode::Herdr,
        4,
        StopAt::Never,
        |_, presses| {
            if presses < 4 {
                Leader::Status(Blocked)
            } else {
                Leader::Gone
            }
        },
        |_, presses| {
            (
                "claude".into(),
                if presses < 3 {
                    "live_blocked_form"
                } else {
                    "legacy_no_prompt_blocker"
                }
                .into(),
                if presses < 3 {
                    format!(
                        "Question {} Enter to select",
                        (b'a' + presses as u8) as char
                    )
                } else {
                    "Review your answers".into()
                },
            )
        },
    );
    assert_eq!(run.presses, [4, 11, 18, 25]);
}

#[test]
fn herdr_question_gates_reject_command_approvals_and_edit_prompts() {
    for (kind, rule, screen) in [
        ("codex", "live_strong_blocker", "Allow command?"),
        (
            "claude",
            "legacy_no_prompt_blocker",
            "Do you want to edit this file?",
        ),
    ] {
        let run = run_watch_dialog(
            YoloMode::Herdr,
            0,
            StopAt::Never,
            |t, _| {
                if t < 20 {
                    Leader::Status(Blocked)
                } else {
                    Leader::Gone
                }
            },
            |_, _| (kind.into(), rule.into(), screen.into()),
        );
        assert!(run.presses.is_empty());
    }
    for (kind, rule, screen) in [
        (
            "claude",
            "live_blocked_form_named_session",
            "ENTER TO SELECT",
        ),
        ("codex", "live_strong_blocker", "Enter to submit answer"),
        ("codex", "live_strong_blocker", "Enter to submit all"),
    ] {
        let run = run_watch_dialog(
            YoloMode::Herdr,
            0,
            StopAt::Never,
            answered_by_one_press,
            |_, _| (kind.into(), rule.into(), screen.into()),
        );
        assert_eq!(run.presses, [0]);
    }
}
