//! Named `EndpointControl` messages for `herdr switch` passthrough clients.
//!
//! They ride a same-install direct terminal connection (`TerminalHello`), so the
//! client and server are the same Herdr build. Endpoint generation 1 is unchanged.

// Only the Unix client sends spawns and decodes events; `herdr switch` is Unix-only for now.
#![cfg_attr(windows, allow(dead_code))]

use base64::Engine as _;
use serde::{Deserialize, Serialize};

use super::{ClientMessage, ServerMessage};

pub(crate) const SPAWN_KIND: &str = "terminal.passthrough.spawn.v1";
const STARTED_KIND: &str = "terminal.passthrough.started.v1";
const OUTPUT_KIND: &str = "terminal.passthrough.output.v1";
const EXIT_KIND: &str = "terminal.passthrough.exit.v1";

/// Client request to run argv in a pane owned by this connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PassthroughSpawnRequest {
    /// Program and arguments, run without a shell.
    pub(crate) argv: Vec<String>,
    /// Working directory for the program.
    pub(crate) cwd: String,
    /// Full environment for the program.
    pub(crate) env: Vec<(String, String)>,
    /// Requested managed agent name; the server may add a numeric suffix.
    pub(crate) agent_name: Option<String>,
    /// The requested command when argv starts with a launcher shell; names the tab.
    pub(crate) program_name: Option<String>,
}

/// The pane was spawned; output follows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct PassthroughStarted {
    pub(crate) terminal_id: String,
    pub(crate) pane_id: String,
    /// Actual managed agent name after conflict suffixing.
    pub(crate) agent_name: Option<String>,
    /// The session server's process ID.
    #[serde(default)]
    pub(crate) server_pid: Option<u32>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
struct PassthroughExit {
    code: i32,
}

/// One server message for a passthrough client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum PassthroughEvent {
    Started(PassthroughStarted),
    /// Raw PTY output bytes.
    Output(Vec<u8>),
    /// The program exited; no output follows.
    Exit(i32),
}

pub(crate) fn spawn_message(
    request: &PassthroughSpawnRequest,
) -> serde_json::Result<ClientMessage> {
    Ok(ClientMessage::EndpointControl {
        kind: SPAWN_KIND.into(),
        data: serde_json::to_string(request)?,
    })
}

pub(crate) fn decode_spawn(data: &str) -> serde_json::Result<PassthroughSpawnRequest> {
    serde_json::from_str(data)
}

pub(crate) fn started_message(started: &PassthroughStarted) -> serde_json::Result<ServerMessage> {
    Ok(ServerMessage::EndpointControl {
        kind: STARTED_KIND.into(),
        data: serde_json::to_string(started)?,
    })
}

/// `EndpointControl` carries text, so raw bytes travel as base64.
pub(crate) fn output_message(bytes: &[u8]) -> ServerMessage {
    ServerMessage::EndpointControl {
        kind: OUTPUT_KIND.into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
    }
}

pub(crate) fn exit_message(code: i32) -> serde_json::Result<ServerMessage> {
    Ok(ServerMessage::EndpointControl {
        kind: EXIT_KIND.into(),
        data: serde_json::to_string(&PassthroughExit { code })?,
    })
}

/// Decodes a passthrough control; `None` means `kind` is some other control.
pub(crate) fn decode_event(kind: &str, data: &str) -> Option<Result<PassthroughEvent, String>> {
    let event = match kind {
        STARTED_KIND => serde_json::from_str(data)
            .map(PassthroughEvent::Started)
            .map_err(|err| err.to_string()),
        OUTPUT_KIND => base64::engine::general_purpose::STANDARD
            .decode(data)
            .map(PassthroughEvent::Output)
            .map_err(|err| err.to_string()),
        EXIT_KIND => serde_json::from_str::<PassthroughExit>(data)
            .map(|exit| PassthroughEvent::Exit(exit.code))
            .map_err(|err| err.to_string()),
        _ => return None,
    };
    Some(event)
}

#[cfg(test)]
#[path = "passthrough_tests.rs"]
mod tests;
