//! Agent panes spawned for passthrough clients (`herdr switch`).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use tracing::warn;

use super::agents::{
    valid_agent_name, AGENT_START_SETTLE_DELAY, INVALID_AGENT_NAME_MESSAGE, MAX_AGENT_START_TIMEOUT,
};
use super::App;
use crate::pane::PaneLaunchEnv;
use crate::pty::passthrough::PtyPassthrough;
use crate::workspace::Workspace;

const MAX_AGENT_NAME_LEN: usize = 32;
const MAX_AGENT_NAME_SUFFIX: usize = 99;

/// A passthrough agent pane request from one client.
pub(crate) struct PassthroughSpawn {
    pub(crate) argv: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) agent_name: Option<String>,
    pub(crate) program_name: Option<String>,
    pub(crate) rows: u16,
    pub(crate) cols: u16,
    pub(crate) passthrough: Arc<PtyPassthrough>,
}

/// The spawned pane, identified for the owning client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PassthroughSpawned {
    pub(crate) terminal_id: crate::terminal::TerminalId,
    pub(crate) pane_id: String,
    pub(crate) agent_name: Option<String>,
}

/// Returns `base`, or the first free `base-N` (N = 2..=99) trimmed to the name length limit.
pub(crate) fn allocate_agent_name(base: &str, is_taken: impl Fn(&str) -> bool) -> Option<String> {
    if !is_taken(base) {
        return Some(base.to_owned());
    }
    (2..=MAX_AGENT_NAME_SUFFIX)
        .map(|number| {
            let suffix = format!("-{number}");
            // Valid names are ASCII, so byte slicing is safe.
            let keep = base.len().min(MAX_AGENT_NAME_LEN - suffix.len());
            format!("{}{suffix}", &base[..keep])
        })
        .find(|candidate| !is_taken(candidate))
}

fn program_basename(argv: &[String]) -> &str {
    argv.first()
        .and_then(|program| Path::new(program).file_name())
        .and_then(|name| name.to_str())
        .unwrap_or_default()
}

impl App {
    /// Spawns argv as a new tab in this server's workspace, creating the workspace if needed.
    pub(crate) fn spawn_passthrough_agent(
        &mut self,
        spawn: PassthroughSpawn,
    ) -> Result<PassthroughSpawned, String> {
        let agent_name = match spawn.agent_name.as_deref() {
            Some(base) if !valid_agent_name(base) => return Err(INVALID_AGENT_NAME_MESSAGE.into()),
            Some(base) => Some(
                allocate_agent_name(base, |name| !self.agent_name_conflicts(name, "").is_empty())
                    .ok_or_else(|| format!("no free agent name for {base}"))?,
            ),
            None => None,
        };
        let program = spawn
            .program_name
            .clone()
            .unwrap_or_else(|| program_basename(&spawn.argv).to_owned());
        let launch = PaneLaunchEnv::default()
            .with_base_env(spawn.env)
            .with_passthrough(spawn.passthrough);
        let scrollback_limit_bytes = self.state.pane_scrollback_limit_bytes;
        let host_terminal_theme = self.state.host_terminal_theme;
        let host_terminal_appearance = self.pane_host_terminal_appearance();

        let (ws_idx, tab_idx, terminal, runtime, created_workspace) =
            if let Some(ws) = self.state.workspaces.first_mut() {
                let (tab_idx, terminal, runtime) = ws
                    .create_tab_argv_command_with_launch(
                        spawn.rows,
                        spawn.cols,
                        spawn.cwd,
                        &spawn.argv,
                        launch,
                        scrollback_limit_bytes,
                        host_terminal_theme,
                        host_terminal_appearance,
                    )
                    .map_err(|err| err.to_string())?;
                (0, tab_idx, terminal, runtime, false)
            } else {
                let (ws, terminal, runtime) = Workspace::new_argv_command_with_launch(
                    spawn.cwd,
                    spawn.rows,
                    spawn.cols,
                    &spawn.argv,
                    scrollback_limit_bytes,
                    host_terminal_theme,
                    host_terminal_appearance,
                    self.event_tx.clone(),
                    self.render_notify.clone(),
                    self.render_dirty.clone(),
                    launch,
                )
                .map_err(|err| err.to_string())?;
                self.state.workspaces.push(ws);
                (self.state.workspaces.len() - 1, 0, terminal, runtime, true)
            };

        let terminal_id = terminal.id.clone();
        let tab = &mut self.state.workspaces[ws_idx].tabs[tab_idx];
        tab.set_custom_name(agent_name.clone().unwrap_or_else(|| program.clone()));
        let root_pane = tab.root_pane;
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.remove_alias_shadowed_by_new_pane(root_pane);
        self.state.terminals.insert(terminal_id.clone(), terminal);
        if let Some(name) = agent_name.clone() {
            if let Some(terminal) = self.state.terminals.get_mut(&terminal_id) {
                match crate::detect::identify_agent(&program) {
                    Some(kind) => terminal.begin_managed_agent(
                        name,
                        kind,
                        Instant::now(),
                        AGENT_START_SETTLE_DELAY,
                        MAX_AGENT_START_TIMEOUT,
                    ),
                    None => terminal.set_agent_name(name),
                }
            }
        }
        if created_workspace {
            if self.state.active.is_none() {
                self.state.switch_workspace(ws_idx);
            }
            self.emit_workspace_open_events(ws_idx);
        } else {
            self.emit_tab_created_events(ws_idx, tab_idx);
        }
        let pane_id = self
            .public_pane_id(ws_idx, root_pane)
            .ok_or_else(|| "spawned pane has no public id".to_owned())?;
        Ok(PassthroughSpawned {
            terminal_id,
            pane_id,
            agent_name,
        })
    }

    /// Closes the pane showing this terminal; its runtime shutdown ends the program.
    pub(crate) fn close_passthrough_terminal(&mut self, terminal_id: &crate::terminal::TerminalId) {
        let pane = self
            .state
            .workspaces
            .iter()
            .enumerate()
            .find_map(|(ws_idx, ws)| {
                ws.tabs
                    .iter()
                    .flat_map(|tab| tab.panes.iter())
                    .find(|(_, pane)| pane.attached_terminal_id == *terminal_id)
                    .map(|(&pane_id, _)| (ws_idx, pane_id))
            });
        let Some(pane_id) = pane.and_then(|(ws_idx, pane_id)| self.public_pane_id(ws_idx, pane_id))
        else {
            return;
        };
        if let Err(response) = self.close_pane(
            "passthrough:close".into(),
            &crate::api::schema::PaneTarget { pane_id },
        ) {
            warn!(terminal_id = %terminal_id, response, "failed to close passthrough pane");
        }
    }
}

// These tests run shell programs through a Unix PTY.
#[cfg(all(test, unix))]
#[path = "passthrough_tests.rs"]
mod tests;
