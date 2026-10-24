use super::*;
use std::sync::atomic::AtomicUsize;

#[derive(Debug, Clone, PartialEq, Eq)]
enum SinkEvent {
    Output(Vec<u8>),
    Exit(i32),
}

#[derive(Default)]
struct FakeSink {
    events: Mutex<Vec<SinkEvent>>,
    queued: AtomicUsize,
    /// Queue size reported by queued_bytes(), when it differs from push results.
    queued_after_push: Mutex<Option<usize>>,
    gone: AtomicBool,
}

impl PassthroughSink for Arc<FakeSink> {
    fn push_output(&self, bytes: &[u8]) -> Option<usize> {
        if self.gone.load(Ordering::SeqCst) {
            return None;
        }
        self.events
            .lock()
            .unwrap()
            .push(SinkEvent::Output(bytes.to_vec()));
        Some(self.queued.fetch_add(bytes.len(), Ordering::SeqCst) + bytes.len())
    }

    fn queued_bytes(&self) -> Option<usize> {
        if self.gone.load(Ordering::SeqCst) {
            return None;
        }
        Some(
            self.queued_after_push
                .lock()
                .unwrap()
                .unwrap_or_else(|| self.queued.load(Ordering::SeqCst)),
        )
    }

    fn push_exit(&self, code: i32) {
        self.events.lock().unwrap().push(SinkEvent::Exit(code));
    }
}

fn passthrough_with_sink() -> (Arc<PtyPassthrough>, Arc<FakeSink>, Arc<AtomicUsize>) {
    let sink = Arc::new(FakeSink::default());
    let passthrough = PtyPassthrough::new(Box::new(sink.clone()));
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake_count = wakes.clone();
    passthrough.set_waker(Box::new(move || {
        wake_count.fetch_add(1, Ordering::SeqCst);
    }));
    (passthrough, sink, wakes)
}

#[test]
fn reads_pause_above_high_watermark_and_resume_with_wake_after_drain() {
    let (passthrough, _sink, wakes) = passthrough_with_sink();

    passthrough.on_output(&vec![b'x'; PAUSE_QUEUED_BYTES]);
    assert!(
        !passthrough.reads_paused(),
        "at the watermark is still fine"
    );
    passthrough.on_output(b"y");
    assert!(passthrough.reads_paused());

    passthrough.on_drained(RESUME_QUEUED_BYTES + 1);
    assert!(passthrough.reads_paused(), "still above low watermark");
    assert_eq!(wakes.load(Ordering::SeqCst), 0);

    passthrough.on_drained(RESUME_QUEUED_BYTES);
    assert!(!passthrough.reads_paused());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
}

#[test]
fn reads_do_not_stay_paused_when_queue_drained_before_pause_flag() {
    let (passthrough, sink, _) = passthrough_with_sink();
    *sink.queued_after_push.lock().unwrap() = Some(0);

    passthrough.on_output(&vec![b'x'; PAUSE_QUEUED_BYTES + 1]);

    assert!(!passthrough.reads_paused());
}

#[test]
fn reads_never_pause_once_client_is_gone() {
    let (passthrough, sink, _) = passthrough_with_sink();
    passthrough.on_output(&vec![b'x'; PAUSE_QUEUED_BYTES + 1]);
    assert!(passthrough.reads_paused());

    sink.gone.store(true, Ordering::SeqCst);
    passthrough.on_output(b"more");

    assert!(!passthrough.reads_paused());
}

#[test]
fn exit_follows_last_output_when_child_exits_first() {
    let (passthrough, sink, _) = passthrough_with_sink();

    passthrough.on_output(b"bye");
    passthrough.record_exit(3);
    assert_eq!(sink.events.lock().unwrap().len(), 1, "reader still running");
    passthrough.on_output(b"\x1b[?1049l");
    passthrough.finish_reader();
    passthrough.finish_reader();

    assert_eq!(
        *sink.events.lock().unwrap(),
        [
            SinkEvent::Output(b"bye".to_vec()),
            SinkEvent::Output(b"\x1b[?1049l".to_vec()),
            SinkEvent::Exit(3),
        ]
    );
}

#[test]
fn exit_is_sent_once_when_reader_finishes_first() {
    let (passthrough, sink, _) = passthrough_with_sink();

    passthrough.finish_reader();
    assert!(sink.events.lock().unwrap().is_empty());
    passthrough.record_exit(0);
    passthrough.record_exit(0);

    assert_eq!(*sink.events.lock().unwrap(), [SinkEvent::Exit(0)]);
}

#[test]
fn reads_stay_paused_until_started_so_the_client_hears_started_first() {
    let sink = Arc::new(FakeSink::default());
    let passthrough = PtyPassthrough::new_paused(Box::new(sink.clone()));
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake_count = wakes.clone();
    passthrough.set_waker(Box::new(move || {
        wake_count.fetch_add(1, Ordering::SeqCst);
    }));
    assert!(passthrough.reads_paused());

    passthrough.start();

    assert!(!passthrough.reads_paused());
    assert_eq!(wakes.load(Ordering::SeqCst), 1);
}
