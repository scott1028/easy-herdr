use super::*;
use crate::pty::passthrough::{PassthroughSink, PAUSE_QUEUED_BYTES};
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug, Clone, PartialEq, Eq)]
enum SinkEvent {
    Output(Vec<u8>),
    Exit(i32),
}

#[derive(Default)]
struct RecordingSink {
    events: Mutex<Vec<SinkEvent>>,
    queued: AtomicUsize,
    changed: std::sync::Condvar,
}

impl RecordingSink {
    fn wait_for(&self, timeout: Duration, done: impl Fn(&[SinkEvent]) -> bool) -> Vec<SinkEvent> {
        let deadline = Instant::now() + timeout;
        let mut events = self.events.lock().unwrap();
        while !done(&events) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            events = self.changed.wait_timeout(events, remaining).unwrap().0;
        }
        events.clone()
    }

    fn output(events: &[SinkEvent]) -> Vec<u8> {
        events
            .iter()
            .filter_map(|event| match event {
                SinkEvent::Output(bytes) => Some(bytes.as_slice()),
                SinkEvent::Exit(_) => None,
            })
            .flatten()
            .copied()
            .collect()
    }
}

impl PassthroughSink for Arc<RecordingSink> {
    fn push_output(&self, bytes: &[u8]) -> Option<usize> {
        self.events
            .lock()
            .unwrap()
            .push(SinkEvent::Output(bytes.to_vec()));
        self.changed.notify_all();
        Some(self.queued.load(Ordering::SeqCst))
    }

    fn queued_bytes(&self) -> Option<usize> {
        Some(self.queued.load(Ordering::SeqCst))
    }

    fn push_exit(&self, code: i32) {
        self.events.lock().unwrap().push(SinkEvent::Exit(code));
        self.changed.notify_all();
    }
}

