use super::*;
use crate::config::Config;
use crate::pty::passthrough::PassthroughSink;

struct DiscardSink;

impl PassthroughSink for DiscardSink {
    fn push_output(&self, _bytes: &[u8]) -> Option<usize> {
        Some(0)
    }

    fn queued_bytes(&self) -> Option<usize> {
        Some(0)
    }

    fn push_exit(&self, _code: i32) {}
}

fn empty_app() -> App {
    let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    App::new(
        &Config::default(),
        crate::app::AppPolicy::TEST,
        None,
        api_rx,
        crate::api::EventHub::default(),
    )
}

fn spawn_request(program: &str, agent_name: Option<&str>) -> PassthroughSpawn {
    PassthroughSpawn {
        argv: vec![program.to_owned()],
        cwd: std::env::temp_dir(),
        env: vec![("PATH".to_owned(), "/usr/bin:/bin".to_owned())],
        agent_name: agent_name.map(str::to_owned),
        program_name: None,
        rows: 30,
        cols: 100,
        passthrough: PtyPassthrough::new(Box::new(DiscardSink)),
    }
}

fn tab_labels(app: &App) -> Vec<String> {
    app.state.workspaces[0]
        .tabs
        .iter()
        .map(|tab| tab.custom_name.clone().unwrap_or_default())
        .collect()
}

#[test]
fn allocate_agent_name_prefers_base_then_numbered_suffixes() {
    assert_eq!(
        allocate_agent_name("reviewer", |_| false).as_deref(),
        Some("reviewer")
    );
    assert_eq!(
        allocate_agent_name("reviewer", |name| name == "reviewer").as_deref(),
        Some("reviewer-2")
    );
    assert_eq!(
        allocate_agent_name("reviewer", |name| matches!(name, "reviewer" | "reviewer-2"))
            .as_deref(),
        Some("reviewer-3")
    );
    assert_eq!(allocate_agent_name("reviewer", |_| true), None);
}

#[test]
fn allocate_agent_name_keeps_suffixed_names_within_limit() {
    let base = "a".repeat(32);

    let name = allocate_agent_name(&base, |name| name == base).unwrap();

    assert_eq!(name, format!("{}-2", "a".repeat(30)));
}

#[tokio::test]
async fn first_spawn_creates_workspace_with_agent_tab() {
    let mut app = empty_app();

    let spawned = app
        .spawn_passthrough_agent(spawn_request("/bin/cat", Some("reviewer")))
        .unwrap();

    assert_eq!(app.state.workspaces.len(), 1);
    assert_eq!(tab_labels(&app), ["reviewer"]);
    assert_eq!(spawned.agent_name.as_deref(), Some("reviewer"));
    assert_eq!(
        app.state.terminals[&spawned.terminal_id]
            .agent_name
            .as_deref(),
        Some("reviewer")
    );
    assert!(app.terminal_runtimes.get(&spawned.terminal_id).is_some());
    assert_eq!(
        app.parse_pane_id(&spawned.pane_id)
            .map(|(ws_idx, _)| ws_idx),
        Some(0)
    );
    app.close_passthrough_terminal(&spawned.terminal_id);
}

#[tokio::test]
async fn later_spawns_add_tabs_and_suffix_taken_names() {
    let mut app = empty_app();
    let first = app
        .spawn_passthrough_agent(spawn_request("/bin/cat", Some("reviewer")))
        .unwrap();

    let second = app
        .spawn_passthrough_agent(spawn_request("/bin/cat", Some("reviewer")))
        .unwrap();
    let unnamed = app
        .spawn_passthrough_agent(spawn_request("/bin/cat", None))
        .unwrap();

    assert_eq!(app.state.workspaces.len(), 1);
    assert_eq!(second.agent_name.as_deref(), Some("reviewer-2"));
    assert_eq!(unnamed.agent_name, None);
    assert_eq!(tab_labels(&app), ["reviewer", "reviewer-2", "cat"]);
    for spawned in [first, second, unnamed] {
        app.close_passthrough_terminal(&spawned.terminal_id);
    }
}

#[tokio::test]
async fn spawn_rejects_invalid_agent_name() {
    let mut app = empty_app();

    let result = app.spawn_passthrough_agent(spawn_request("/bin/cat", Some("Bad Name")));

    assert!(result.is_err());
    assert!(app.state.workspaces.is_empty());
}

#[tokio::test]
async fn closing_passthrough_terminals_removes_tab_then_workspace() {
    let mut app = empty_app();
    let first = app
        .spawn_passthrough_agent(spawn_request("/bin/cat", Some("one")))
        .unwrap();
    let second = app
        .spawn_passthrough_agent(spawn_request("/bin/cat", Some("two")))
        .unwrap();

    app.close_passthrough_terminal(&first.terminal_id);
    assert_eq!(tab_labels(&app), ["two"]);
    assert!(app.terminal_runtimes.get(&first.terminal_id).is_none());

    app.close_passthrough_terminal(&second.terminal_id);
    assert!(app.state.workspaces.is_empty());
}

#[tokio::test]
async fn unnamed_tab_shows_the_requested_program_instead_of_its_launcher() {
    let mut app = empty_app();
    let mut request = spawn_request("/bin/cat", None);
    request.program_name = Some("codex-personal".into());

    let spawned = app.spawn_passthrough_agent(request).unwrap();

    assert_eq!(tab_labels(&app), ["codex-personal"]);
    app.close_passthrough_terminal(&spawned.terminal_id);
}

#[tokio::test]
async fn pane_spawned_during_appearance_hold_gets_consistent_pair() {
    use crate::terminal_theme::{HostAppearance, RgbColor, TerminalTheme};
    use bytes::Bytes;

    let mut app = empty_app();
    let background = |app: &mut App, color: RgbColor| {
        app.set_host_terminal_theme(TerminalTheme {
            background: Some(color),
            ..app.state.host_terminal_theme
        });
    };
    background(
        &mut app,
        RgbColor {
            r: 17,
            g: 17,
            b: 17,
        },
    );
    app.set_host_terminal_appearance_state(Some(HostAppearance::Dark), true);
    app.set_host_terminal_appearance_state(Some(HostAppearance::Light), true);

    let spawned = app
        .spawn_passthrough_agent(spawn_request("/bin/cat", None))
        .unwrap();
    let query = |app: &App, request: &[u8]| {
        app.terminal_runtimes
            .get(&spawned.terminal_id)
            .unwrap()
            .test_process_pty_bytes(request)
    };

    assert_eq!(
        query(&app, b"\x1b[?996n"),
        [Bytes::from_static(b"\x1b[?997;1n")]
    );
    assert_eq!(
        query(&app, b"\x1b]11;?\x07"),
        [Bytes::from_static(b"\x1b]11;rgb:1111/1111/1111\x1b\\")]
    );
    background(
        &mut app,
        RgbColor {
            r: 255,
            g: 255,
            b: 255,
        },
    );
    assert_eq!(
        query(&app, b"\x1b[?996n"),
        [Bytes::from_static(b"\x1b[?997;2n")]
    );
    assert_eq!(
        query(&app, b"\x1b]11;?\x07"),
        [Bytes::from_static(b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\")]
    );
    app.close_passthrough_terminal(&spawned.terminal_id);
}
