use super::*;
use crate::protocol::passthrough::{
    exit_message, output_message, spawn_message, PassthroughSpawnRequest,
};
use crate::pty::passthrough::PassthroughSink;

#[test]
fn client_read_loop_forwards_spawn_passthrough() {
    let (mut client_stream, server_stream, _path) = local_stream_pair("client-read-spawn");
    let (server_event_tx, mut server_event_rx) = mpsc::channel(4);
    let should_quit = Arc::new(AtomicBool::new(false));
    let read_quit = should_quit.clone();
    let handle = std::thread::spawn(move || {
        client_read_loop(server_stream, 7, &server_event_tx, &read_quit)
    });

    let request = PassthroughSpawnRequest {
        argv: vec!["codex".to_owned(), "--model".to_owned()],
        cwd: "/work/project-a".to_owned(),
        env: vec![("TERM".to_owned(), "xterm-kitty".to_owned())],
        agent_name: Some("reviewer".to_owned()),
        program_name: None,
    };
    protocol::write_message(&mut client_stream, &spawn_message(&request).unwrap())
        .expect("write spawn");

    match server_event_rx.blocking_recv().expect("spawn event") {
        ServerEvent::ClientSpawnPassthrough { client_id, spawn } => {
            assert_eq!(client_id, 7);
            assert_eq!(spawn, request);
        }
        other => panic!("expected ClientSpawnPassthrough, got {other:?}"),
    }

    drop(client_stream);
    should_quit.store(true, Ordering::Release);
    handle
        .join()
        .expect("read thread join")
        .expect("read thread result");
}

#[test]
fn passthrough_output_follows_control_and_precedes_render() {
    let (writer, queue) = test_queue_writer();
    let sink = writer.passthrough_sink();
    writer.render.try_send(b"render".to_vec()).unwrap();

    assert_eq!(sink.push_output(b"hi"), Some(2));
    writer.control.send(b"control".to_vec()).unwrap();

    assert_eq!(
        queue.recv(),
        Some(ClientWriteItem::Control(b"control".to_vec()))
    );
    assert_eq!(
        queue.recv(),
        Some(ClientWriteItem::Passthrough {
            data: frame_server_message(&output_message(b"hi")),
            payload_len: 2,
        })
    );
    assert_eq!(
        queue.recv(),
        Some(ClientWriteItem::Render(b"render".to_vec()))
    );
}

#[test]
fn passthrough_sink_reports_gone_after_writer_closes() {
    let (writer, queue) = test_queue_writer();
    let sink = writer.passthrough_sink();

    queue.close_writer();

    assert_eq!(sink.push_output(b"late"), None);
    assert_eq!(sink.queued_bytes(), None);
}

#[test]
fn client_writer_streams_passthrough_then_exit_and_reports_drain() {
    let (mut client_stream, server_stream, _path) = local_stream_pair("client-writer-pass");
    let (writer, queue) = test_queue_writer();
    let sink = writer.passthrough_sink();
    let drained = Arc::new(Mutex::new(Vec::new()));
    let drained_record = drained.clone();
    writer.set_passthrough_drained(Box::new(move |queued| {
        drained_record.lock().unwrap().push(queued);
    }));
    assert_eq!(sink.push_output(b"\x1b[?1049h"), Some(8));
    assert_eq!(sink.push_output(b"ok"), Some(10));
    assert_eq!(sink.queued_bytes(), Some(10));
    sink.push_exit(3);

    let (server_event_tx, _server_event_rx) = mpsc::channel(4);
    let handle = std::thread::spawn(move || {
        client_writer_loop(server_stream, 9, queue, server_event_tx);
    });

    for expected in [
        output_message(b"\x1b[?1049h"),
        output_message(b"ok"),
        exit_message(3).unwrap(),
    ] {
        let message: ServerMessage =
            protocol::read_message(&mut client_stream, MAX_FRAME_SIZE).expect("read passthrough");
        assert_eq!(message, expected);
    }
    drop(writer);
    drop(sink);
    handle.join().expect("writer exits after senders drop");
    assert_eq!(*drained.lock().unwrap(), [2, 0, 0]);
}

#[test]
fn client_read_loop_disconnects_on_an_invalid_spawn_request() {
    let (mut client_stream, server_stream, _path) = local_stream_pair("client-read-bad-spawn");
    let (server_event_tx, mut server_event_rx) = mpsc::channel(4);
    let should_quit = Arc::new(AtomicBool::new(false));
    let read_quit = should_quit.clone();
    let handle = std::thread::spawn(move || {
        client_read_loop(server_stream, 7, &server_event_tx, &read_quit)
    });

    protocol::write_message(
        &mut client_stream,
        &ClientMessage::EndpointControl {
            kind: crate::protocol::passthrough::SPAWN_KIND.to_owned(),
            data: "{".to_owned(),
        },
    )
    .expect("write invalid spawn");

    assert!(matches!(
        server_event_rx.blocking_recv(),
        Some(ServerEvent::ClientDisconnected { client_id: 7 })
    ));
    should_quit.store(true, Ordering::Release);
    handle
        .join()
        .expect("read thread join")
        .expect("read thread result");
}
