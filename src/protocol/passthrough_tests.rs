use super::*;

fn decode_server(message: ServerMessage) -> Option<Result<PassthroughEvent, String>> {
    let ServerMessage::EndpointControl { kind, data } = message else {
        panic!("passthrough messages are endpoint controls: {message:?}");
    };
    decode_event(&kind, &data)
}

#[test]
fn spawn_request_round_trips_through_an_endpoint_control() {
    let request = PassthroughSpawnRequest {
        argv: vec!["codex".into(), "--flag".into()],
        cwd: "/work/project-a".into(),
        env: vec![("TERM".into(), "xterm-256color".into())],
        agent_name: Some("rev".into()),
        program_name: Some("codex-personal".into()),
    };

    let ClientMessage::EndpointControl { kind, data } = spawn_message(&request).unwrap() else {
        panic!("spawn must be an endpoint control");
    };

    assert_eq!(kind, SPAWN_KIND);
    assert_eq!(decode_spawn(&data).unwrap(), request);
}

#[test]
fn server_events_round_trip_including_non_utf8_output() {
    let started = PassthroughStarted {
        terminal_id: "term_1".into(),
        pane_id: "w1:p1".into(),
        agent_name: Some("rev-2".into()),
        server_pid: Some(39001),
    };
    let bytes = b"\x1b[?1049h\xff\xfe hi".to_vec();

    assert_eq!(
        decode_server(started_message(&started).unwrap()),
        Some(Ok(PassthroughEvent::Started(started)))
    );
    assert_eq!(
        decode_server(output_message(&bytes)),
        Some(Ok(PassthroughEvent::Output(bytes)))
    );
    assert_eq!(
        decode_server(exit_message(7).unwrap()),
        Some(Ok(PassthroughEvent::Exit(7)))
    );
}

#[test]
fn other_endpoint_controls_are_not_passthrough_events() {
    assert_eq!(decode_event("endpoint.health.pong.v1", ""), None);
    assert!(matches!(
        decode_event(OUTPUT_KIND, "not base64!"),
        Some(Err(_))
    ));
}

#[test]
fn spawn_request_without_a_program_name_still_decodes() {
    let data = r#"{"argv":["codex"],"cwd":"/w","env":[],"agent_name":null}"#;

    assert_eq!(decode_spawn(data).unwrap().program_name, None);
}

#[test]
fn started_without_a_server_pid_still_decodes() {
    let data = r#"{"terminal_id":"term_1","pane_id":"w1:p1","agent_name":null}"#;

    assert_eq!(
        decode_event(STARTED_KIND, data),
        Some(Ok(PassthroughEvent::Started(PassthroughStarted {
            terminal_id: "term_1".into(),
            pane_id: "w1:p1".into(),
            agent_name: None,
            server_pid: None,
        })))
    );
}
