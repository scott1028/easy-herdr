//! `herdr switch` client: raw byte passthrough between the host terminal and one agent pane.
//!
//! Herdr draws nothing here. Output bytes go straight to the host terminal, so
//! scrollback, selection, keyboard protocols and queries behave as if the
//! agent ran directly.

use std::io::{self, Read as _, Write as _};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use interprocess::TryClone as _;

use super::terminal_geometry::initial_terminal_geometry;
use super::{do_handshake, init_logging, write_to_server, HandshakePixelCapabilities};
use crate::api::schema::AgentStatus;
use crate::ipc::LocalStream;
use crate::protocol::passthrough::{
    decode_event, spawn_message, PassthroughEvent, PassthroughSpawnRequest, PassthroughStarted,
};
use crate::protocol::{self, ClientMessage, ServerMessage, MAX_GRAPHICS_FRAME_SIZE};
use crate::server::headless::PASSTHROUGH_SPAWN_FAILED;
use crate::server::socket_paths::client_socket_path;

#[path = "passthrough_title.rs"]
mod passthrough_title;
use passthrough_title::TitleRewriter;
#[path = "passthrough_status.rs"]
mod passthrough_status;

const STDIN_CHUNK_BYTES: usize = 64 * 1024;
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(50);

enum RelayInput {
    Output(Vec<u8>),
    Exit(i32),
    Invalid(String),
    Shutdown(Option<String>),
    Lost,
    Status(Option<AgentStatus>),
    Role(Option<String>),
}

/// Notice shown before the agent starts when its requested name was taken.
fn rename_notice(requested: Option<&str>, actual: Option<&str>) -> Option<String> {
    match (requested, actual) {
        (Some(requested), Some(actual)) if requested != actual => {
            Some(format!("agent name {requested} is taken; using {actual}"))
        }
        _ => None,
    }
}

/// True when the server refused before the pane started only because it is shutting down,
/// so a fresh server can take the launch.
fn refusal_is_retryable(reason: &str) -> bool {
    !reason.starts_with(PASSTHROUGH_SPAWN_FAILED)
}

/// Spawns argv as a pane in the active session and relays raw bytes until it exits.
///
/// Returns the program's exit code. `Err` means the server was unreachable before
/// the pane started, so the caller may start a fresh server and retry.
pub(crate) fn run_passthrough(
    argv: Vec<String>,
    cwd: PathBuf,
    env: Vec<(String, String)>,
    agent_name: Option<String>,
    program_name: Option<String>,
    title: Option<super::startup::PassthroughTitle>,
    on_started: Option<super::startup::PassthroughStartedCallback>,
    stop: Option<Arc<AtomicBool>>,
) -> io::Result<i32> {
    init_logging();
    let Some(cwd_text) = cwd.to_str().map(str::to_owned) else {
        eprintln!("herdr-switch: working directory is not valid UTF-8");
        return Ok(1);
    };
    let mut stream = crate::ipc::connect_local_stream(&client_socket_path())?;
    let (cols, rows, cell_width_px, cell_height_px) = host_geometry().unwrap_or((80, 24, 0, 0));
    // A direct terminal handshake: the same-install protocol guards this connection.
    do_handshake(
        &mut stream,
        cols,
        rows,
        cell_width_px,
        cell_height_px,
        HandshakePixelCapabilities {
            exact_geometry: false,
            sgr_mouse: false,
        },
        None,
        false,
        false,
        true,
        true,
        &AtomicBool::new(false),
    )
    .map_err(|err| io::Error::other(err.to_string()))?;
    let request = PassthroughSpawnRequest {
        argv,
        cwd: cwd_text,
        env,
        agent_name: agent_name.clone(),
        program_name,
    };
    write_to_server(
        &mut stream,
        &spawn_message(&request).map_err(io::Error::other)?,
    )?;

    let started = match wait_for_start(&mut stream)? {
        Ok(started) => started,
        Err(reason) => {
            eprintln!("herdr-switch: {reason}");
            return Ok(1);
        }
    };
    if let Some(on_started) = on_started {
        on_started(&started);
    }
    let title = title.map(|title| {
        TitleRewriter::new(
            &format!("{} · {}", title.label, started.pane_id),
            title.group,
            started.agent_name.clone(),
        )
    });
    if let Some(title) = &title {
        if let Err(err) =
            crate::terminal_effects::write_window_title(&mut io::stdout(), Some(&title.label()))
        {
            eprintln!("herdr-switch: {err}");
            return Ok(1);
        }
    }
    if let Some(notice) = rename_notice(agent_name.as_deref(), started.agent_name.as_deref()) {
        eprintln!("herdr-switch: {notice}");
    }

    // The pane is running now, so failures end this client instead of retrying.
    let writer = match stream.try_clone() {
        Ok(writer) => Arc::new(Mutex::new(writer)),
        Err(err) => {
            eprintln!("herdr-switch: {err}");
            return Ok(1);
        }
    };
    if let Err(err) = crossterm::terminal::enable_raw_mode() {
        eprintln!("herdr-switch: needs an interactive terminal: {err}");
        return Ok(1);
    }
    install_termination_handler();
    spawn_stdin_forwarder(writer.clone());
    spawn_resize_forwarder(writer, (cols, rows, cell_width_px, cell_height_px));
    let code = relay_output(stream, title, &started.pane_id, started.agent_name, stop);
    restore_terminal();
    Ok(code)
}

