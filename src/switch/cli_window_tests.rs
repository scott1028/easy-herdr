use super::super::{parse_switch_args, SwitchCommand};
use super::*;
use serde_json::json;

fn method(name: &str, params: serde_json::Value) -> Method {
    serde_json::from_value::<Request>(json!({"id":"test", "method":name, "params":params}))
        .unwrap()
        .method
}

#[test]
fn window_arguments_round_trip_through_the_switch_parser() {
    let exe = Path::new("/herdr");
    let ready = Path::new("/ready");
    let cwd = Path::new("/fallback");
    let split = method(
        "pane.split",
        json!({"direction":"right", "env":{"KEY":"a=b"}}),
    );
    let spec = window_spec(&split, exe, "sw-repo", ready, cwd).unwrap();
    assert_eq!(spec.cwd, Path::new("/fallback"));
    assert_eq!(
        parse_switch_args(&spec.argv[2..]),
        Ok(SwitchCommand::WindowShell {
            session: "sw-repo".into(),
            ready_file: ready.to_path_buf(),
            label: None,
            env: vec![("KEY".into(), "a=b".into())],
        })
    );
    let create = method(
        "tab.create",
        json!({"cwd":"/target", "label":"label space"}),
    );
    let spec = window_spec(&create, exe, "sw-repo", ready, cwd).unwrap();
    assert_eq!(spec.cwd, Path::new("/target"));
    assert_eq!(
        parse_switch_args(&spec.argv[2..]),
        Ok(SwitchCommand::WindowShell {
            session: "sw-repo".into(),
            ready_file: ready.to_path_buf(),
            label: Some("label space".into()),
            env: vec![],
        })
    );
}

#[test]
fn window_create_responses_keep_the_normal_success_envelopes() {
    let pane: PaneInfo = serde_json::from_value(json!({
        "pane_id":"w2:p3", "terminal_id":"term", "workspace_id":"w2", "tab_id":"w2:t4",
        "focused":false, "agent_status":"unknown", "revision":0
    }))
    .unwrap();
    let tab: TabInfo = serde_json::from_value(json!({
        "tab_id":"w2:t4", "workspace_id":"w2", "number":1, "label":"shell",
        "focused":false, "pane_count":1, "agent_status":"unknown"
    }))
    .unwrap();
    let workspace: WorkspaceInfo = serde_json::from_value(json!({
        "workspace_id":"w2", "number":1, "label":"existing", "focused":false,
        "pane_count":1, "tab_count":1, "active_tab_id":"w2:t4", "agent_status":"unknown"
    }))
    .unwrap();
    for (name, kind, pane_field) in [
        ("pane.split", "pane_info", "pane"),
        ("tab.create", "tab_created", "root_pane"),
        ("workspace.create", "workspace_created", "root_pane"),
    ] {
        let params = if name == "pane.split" {
            json!({"direction":"right"})
        } else {
            json!({})
        };
        let result = create_response(&method(name, params), pane.clone(), |query| {
            Ok(match query {
                Method::TabGet(target) => {
                    assert_eq!(target.tab_id, pane.tab_id);
                    ResponseResult::TabInfo { tab: tab.clone() }
                }
                Method::WorkspaceGet(target) => {
                    assert_eq!(target.workspace_id, pane.workspace_id);
                    ResponseResult::WorkspaceInfo {
                        workspace: workspace.clone(),
                    }
                }
                _ => panic!("unexpected query"),
            })
        })
        .unwrap();
        let response = serde_json::to_value(SuccessResponse {
            id: "cli:create".into(),
            result,
        })
        .unwrap();
        assert_eq!(response["id"], "cli:create");
        assert_eq!(response["result"]["type"], kind);
        assert_eq!(response["result"][pane_field]["pane_id"], "w2:p3");
        if name != "pane.split" {
            assert_eq!(response["result"]["tab"]["tab_id"], "w2:t4");
        }
        if name == "workspace.create" {
            assert_eq!(response["result"]["workspace"]["workspace_id"], "w2");
        }
    }
}
