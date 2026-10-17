use super::*;

pub(super) type PassthroughStartedCallback =
    Box<dyn FnOnce(&crate::protocol::passthrough::PassthroughStarted)>;

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
    _title_label: Option<String>,
    _on_started: Option<PassthroughStartedCallback>,
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
