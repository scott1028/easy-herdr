use super::*;
use serde_json::json;

#[test]
fn parses_each_status_from_the_subscription_envelope() {
    for status in [
        AgentStatus::Done,
        AgentStatus::Working,
        AgentStatus::Blocked,
        AgentStatus::Idle,
        AgentStatus::Unknown,
    ] {
        let line = json!({"event":"pane.agent_status_changed", "data":{
            "pane_id":"w1:p2", "workspace_id":"w1", "agent_status":status, "agent":"codex"
        }})
        .to_string();
        assert_eq!(parse_line(&line, "w1:p2"), StatusLine::Status(Some(status)));
        assert_eq!(parse_line(&line, "w1:p3"), StatusLine::Ignore);
    }
}

#[test]
fn clears_the_status_when_the_pane_has_no_agent() {
    let line = json!({"event":"pane.agent_status_changed", "data":{
        "pane_id":"w1:p2", "workspace_id":"w1", "agent_status":"done"
    }});
    assert_eq!(
        parse_line(&line.to_string(), "w1:p2"),
        StatusLine::Status(None)
    );
}

#[test]
fn distinguishes_subscription_acknowledgement_ignored_events_and_loss() {
    for (line, expected) in [
        (
            json!({"id":"switch:status", "result":{"type":"subscription_started"}}),
            StatusLine::Started,
        ),
        (
            json!({"event":"pane.created", "data":{}}),
            StatusLine::Ignore,
        ),
        (
            json!({"id":"switch:status", "error":{"code":"events_lost", "message":"lost events"}}),
            StatusLine::Lost,
        ),
        (
            json!({"id":"switch:status", "error":{"code":"pane_not_found", "message":"closed"}}),
            StatusLine::Lost,
        ),
    ] {
        assert_eq!(parse_line(&line.to_string(), "w1:p2"), expected);
    }
    assert_eq!(parse_line("not JSON", "w1:p2"), StatusLine::Lost);
}

#[test]
fn title_icons_show_each_known_status() {
    for (status, icon) in [
        (AgentStatus::Idle, Some("💤")),
        (AgentStatus::Working, Some("⏳")),
        (AgentStatus::Done, Some("✅")),
        (AgentStatus::Blocked, Some("✋")),
        (AgentStatus::Unknown, None),
    ] {
        assert_eq!(title_icon(status), icon, "{status:?}");
    }
}

#[test]
fn idle_after_work_stays_done_until_the_next_task() {
    let mut completion = CompletionTracker::default();
    for (status, shown) in [
        (Some(AgentStatus::Idle), Some(AgentStatus::Idle)),
        (Some(AgentStatus::Working), Some(AgentStatus::Working)),
        (Some(AgentStatus::Idle), Some(AgentStatus::Done)),
        (Some(AgentStatus::Unknown), Some(AgentStatus::Unknown)),
        (Some(AgentStatus::Idle), Some(AgentStatus::Done)),
        (Some(AgentStatus::Blocked), Some(AgentStatus::Blocked)),
        (Some(AgentStatus::Idle), Some(AgentStatus::Done)),
        (None, None),
        (Some(AgentStatus::Idle), Some(AgentStatus::Idle)),
    ] {
        assert_eq!(completion.display(status), shown, "{status:?}");
    }
}
