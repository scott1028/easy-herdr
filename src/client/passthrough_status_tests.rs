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

fn agent_named(name: Option<&str>) -> AgentInfo {
    serde_json::from_value(json!({
        "terminal_id": "term_1", "name": name, "agent": "claude", "agent_status": "idle",
        "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p2",
        "focused": false, "revision": 1,
    }))
    .unwrap()
}

#[test]
fn role_refresh_reports_only_changed_agent_names() {
    let mut role = None;
    for (response, change, kept) in [
        (Ok(Some(None)), None, None),
        (
            Ok(Some(Some("planner"))),
            Some(Some("planner")),
            Some("planner"),
        ),
        (Ok(Some(Some("planner"))), None, Some("planner")),
        (Err(()), None, Some("planner")),
        (Ok(None), None, Some("planner")),
        (
            Ok(Some(Some("reviewer"))),
            Some(Some("reviewer")),
            Some("reviewer"),
        ),
        (Ok(Some(None)), Some(None), None),
    ] {
        let polled = poll_role_change("w1:p2", &mut role, |method| {
            let Method::AgentGet(target) = method else {
                panic!("unexpected API method");
            };
            assert_eq!(target.target, "w1:p2");
            match response {
                Ok(Some(name)) => Ok(ResponseResult::AgentInfo {
                    agent: agent_named(name),
                }),
                Ok(None) => Ok(ResponseResult::Pong {
                    version: "test".into(),
                    protocol: 1,
                    capabilities: None,
                }),
                Err(()) => Err(io::Error::other("server gone").into()),
            }
        });
        assert_eq!(
            polled,
            change.map(|name: Option<&str>| name.map(str::to_owned)),
            "{response:?}"
        );
        assert_eq!(role.as_deref(), kept, "{response:?}");
    }
}
