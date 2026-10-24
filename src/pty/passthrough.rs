//! Raw PTY output passthrough for a client that owns one pane.

// Only the Unix PTY actor taps output; `herdr switch` is Unix-only for now.
#![cfg_attr(windows, allow(dead_code))]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Pause PTY reads once this many output bytes wait for the client.
pub(crate) const PAUSE_QUEUED_BYTES: usize = 1024 * 1024;
/// Resume PTY reads once the client queue drains to this many bytes.
pub(crate) const RESUME_QUEUED_BYTES: usize = 256 * 1024;

/// Delivery channel to the owning client.
pub(crate) trait PassthroughSink: Send + Sync {
    /// Queues raw PTY output. Returns bytes still queued, or None once the client is gone.
    fn push_output(&self, bytes: &[u8]) -> Option<usize>;
    /// Bytes still queued, or None once the client is gone.
    fn queued_bytes(&self) -> Option<usize>;
    /// Sends the program exit code after the last output byte.
    fn push_exit(&self, code: i32);
}

type Waker = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
struct ExitState {
    reader_finished: bool,
    code: Option<i32>,
    sent: bool,
}

pub(crate) struct PtyPassthrough {
    sink: Box<dyn PassthroughSink>,
    reads_paused: AtomicBool,
    waker: OnceLock<Waker>,
    exit: Mutex<ExitState>,
}

impl std::fmt::Debug for PtyPassthrough {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyPassthrough")
            .field("reads_paused", &self.reads_paused())
            .finish_non_exhaustive()
    }
}

impl PtyPassthrough {
    pub(crate) fn new(sink: Box<dyn PassthroughSink>) -> Arc<Self> {
        Arc::new(Self {
            sink,
            reads_paused: AtomicBool::new(false),
            waker: OnceLock::new(),
            exit: Mutex::new(ExitState::default()),
        })
    }

    /// Starts with reads paused so the server can announce the pane before any output.
    pub(crate) fn new_paused(sink: Box<dyn PassthroughSink>) -> Arc<Self> {
        let passthrough = Self::new(sink);
        passthrough.reads_paused.store(true, Ordering::SeqCst);
        passthrough
    }

    /// Begins reading PTY output.
    pub(crate) fn start(&self) {
        if self.reads_paused.swap(false, Ordering::SeqCst) {
            self.wake();
        }
    }

    fn wake(&self) {
        if let Some(wake) = self.waker.get() {
            wake();
        }
    }

    pub(crate) fn reads_paused(&self) -> bool {
        self.reads_paused.load(Ordering::SeqCst)
    }

    /// Wakes the PTY actor after reads resume.
    pub(crate) fn set_waker(&self, waker: Waker) {
        let _ = self.waker.set(waker);
    }

    /// Forwards one PTY read chunk; called on the PTY actor thread.
    pub(crate) fn on_output(&self, bytes: &[u8]) {
        let Some(queued) = self.sink.push_output(bytes) else {
            self.reads_paused.store(false, Ordering::SeqCst);
            return;
        };
        if queued <= PAUSE_QUEUED_BYTES {
            return;
        }
        self.reads_paused.store(true, Ordering::SeqCst);
        // The writer may have drained everything before the flag was set.
        if self
            .sink
            .queued_bytes()
            .is_none_or(|queued| queued <= RESUME_QUEUED_BYTES)
        {
            self.reads_paused.store(false, Ordering::SeqCst);
        }
    }

    /// Called after the client writer flushed passthrough bytes.
    pub(crate) fn on_drained(&self, queued: usize) {
        if queued <= RESUME_QUEUED_BYTES && self.reads_paused.swap(false, Ordering::SeqCst) {
            self.wake();
        }
    }

    /// The PTY reader saw EOF; no more output follows.
    pub(crate) fn finish_reader(&self) {
        self.update_exit(|state| state.reader_finished = true);
    }

    /// The child process exited with this code.
    pub(crate) fn record_exit(&self, code: i32) {
        self.update_exit(|state| state.code = Some(code));
    }

    /// Sends Exit once both the reader and the child are done, so it follows the last byte.
    fn update_exit(&self, update: impl FnOnce(&mut ExitState)) {
        let code = {
            let mut state = self
                .exit
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            update(&mut state);
            match (state.reader_finished, state.code, state.sent) {
                (true, Some(code), false) => {
                    state.sent = true;
                    code
                }
                _ => return,
            }
        };
        self.sink.push_exit(code);
    }
}

#[cfg(test)]
#[path = "passthrough_tests.rs"]
mod tests;