/// Waits for the pane to start. The outer `Err` is retryable; the inner one is a refusal.
fn wait_for_start(stream: &mut LocalStream) -> io::Result<Result<PassthroughStarted, String>> {
    loop {
        match protocol::read_message(stream, MAX_GRAPHICS_FRAME_SIZE) {
            Ok(ServerMessage::EndpointControl { kind, data }) => match decode_event(&kind, &data) {
                Some(Ok(PassthroughEvent::Started(started))) => return Ok(Ok(started)),
                Some(Ok(event)) => return Ok(Err(format!("unexpected server message: {event:?}"))),
                Some(Err(err)) => return Ok(Err(format!("invalid server message: {err}"))),
                None => {}
            },
            Ok(ServerMessage::ServerShutdown { reason }) => {
                let reason = reason.unwrap_or_else(|| "server refused the agent".to_owned());
                if refusal_is_retryable(&reason) {
                    return Err(io::Error::other(reason));
                }
                return Ok(Err(reason));
            }
            // Direct terminal connections can get unrelated server notices.
            Ok(_) => {}
            Err(err) => return Err(io::Error::other(err.to_string())),
        }
    }
}

/// Writes passthrough output to stdout until the program exits, the server goes away
/// or `stop` is set. Stopping returns 0 without waiting; disconnecting closes the pane.
fn relay_output(
    mut stream: LocalStream,
    mut title: Option<TitleRewriter>,
    pane_id: &str,
    role: Option<String>,
    stop: Option<Arc<AtomicBool>>,
) -> i32 {
    let (sender, receiver) = mpsc::sync_channel(64);
    // This token follows the receiver's lifetime, including early returns.
    let alive = Arc::new(());
    if title.is_some() {
        passthrough_status::spawn(pane_id.into(), sender.clone(), Arc::downgrade(&alive));
        passthrough_status::spawn_role_refresh(
            pane_id.into(),
            role,
            sender.clone(),
            Arc::downgrade(&alive),
        );
    }
    std::thread::spawn(move || loop {
        let input = match protocol::read_message(&mut stream, MAX_GRAPHICS_FRAME_SIZE) {
            Ok(ServerMessage::EndpointControl { kind, data }) => match decode_event(&kind, &data) {
                Some(Ok(PassthroughEvent::Output(bytes))) => RelayInput::Output(bytes),
                Some(Ok(PassthroughEvent::Exit(code))) => RelayInput::Exit(code),
                Some(Err(error)) => RelayInput::Invalid(error.to_string()),
                _ => continue,
            },
            Ok(ServerMessage::ServerShutdown { reason }) => RelayInput::Shutdown(reason),
            Err(_) => RelayInput::Lost,
            _ => continue,
        };
        let finished = !matches!(input, RelayInput::Output(_));
        if sender.send(input).is_err() || finished {
            break;
        }
    });
    let mut stdout = io::stdout().lock();
    let mut rewritten = Vec::new();
    let quiet = Duration::from_millis(50);
    let mut last_output = Instant::now();
    let mut dirty = false;
    let mut completion = passthrough_status::CompletionTracker::default();
    let mut alt_screen = crate::terminal_modes::AltScreenTracker::default();
    let mut is_stopped = false;
    let code = loop {
        match receiver.recv_timeout(quiet) {
            Ok(RelayInput::Output(bytes)) => {
                alt_screen.track(&bytes);
                let bytes = if let Some(title) = &mut title {
                    rewritten.clear();
                    title.rewrite(&bytes, &mut rewritten);
                    &rewritten[..]
                } else {
                    &bytes[..]
                };
                if stdout
                    .write_all(bytes)
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    return 1;
                }
                last_output = Instant::now();
            }
            Ok(RelayInput::Exit(code)) => break code,
            Ok(RelayInput::Status(status)) => {
                let icon = completion
                    .display(status)
                    .and_then(passthrough_status::title_icon);
                if let Some(title) = &mut title {
                    dirty |= title.set_status(icon);
                }
            }
            Ok(RelayInput::Role(role)) => {
                if let Some(title) = &mut title {
                    dirty |= title.set_role(role);
                }
            }
            Ok(RelayInput::Invalid(err)) => {
                restore_terminal();
                eprintln!("\r\nherdr-switch: invalid server message: {err}");
                break 1;
            }
            Ok(RelayInput::Shutdown(reason)) => {
                restore_terminal();
                eprintln!(
                    "\r\nherdr-switch: {}",
                    reason.as_deref().unwrap_or("server shut down")
                );
                break 1;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Ok(RelayInput::Lost) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                restore_terminal();
                eprintln!("\r\nherdr-switch: lost connection to the herdr server");
                break 1;
            }
        }
        if let Some(title) = &title {
            if dirty && title.at_ground() && last_output.elapsed() >= quiet {
                if stdout
                    .write_all(&title.status_title())
                    .and_then(|()| stdout.flush())
                    .is_err()
                {
                    return 1;
                }
                dirty = false;
            }
        }
        if stop
            .as_ref()
            .is_some_and(|stop| stop.load(Ordering::SeqCst))
        {
            is_stopped = true;
            break 0;
        }
    };
    if let Some(title) = &mut title {
        rewritten.clear();
        title.finish(&mut rewritten);
        if stdout
            .write_all(&rewritten)
            .and_then(|()| stdout.flush())
            .is_err()
        {
            return 1;
        }
    }
    // A stopped agent never sends its own cleanup.
    if is_stopped
        && crate::terminal_modes::reset_passthrough_modes(&mut stdout, alt_screen.get_is_active())
            .is_err()
    {
        return 1;
    }
    code
}