fn wait_until(timeout: Duration, done: impl Fn() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while !done() {
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    true
}

struct PassthroughActor {
    handle: PtyIoActorHandle,
    peer: UnixStream,
    sink: Arc<RecordingSink>,
    passthrough: Arc<PtyPassthrough>,
    reads: std_mpsc::Receiver<Bytes>,
}

fn spawn_passthrough_actor(responses: Vec<Bytes>) -> PassthroughActor {
    spawn_passthrough_actor_with_parse_delay(responses, Duration::ZERO)
}

/// `parse_delay` stands in for terminal-lock waits while herdr parses each read.
fn spawn_passthrough_actor_with_parse_delay(
    responses: Vec<Bytes>,
    parse_delay: Duration,
) -> PassthroughActor {
    let (actor_socket, peer) = UnixStream::pair().expect("socket pair");
    actor_socket.set_nonblocking(true).expect("nonblocking");
    peer.set_read_timeout(Some(Duration::from_millis(200)))
        .expect("peer timeout");
    let owned = unsafe { OwnedFd::from_raw_fd(actor_socket.into_raw_fd()) };
    let sink = Arc::new(RecordingSink::default());
    let passthrough = PtyPassthrough::new(Box::new(sink.clone()));
    let (read_tx, reads) = std_mpsc::channel();
    let handle = PtyIoActor::spawn(PtyIoActorConfig {
        pane_id: 1,
        master_fd: owned,
        initially_quiesced: false,
        on_read: Box::new(move |bytes| {
            std::thread::sleep(parse_delay);
            let _ = read_tx.send(Bytes::copy_from_slice(bytes));
            PtyReadResult {
                terminal_responses: responses.clone(),
            }
        }),
        on_reader_exit: None,
        passthrough: Some(passthrough.clone()),
    })
    .expect("actor spawn");
    PassthroughActor {
        handle,
        peer,
        sink,
        passthrough,
        reads,
    }
}

#[test]
fn passthrough_actor_forwards_raw_output_and_still_parses_it() {
    let mut actor = spawn_passthrough_actor(Vec::new());

    actor.peer.write_all(b"\x1b[?1049hhi").unwrap();

    let events = actor.sink.wait_for(Duration::from_secs(1), |events| {
        RecordingSink::output(events) == b"\x1b[?1049hhi"
    });
    assert_eq!(RecordingSink::output(&events), b"\x1b[?1049hhi");
    let parsed = actor.reads.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(!parsed.is_empty());
    actor.handle.shutdown();
}

#[test]
fn passthrough_actor_suppresses_herdr_terminal_responses() {
    let mut actor = spawn_passthrough_actor(vec![Bytes::from_static(b"\x1b[?62c")]);

    actor.peer.write_all(b"\x1b[c").unwrap();
    actor.reads.recv_timeout(Duration::from_secs(1)).unwrap();
    actor
        .handle
        .write_terminal_response(|| Some(Bytes::from_static(b"\x1b]11;rgb:0/0/0\x07")));

    let mut buf = [0u8; 64];
    let read = actor.peer.read(&mut buf);
    assert!(
        matches!(&read, Err(err) if err.kind() == std::io::ErrorKind::WouldBlock),
        "host terminal answers queries; herdr must not: {read:?}"
    );
    actor.handle.shutdown();
}

#[test]
fn passthrough_actor_stops_reading_while_paused_and_resumes_after_drain() {
    let mut actor = spawn_passthrough_actor(Vec::new());
    actor
        .sink
        .queued
        .store(PAUSE_QUEUED_BYTES + 1, Ordering::SeqCst);

    actor.peer.write_all(b"a").unwrap();
    assert!(wait_until(Duration::from_secs(1), || actor
        .passthrough
        .reads_paused()));
    actor.peer.write_all(b"b").unwrap();
    let events = actor
        .sink
        .wait_for(Duration::from_millis(200), |events| events.len() > 1);
    assert_eq!(
        RecordingSink::output(&events),
        b"a",
        "paused actor must not read"
    );

    actor.sink.queued.store(0, Ordering::SeqCst);
    actor.passthrough.on_drained(0);

    let events = actor.sink.wait_for(Duration::from_millis(500), |events| {
        RecordingSink::output(events) == b"ab"
    });
    assert_eq!(RecordingSink::output(&events), b"ab");
    actor.handle.shutdown();
}

#[test]
fn passthrough_actor_drains_trailing_output_on_shutdown() {
    let mut actor = spawn_passthrough_actor(Vec::new());
    actor
        .sink
        .queued
        .store(PAUSE_QUEUED_BYTES + 1, Ordering::SeqCst);
    actor.peer.write_all(b"a").unwrap();
    assert!(wait_until(Duration::from_secs(1), || actor
        .passthrough
        .reads_paused()));
    actor.peer.write_all(b"\x1b[?1049l").unwrap();
    actor.passthrough.record_exit(7);

    actor.handle.shutdown();

    let events = actor.sink.wait_for(Duration::from_secs(1), |events| {
        events.last() == Some(&SinkEvent::Exit(7))
    });
    assert_eq!(RecordingSink::output(&events), b"a\x1b[?1049l");
    assert_eq!(events.last(), Some(&SinkEvent::Exit(7)));
}

/// Pauses reads with one byte delivered, as when the client falls behind.
fn pause_after_first_byte(actor: &mut PassthroughActor) {
    actor
        .sink
        .queued
        .store(PAUSE_QUEUED_BYTES + 1, Ordering::SeqCst);
    actor.peer.write_all(b"a").unwrap();
    assert!(wait_until(Duration::from_secs(1), || actor
        .passthrough
        .reads_paused()));
}

#[test]
fn passthrough_actor_shutdown_drain_is_not_slowed_by_parsing() {
    let mut actor = spawn_passthrough_actor_with_parse_delay(Vec::new(), Duration::from_millis(40));
    pause_after_first_byte(&mut actor);
    let mut tail = vec![b'x'; 40 * 1024];
    tail.extend_from_slice(b"\x1b[?1049l");
    let mut peer = actor.peer;
    // macOS's 8 KiB socket buffers block writes while actor reads are paused.
    let writer = std::thread::spawn({
        let tail = tail.clone();
        move || peer.write_all(&tail)
    });
    actor.passthrough.record_exit(0);

    actor.handle.shutdown();

    let events = actor.sink.wait_for(Duration::from_secs(2), |events| {
        events.last() == Some(&SinkEvent::Exit(0))
    });
    writer.join().expect("tail writer").expect("tail write");
    let mut expected = b"a".to_vec();
    expected.extend_from_slice(&tail);
    assert_eq!(RecordingSink::output(&events).len(), expected.len());
    assert_eq!(RecordingSink::output(&events), expected);
}

#[test]
fn passthrough_actor_shutdown_drain_waits_for_bytes_still_in_flight() {
    let mut actor = spawn_passthrough_actor(Vec::new());
    pause_after_first_byte(&mut actor);
    actor.passthrough.record_exit(0);

    actor.handle.shutdown();
    // Linux moves PTY writes to the master asynchronously, so the tail can land after Shutdown.
    std::thread::sleep(Duration::from_millis(20));
    // Fails with EPIPE when the actor already gave up and closed the PTY.
    let _ = actor.peer.write_all(b"\x1b[?1049l");
    drop(actor.peer);

    let events = actor.sink.wait_for(Duration::from_secs(2), |events| {
        events.last() == Some(&SinkEvent::Exit(0))
    });
    assert_eq!(RecordingSink::output(&events), b"a\x1b[?1049l");
    assert_eq!(events.last(), Some(&SinkEvent::Exit(0)));
}

#[test]
fn passthrough_actor_reports_reader_end_after_eof() {
    let actor = spawn_passthrough_actor(Vec::new());
    actor.passthrough.record_exit(0);

    drop(actor.peer);

    let events = actor
        .sink
        .wait_for(Duration::from_secs(1), |events| !events.is_empty());
    assert_eq!(events, [SinkEvent::Exit(0)]);
}
