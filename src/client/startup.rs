use super::*;

pub(super) type PassthroughStartedCallback =
    Box<dyn FnOnce(&crate::protocol::passthrough::PassthroughStarted)>;

/// Window title of a passthrough client: `<label> · <pane id>`, tagged with its group.
// The Windows passthrough stub never reads the title.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) struct PassthroughTitle {
    pub(crate) label: String,
    pub(crate) group: Option<PassthroughGroup>,
}

/// The group a passthrough window title shows.
// The Windows passthrough stub never reads the group.
#[cfg_attr(windows, allow(dead_code))]
pub(crate) enum PassthroughGroup {
    /// A cluster's leader pid.
    Cluster(u32),
    /// The session server's pid, for plain switch launches.
    Server,
}

/// Runs the thin client and enters the main event loop.
pub fn run_client() -> io::Result<()> {
    run_client_with_mode(None, None, "connecting to server")
}

#[cfg(unix)]
pub fn run_terminal_attach(terminal_id: String, takeover: bool) -> io::Result<()> {
    run_client_with_mode(
        Some((terminal_id, takeover)),
        Some(AttachEscapeState::default()),
        "attaching to terminal",
    )
}

/// Passthrough needs Unix raw-byte stdin, like direct attach.
#[cfg(windows)]
pub(crate) fn run_passthrough(
    _argv: Vec<String>,
    _cwd: std::path::PathBuf,
    _env: Vec<(String, String)>,
    _agent_name: Option<String>,
    _program_name: Option<String>,
    _title: Option<PassthroughTitle>,
    _on_started: Option<PassthroughStartedCallback>,
    _stop: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
) -> io::Result<i32> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "herdr switch is not supported on Windows yet",
    ))
}

#[cfg(windows)]
pub fn run_terminal_attach(_terminal_id: String, _takeover: bool) -> io::Result<()> {
    debug_assert!(!crate::platform::capabilities().direct_terminal_attach);
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "direct terminal attach is not supported on Windows yet",
    ))
}