fn spawn_stdin_forwarder(writer: Arc<Mutex<LocalStream>>) {
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = vec![0u8; STDIN_CHUNK_BYTES];
        loop {
            let read = match stdin.read(&mut buf) {
                Ok(0) => return,
                Ok(read) => read,
                Err(err) if err.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return,
            };
            let message = ClientMessage::Input {
                data: buf[..read].to_vec(),
            };
            if send(&writer, &message).is_err() {
                return;
            }
        }
    });
}

fn spawn_resize_forwarder(writer: Arc<Mutex<LocalStream>>, initial: (u16, u16, u32, u32)) {
    std::thread::spawn(move || {
        crate::platform::watch_terminal_resize_signal();
        let mut last = initial;
        loop {
            std::thread::sleep(RESIZE_POLL_INTERVAL);
            let signalled = crate::platform::take_terminal_resize_signal();
            let Some(current) = host_geometry() else {
                continue;
            };
            let (cols, rows, cell_width_px, cell_height_px) = current;
            if !signalled && current == last {
                continue;
            }
            last = current;
            let message = ClientMessage::Resize {
                cols,
                rows,
                cell_width_px,
                cell_height_px,
                // The host terminal talks to the program directly; Herdr reads no mouse input.
                pixel_mouse: false,
            };
            if send(&writer, &message).is_err() {
                return;
            }
        }
    });
}

fn send(writer: &Mutex<LocalStream>, message: &ClientMessage) -> io::Result<()> {
    let mut stream = writer
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_to_server(&mut *stream, message)
}

/// Host grid and cell size; the cell size is 0 when the terminal does not report pixels.
fn host_geometry() -> Option<(u16, u16, u32, u32)> {
    let (cols, rows, cell_width_px, cell_height_px, _) =
        initial_terminal_geometry(true, false).ok()?;
    Some((cols, rows, cell_width_px, cell_height_px))
}

/// SIGHUP (terminal closed) or SIGTERM: leave raw mode and exit; the socket
/// closing makes the server end the agent.
fn install_termination_handler() {
    let _ = ctrlc::set_handler(|| {
        restore_terminal();
        std::process::exit(1);
    });
}

fn restore_terminal() {
    let _ = crossterm::terminal::disable_raw_mode();
}

#[cfg(test)]
#[path = "passthrough_tests.rs"]
mod tests;
