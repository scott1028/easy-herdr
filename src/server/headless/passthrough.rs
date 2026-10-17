//! Passthrough clients: one connection owns one agent pane and receives its raw PTY bytes.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tracing::info;

use super::HeadlessServer;
use crate::app::passthrough::PassthroughSpawn;
use crate::protocol::passthrough::{PassthroughSpawnRequest, PassthroughStarted};
use crate::protocol::ServerMessage;
use crate::pty::passthrough::PtyPassthrough;
use crate::server::clients::ClientConnectionMode;

/// `herdr server` argument that starts an ephemeral group server.
pub(crate) const EPHEMERAL_SERVER_ARG: &str = "--ephemeral";

/// Starts every refusal of a passthrough spawn, so clients can tell it from a shutdown.
pub(crate) const PASSTHROUGH_SPAWN_FAILED: &str = "passthrough spawn failed";

/// Ephemeral (herdr switch) servers exit once nothing has used them for this long.
pub(super) const EPHEMERAL_IDLE_EXIT: Duration = Duration::from_secs(5);

/// Idle tracking for a server that only hosts passthrough panes.
#[derive(Debug, Default)]
pub(super) struct EphemeralLifecycle {
    idle_since: Option<Instant>,
}

impl EphemeralLifecycle {
    /// Returns true once the server has been unused for a full idle window.
    pub(super) fn observe(&mut self, in_use: bool, now: Instant) -> bool {
        if in_use {
            self.idle_since = None;
            return false;
        }
        let idle_since = *self.idle_since.get_or_insert(now);
        now.saturating_duration_since(idle_since) >= EPHEMERAL_IDLE_EXIT
    }
}

impl HeadlessServer {
    /// True when an ephemeral server should shut down now.
    /// Group servers only host passthrough panes, never a default shell workspace.
    pub(super) fn ensure_default_workspace(&mut self) -> bool {
        self.ephemeral.is_none() && self.app.ensure_default_workspace()
    }

    pub(super) fn ephemeral_idle_expired(&mut self, now: Instant) -> bool {
        let in_use = !self.app.state.workspaces.is_empty() || !self.clients.is_empty();
        self.ephemeral
            .as_mut()
            .is_some_and(|lifecycle| lifecycle.observe(in_use, now))
    }

    pub(super) fn spawn_passthrough_client(
        &mut self,
        client_id: u64,
        spawn: PassthroughSpawnRequest,
    ) -> bool {
        let PassthroughSpawnRequest {
            argv,
            cwd,
            env,
            agent_name,
            program_name,
        } = spawn;
        if !self.client_is_pending_terminal_mode(client_id) {
            self.reject_passthrough_client(client_id, "connection is not pending terminal attach");
            return false;
        }
        if !crate::platform::capabilities().direct_terminal_attach {
            self.reject_passthrough_client(client_id, "not supported on this platform");
            return false;
        }
        let has_nul = argv
            .iter()
            .chain(env.iter().flat_map(|(key, value)| [key, value]))
            .chain([&cwd])
            .any(|text| text.contains('\0'));
        if argv.is_empty() || has_nul {
            self.reject_passthrough_client(client_id, "invalid command");
            return false;
        }
        let Some((writer, (cols, rows), cell_size)) =
            self.clients.get(&client_id).and_then(|client| {
                Some((
                    client.writer.clone()?,
                    client.terminal_size,
                    client.cell_size,
                ))
            })
        else {
            return false;
        };

        // Reads stay paused until PassthroughStarted is queued ahead of any output.
        let passthrough = PtyPassthrough::new_paused(Box::new(writer.passthrough_sink()));
        let drained = Arc::downgrade(&passthrough);
        writer.set_passthrough_drained(Box::new(move |queued| {
            if let Some(passthrough) = drained.upgrade() {
                passthrough.on_drained(queued);
            }
        }));
        let spawned = match self.app.spawn_passthrough_agent(PassthroughSpawn {
            argv,
            cwd: PathBuf::from(cwd),
            env,
            agent_name,
            program_name,
            rows,
            cols,
            passthrough: passthrough.clone(),
        }) {
            Ok(spawned) => spawned,
            Err(reason) => {
                self.reject_passthrough_client(client_id, &reason);
                return false;
            }
        };

        let terminal_id = spawned.terminal_id.to_string();
        let stamp = self.allocate_activity_stamp();
        if let Some(client) = self.clients.get_mut(&client_id) {
            client.mode = ClientConnectionMode::TerminalPassthrough {
                terminal_id: terminal_id.clone(),
            };
            client.last_activity = stamp;
        }
        self.terminal_attach_owners
            .insert(terminal_id.clone(), client_id);
        self.app
            .state
            .direct_attach_resize_locks
            .insert(spawned.terminal_id.clone());
        if let Some(runtime) = self.app.terminal_runtimes.get(&spawned.terminal_id) {
            runtime.resize(rows, cols, cell_size.width_px, cell_size.height_px);
        }
        info!(client_id, cols, rows, terminal_id = %terminal_id, "passthrough client spawned pane");
        let started = PassthroughStarted {
            terminal_id,
            pane_id: spawned.pane_id,
            agent_name: spawned.agent_name,
        };
        match crate::protocol::passthrough::started_message(&started) {
            Ok(message) => {
                self.send_to_client(client_id, message);
            }
            Err(err) => {
                self.reject_passthrough_client(client_id, &err.to_string());
                return false;
            }
        }
        passthrough.start();
        true
    }

    pub(super) fn client_owns_passthrough(&self, client_id: u64) -> bool {
        self.clients.get(&client_id).is_some_and(|client| {
            matches!(
                client.mode,
                ClientConnectionMode::TerminalPassthrough { .. }
            )
        })
    }

    /// The owning client is gone, so its program ends with it.
    pub(super) fn release_passthrough_terminal(&mut self, terminal_id: &str) {
        self.terminal_attach_owners.remove(terminal_id);
        let Some(terminal_id) = self.terminal_id_by_string(terminal_id) else {
            return;
        };
        self.app
            .state
            .direct_attach_resize_locks
            .remove(&terminal_id);
        self.app.close_passthrough_terminal(&terminal_id);
    }

    fn reject_passthrough_client(&mut self, client_id: u64, reason: &str) {
        self.send_to_client(
            client_id,
            ServerMessage::ServerShutdown {
                reason: Some(format!("{PASSTHROUGH_SPAWN_FAILED}: {reason}")),
            },
        );
        self.remove_client_and_resize_if_needed(client_id);
    }
}
