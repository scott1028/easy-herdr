use super::*;
use crate::protocol::passthrough::{decode_event, PassthroughEvent, PassthroughSpawnRequest};

/// A pending direct terminal client, or a client-owned shell when `pending_terminal` is false.
fn connect_client(
    server: &mut HeadlessServer,
    client_id: u64,
    pending_terminal: bool,
) -> std::sync::mpsc::Receiver<Vec<u8>> {
    if pending_terminal {
        return connect_pending_terminal_client_with_control_rx(server, client_id);
    }
    let (writer, control_rx, _render_rx) = test_client_writer();
    server.handle_server_event(ServerEvent::ClientShellConnected {
        surface_reuse: false,
        surface_delta: false,
        surface_scroll: false,
        client_id,
        surface_cols: 100,
        surface_rows: 30,
        cell_width_px: 0,
        cell_height_px: 0,
        pixel_mouse: false,
        direct_graphics: false,
        endpoint_keybindings: false,
        mouse_capture: false,
        surface_active: true,
        writer,
    });
    control_rx
}

fn spawn(server: &mut HeadlessServer, client_id: u64, script: &str, name: Option<&str>) -> bool {
    server.handle_server_event(ServerEvent::ClientSpawnPassthrough {
        client_id,
        spawn: PassthroughSpawnRequest {
            argv: vec!["/bin/sh".into(), "-c".into(), script.into()],
            cwd: std::env::temp_dir().display().to_string(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
            agent_name: name.map(str::to_owned),
            program_name: None,
        },
    })
}

fn passthrough_event(message: &ServerMessage) -> Option<PassthroughEvent> {
    let ServerMessage::EndpointControl { kind, data } = message else {
        return None;
    };
    decode_event(kind, data)?.ok()
}

/// Collects messages until `done` holds for the collected list or the timeout passes.
fn collect_messages(
    control_rx: &std::sync::mpsc::Receiver<Vec<u8>>,
    done: impl Fn(&[ServerMessage]) -> bool,
) -> Vec<ServerMessage> {
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut messages = Vec::new();
    while !done(&messages) {
        let remaining = deadline.saturating_duration_since(Instant::now());
        match control_rx.recv_timeout(remaining) {
            Ok(bytes) => messages.push(read_server_message(bytes)),
            Err(_) => break,
        }
    }
    messages
}

fn output_text(messages: &[ServerMessage]) -> String {
    messages
        .iter()
        .filter_map(|message| match passthrough_event(message) {
            Some(PassthroughEvent::Output(bytes)) => {
                Some(String::from_utf8_lossy(&bytes).into_owned())
            }
            _ => None,
        })
        .collect()
}

fn passthrough_terminal_id(server: &HeadlessServer, client_id: u64) -> Option<String> {
    match &server.clients.get(&client_id)?.mode {
        ClientConnectionMode::TerminalPassthrough { terminal_id } => Some(terminal_id.clone()),
        _ => None,
    }
}

#[tokio::test]
async fn spawn_is_rejected_for_full_app_clients() {
    let mut server = test_headless_server();
    let control_rx = connect_client(&mut server, 7, false);
    let workspaces = server.app.state.workspaces.len();

    assert!(!spawn(&mut server, 7, "sleep 5", None));

    let messages = collect_messages(&control_rx, |messages| {
        messages
            .iter()
            .any(|message| matches!(message, ServerMessage::ServerShutdown { .. }))
    });
    assert!(messages
        .iter()
        .any(|message| matches!(message, ServerMessage::ServerShutdown { .. })));
    assert_eq!(server.app.state.workspaces.len(), workspaces);
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn spawn_announces_pane_before_streaming_its_output() {
    let mut server = test_headless_server();
    let control_rx = connect_client(&mut server, 7, true);

    assert!(spawn(&mut server, 7, "printf hello; sleep 5", Some("rev")));

    let messages = collect_messages(&control_rx, |messages| {
        output_text(messages).contains("hello")
    });
    match messages.first().and_then(passthrough_event) {
        Some(PassthroughEvent::Started(started)) => {
            assert_eq!(started.agent_name.as_deref(), Some("rev"));
            assert_eq!(started.server_pid, Some(std::process::id()));
        }
        other => panic!("expected PassthroughStarted first, got {other:?}"),
    }
    assert!(output_text(&messages).contains("hello"));
    let terminal_id = passthrough_terminal_id(&server, 7).expect("client bound to pane");
    assert_eq!(server.terminal_attach_owners.get(&terminal_id), Some(&7));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn passthrough_input_reaches_the_program() {
    let mut server = test_headless_server();
    let control_rx = connect_client(&mut server, 7, true);
    assert!(spawn(
        &mut server,
        7,
        "read line; printf 'got:%s' \"$line\"; sleep 5",
        None
    ));

    server.handle_server_event(ServerEvent::ClientInput {
        client_id: 7,
        data: b"ping\r".to_vec(),
    });

    let messages = collect_messages(&control_rx, |messages| {
        output_text(messages).contains("got:ping")
    });
    assert!(output_text(&messages).contains("got:ping"));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn client_disconnect_closes_its_pane() {
    let mut server = test_headless_server();
    let _control_rx = connect_client(&mut server, 7, true);
    assert!(spawn(&mut server, 7, "sleep 30", Some("rev")));
    let terminal_id = passthrough_terminal_id(&server, 7).unwrap();

    server.handle_server_event(ServerEvent::ClientDisconnected { client_id: 7 });

    assert!(server.app.state.workspaces.is_empty());
    assert!(!server.terminal_attach_owners.contains_key(&terminal_id));
    assert!(server
        .app
        .state
        .terminals
        .keys()
        .all(|id| id.to_string() != terminal_id));
}

#[tokio::test]
async fn attach_takeover_cannot_steal_a_passthrough_pane() {
    let mut server = test_headless_server();
    let _owner_rx = connect_client(&mut server, 7, true);
    assert!(spawn(&mut server, 7, "sleep 30", None));
    let terminal_id = passthrough_terminal_id(&server, 7).unwrap();
    let thief_rx = connect_client(&mut server, 8, true);

    server.handle_server_event(ServerEvent::ClientAttachTerminal {
        client_id: 8,
        terminal_id: terminal_id.clone(),
        takeover: true,
    });

    assert!(matches!(
        read_server_message(thief_rx.recv().unwrap()),
        ServerMessage::ServerShutdown { .. }
    ));
    assert_eq!(server.terminal_attach_owners.get(&terminal_id), Some(&7));
    assert_eq!(passthrough_terminal_id(&server, 7), Some(terminal_id));
    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn program_exit_sends_exit_code_without_server_shutdown() {
    let mut server = test_headless_server();
    let control_rx = connect_client(&mut server, 7, true);
    assert!(spawn(&mut server, 7, "printf bye; exit 4", None));
    let pane_id = server.app.state.workspaces[0].tabs[0].root_pane;

    let messages = collect_messages(&control_rx, |messages| {
        matches!(
            messages.last().and_then(passthrough_event),
            Some(PassthroughEvent::Exit(_))
        )
    });
    assert_eq!(
        messages.last().and_then(passthrough_event),
        Some(PassthroughEvent::Exit(4))
    );
    assert!(output_text(&messages).contains("bye"));

    server.handle_internal_event_with_forwarding(AppEvent::PaneDied {
        pane_id,
        exit_reason: crate::platform::ChildExitReason::Exited,
    });

    assert!(
        control_rx.try_recv().is_err(),
        "no ServerShutdown after exit"
    );
}

#[test]
fn ephemeral_lifecycle_expires_only_after_a_full_idle_window() {
    let start = Instant::now();
    let mut lifecycle = EphemeralLifecycle::default();

    assert!(!lifecycle.observe(false, start));
    assert!(!lifecycle.observe(
        false,
        start + EPHEMERAL_IDLE_EXIT - Duration::from_millis(1)
    ));
    assert!(!lifecycle.observe(true, start + EPHEMERAL_IDLE_EXIT));
    assert!(!lifecycle.observe(
        false,
        start + EPHEMERAL_IDLE_EXIT * 2 - Duration::from_millis(1)
    ));
    assert!(lifecycle.observe(false, start + EPHEMERAL_IDLE_EXIT * 3));
}

#[tokio::test]
async fn ephemeral_server_counts_clients_and_panes_as_use() {
    let mut server = test_headless_server();
    server.ephemeral = Some(EphemeralLifecycle::default());
    let start = Instant::now();
    let _control_rx = connect_client(&mut server, 7, true);
    assert!(spawn(&mut server, 7, "sleep 30", None));

    assert!(!server.ephemeral_idle_expired(start));
    assert!(!server.ephemeral_idle_expired(start + EPHEMERAL_IDLE_EXIT * 2));

    server.handle_server_event(ServerEvent::ClientDisconnected { client_id: 7 });
    assert!(!server.ephemeral_idle_expired(start + EPHEMERAL_IDLE_EXIT * 2));
    assert!(server.ephemeral_idle_expired(start + EPHEMERAL_IDLE_EXIT * 3));
}

#[test]
fn regular_server_never_expires_when_idle() {
    let mut server = test_headless_server();

    assert!(!server.ephemeral_idle_expired(Instant::now() + EPHEMERAL_IDLE_EXIT * 10));
}
