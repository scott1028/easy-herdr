//! `herdr switch` / `herdr-switch`: run one agent CLI as an invisible Herdr pane.
//!
//! Each launch directory gets its own ephemeral Herdr session, so agents only
//! discover and message agents started in the same directory.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::client::PassthroughGroup;

#[cfg(unix)]
mod agent_home;
pub(crate) mod cli_window;
#[cfg(unix)]
mod cluster;
#[cfg(unix)]
mod shim;
#[cfg(unix)]
mod shortcut;
#[cfg(unix)]
mod window;

const SWITCH_BINARY_NAME: &str = "herdr-switch";
const SESSION_PREFIX: &str = "sw-";
const MAX_SLUG_LEN: usize = 24;
/// Stay below the Unix socket path limit (104 bytes on macOS).
const MAX_SOCKET_PATH_LEN: usize = 100;
/// A new group server can exit while idle just before this client connects.
const LAUNCH_ATTEMPTS: usize = 3;
const LAUNCH_RETRY_DELAY: std::time::Duration = std::time::Duration::from_millis(100);
/// Hex characters of the directory hash kept in the session name.
const HASH_HEX_LEN: usize = 12;
/// Shells that can run an agent wrapper function passed as `"$@"`.
#[cfg(unix)]
const WRAPPER_SHELLS: [&str; 2] = ["bash", "zsh"];
/// Variables that tie a process to an outer Herdr pane or session.
const INHERITED_HERDR_ENV: [&str; 9] = [
    "HERDR_ENV",
    "HERDR_SESSION",
    "HERDR_SOCKET_PATH",
    "HERDR_CLIENT_SOCKET_PATH",
    "HERDR_PANE_ID",
    "HERDR_TAB_ID",
    "HERDR_WORKSPACE_ID",
    "HERDR_BIN_PATH",
    "HERDR_STARTUP_CWD",
];

#[derive(Debug, Clone, PartialEq, Eq)]
enum SwitchCommand {
    Help,
    SessionName,
    WindowShell {
        session: String,
        ready_file: PathBuf,
        label: Option<String>,
        env: Vec<(String, String)>,
    },
    Launch {
        argv: Vec<String>,
        agent_name: Option<String>,
        member: Option<ClusterMember>,
    },
}

/// A switch launch that a cluster leader opened in a tab.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClusterMember {
    /// The leader's pid, shown in the window title.
    group: u32,
    /// Set with `--exit-when-done`: gets the pane id, and the member follows the leader.
    ready_file: Option<PathBuf>,
}

/// How a launch takes part in a cluster; cloned for each launch attempt.
#[derive(Clone)]
enum ClusterLaunch {
    #[cfg(unix)]
    Leader(cluster::LeaderConfig),
    Member(ClusterMember),
}

impl ClusterLaunch {
    fn group(&self) -> u32 {
        match self {
            #[cfg(unix)]
            Self::Leader(leader) => leader.group,
            Self::Member(member) => member.group,
        }
    }
}

/// Runs `herdr switch` when this process was started for it; returns the exit code.
pub(crate) fn maybe_run(raw_args: &[String]) -> Option<i32> {
    #[cfg(unix)]
    if let Some(program) = raw_args.first().and_then(|argv0| {
        shim::shim_program(argv0, std::env::var_os(shim::SHIM_DIR_ENV).as_deref())
    }) {
        return Some(shim::run_shim(program, &raw_args[1..]));
    }
    let args = switch_args(raw_args)?;
    let code = match parse_switch_args(args) {
        Ok(SwitchCommand::Help) => {
            print_switch_help();
            0
        }
        Ok(SwitchCommand::SessionName) => match resolve_session() {
            Ok((_, session)) => {
                println!("{session}");
                0
            }
            Err(err) => report_error(&err),
        },
        Ok(SwitchCommand::Launch {
            argv,
            agent_name,
            member,
        }) => launch(argv, agent_name, member.map(ClusterLaunch::Member))
            .unwrap_or_else(|err| report_error(&err)),
        Ok(SwitchCommand::WindowShell {
            session,
            ready_file,
            label,
            env,
        }) => {
            #[cfg(unix)]
            {
                window::launch(session, ready_file, label, env)
                    .unwrap_or_else(|err| report_error(&err))
            }
            #[cfg(not(unix))]
            {
                drop((session, ready_file, label, env));
                report_error("window shells are not supported on this platform")
            }
        }
        Err(err) => {
            eprintln!("herdr-switch: {err}");
            print_switch_help();
            2
        }
    };
    Some(code)
}

fn report_error(err: &str) -> i32 {
    eprintln!("herdr-switch: {err}");
    1
}

/// Runs `easy-herdr cluster` before Herdr parses options or agent shortcuts.
#[cfg(unix)]
pub(crate) fn maybe_run_cluster(raw_args: &[String]) -> Option<i32> {
    (raw_args.get(1).is_some_and(|arg| arg == "cluster")).then(|| cluster::run(&raw_args[2..]))
}

/// Runs `easy-herdr <agent-cli> [agent-args...]`; every argument goes to the agent.
#[cfg(unix)]
pub(crate) fn maybe_run_shortcut(raw_args: &[String]) -> Option<i32> {
    let program = raw_args.get(1)?;
    // Check the plain name first: a herdr command lookup builds the CLI spec.
    if !shortcut::get_is_plain_name(program)
        || crate::cli::get_is_command(program)
        || !shortcut::get_is_available(program)
    {
        return None;
    }
    let argv = get_shortcut_argv(&raw_args[1..]);
    Some(launch(argv, None, None).unwrap_or_else(|err| report_error(&err)))
}

/// The agent argv, minus one `--` right after the agent CLI.
#[cfg(unix)]
fn get_shortcut_argv(args: &[String]) -> Vec<String> {
    let mut argv = args.to_vec();
    if argv.get(1).is_some_and(|arg| arg == "--") {
        argv.remove(1);
    }
    argv
}

fn print_switch_help() {
    eprintln!("usage: herdr-switch <AGENT_CLI> [AGENT_NAME] [-- AGENT_ARGS...]");
    eprintln!("       herdr switch <AGENT_CLI> [AGENT_NAME] [-- AGENT_ARGS...]");
    eprintln!("       herdr-switch --session-name");
    eprintln!();
    eprintln!("Runs the agent as an invisible Herdr pane. Agents started in the same");
    eprintln!("directory share one session and can find each other with `herdr agent`.");
    eprintln!("Closing the terminal ends the agent.");
    eprintln!("The agent starts through your shell, so wrapper functions work. Codex runs");
    eprintln!("with --no-daemon and opencode with --standalone, so their tools reach this");
    eprintln!("directory's agents instead of a shared server's.");
    eprintln!("~/.herdr-switch/ links the agent homes these launches use.");
    eprintln!("Panes, tabs and workspaces agents create open a switch shell in a new terminal");
    eprintln!("window. Closing that window ends its pane. HERDR_SWITCH_TERMINAL overrides");
    eprintln!("the terminal command. Shims stay first on PATH only for bash and zsh.");
}

pub(crate) fn active_window_session() -> Option<(String, PathBuf)> {
    #[cfg(unix)]
    {
        get_window_session(&crate::session::active_api_socket_path())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

#[cfg(unix)]
fn get_window_session(socket: &Path) -> Option<(String, PathBuf)> {
    let dir = socket.parent()?;
    let name = dir.file_name()?.to_str()?;
    (name.starts_with(SESSION_PREFIX) && dir.parent()?.file_name()? == "sessions")
        .then(|| (name.to_owned(), dir.to_path_buf()))
}

/// The canonical launch directory and its session name.
fn resolve_session() -> Result<(PathBuf, String), String> {
    let cwd = std::env::current_dir()
        .and_then(|cwd| cwd.canonicalize())
        .map_err(|err| format!("cannot resolve the working directory: {err}"))?;
    let session = [true, false]
        .into_iter()
        .map(|with_slug| switch_session_name(&cwd, with_slug))
        .find(|name| {
            crate::session::client_socket_path_for(Some(name))
                .as_os_str()
                .len()
                <= MAX_SOCKET_PATH_LEN
        })
        .ok_or("the Herdr config directory path is too long for a session socket")?;
    Ok((cwd, session))
}

fn launch(
    mut argv: Vec<String>,
    agent_name: Option<String>,
    cluster: Option<ClusterLaunch>,
) -> Result<i32, String> {
    if !crate::platform::capabilities().direct_terminal_attach {
        return Err("not supported on this platform".into());
    }
    let (cwd, session) = resolve_session()?;
    // Capture the environment before selecting the session changes HERDR_SESSION.
    let mut env = agent_env(std::env::vars_os());
    let program_name = shim_launch(&mut argv, &mut env)?;
    crate::session::select_session_for_process(&session)?;

    // Set by the cluster supervisor or leader watch to end this client.
    let stop = Arc::new(AtomicBool::new(false));
    let code = launch_with_retries(ensure_server, || {
        #[cfg(unix)]
        let on_started = cluster
            .as_ref()
            .and_then(|cluster| cluster::get_started_callback(cluster, &stop));
        #[cfg(not(unix))]
        let on_started = None;
        let group = match &cluster {
            Some(cluster) => PassthroughGroup::Cluster(cluster.group()),
            None => PassthroughGroup::Server,
        };
        crate::client::run_passthrough(
            argv.clone(),
            cwd.clone(),
            env.clone(),
            agent_name.clone(),
            program_name.clone(),
            Some(crate::client::PassthroughTitle {
                label: switch_session_label(&session),
                group: Some(group),
            }),
            on_started,
            Some(stop.clone()),
        )
    })?;
    if let Some(ClusterLaunch::Member(member)) = &cluster {
        if stop.load(Ordering::SeqCst) {
            eprintln!("herdr-switch: cluster {} ended", member.group);
        }
    }
    Ok(code)
}

/// Retries while the group server is unreachable, for example after it exited while idle.
fn launch_with_retries(
    mut ensure_server: impl FnMut() -> Result<(), String>,
    mut run: impl FnMut() -> std::io::Result<i32>,
) -> Result<i32, String> {
    let mut last_error = String::new();
    for attempt in 0..LAUNCH_ATTEMPTS {
        if attempt > 0 {
            // Give a server that is shutting down time to release its sockets.
            std::thread::sleep(LAUNCH_RETRY_DELAY);
        }
        // The server can exit between the listening probe and the status query.
        if let Err(err) = ensure_server() {
            last_error = err;
            continue;
        }
        match run() {
            Ok(code) => return Ok(code),
            Err(err) => last_error = format!("could not reach the herdr server: {err}"),
        }
    }
    Err(last_error)
}

/// Starts the group server if needed; the lock keeps concurrent launches from racing.
fn ensure_server() -> Result<(), String> {
    let data_dir = crate::session::data_dir();
    std::fs::create_dir_all(&data_dir)
        .map_err(|err| format!("cannot create {}: {err}", data_dir.display()))?;
    let lock = std::fs::File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .open(data_dir.join("switch-spawn.lock"))
        .and_then(|file| file.lock().map(|()| file))
        .map_err(|err| format!("cannot lock the session: {err}"))?;
    let result =
        crate::server::autodetect::ensure_ephemeral_server().map_err(|err| err.to_string());
    drop(lock);
    result
}

/// Returns the switch arguments when this process was started as `herdr-switch` or `herdr switch`.
fn switch_args(raw_args: &[String]) -> Option<&[String]> {
    let invoked_as_switch = raw_args
        .first()
        .and_then(|program| Path::new(program).file_name())
        .is_some_and(|name| name == SWITCH_BINARY_NAME);
    if invoked_as_switch {
        return raw_args.get(1..);
    }
    (raw_args.get(1).map(String::as_str) == Some("switch")).then(|| &raw_args[2..])
}

fn parse_switch_args(args: &[String]) -> Result<SwitchCommand, String> {
    if args.first().is_some_and(|arg| arg == "--window-shell") {
        let (mut session, mut ready_file, mut label) = (None, None, None);
        let mut env = Vec::new();
        let mut pairs = args[1..].chunks_exact(2);
        for pair in &mut pairs {
            let value = pair[1].clone();
            match pair[0].as_str() {
                "--session" => session = Some(value),
                "--ready-file" => ready_file = Some(PathBuf::from(value)),
                "--label" => label = Some(value),
                "--env" => {
                    let (key, value) = value
                        .split_once('=')
                        .filter(|(key, _)| !key.is_empty())
                        .ok_or("--env requires KEY=VALUE")?;
                    env.push((key.to_owned(), value.to_owned()));
                }
                flag => return Err(format!("unknown window-shell flag: {flag}")),
            }
        }
        if let Some(flag) = pairs.remainder().first() {
            return Err(format!("missing value for {flag}"));
        }
        let session = session.ok_or("window-shell requires --session")?;
        crate::session::validate_name(&session)?;
        if !session.starts_with(SESSION_PREFIX) {
            return Err(format!(
                "window-shell session must start with {SESSION_PREFIX}"
            ));
        }
        return Ok(SwitchCommand::WindowShell {
            session,
            ready_file: ready_file.ok_or("window-shell requires --ready-file")?,
            label,
            env,
        });
    }
    match args {
        [flag] if matches!(flag.as_str(), "-h" | "--help" | "help") => {
            return Ok(SwitchCommand::Help)
        }
        [flag] if flag == "--session-name" => return Ok(SwitchCommand::SessionName),
        _ => {}
    }
    // Hidden: cluster tabs pass the leader's pid, and maybe a ready file, before the agent CLI.
    let (mut group, mut ready_file, mut args) = (None, None, args);
    while let Some(flag) = args
        .first()
        .filter(|flag| matches!(flag.as_str(), "--cluster-group" | "--cluster-ready"))
    {
        let is_group = flag == "--cluster-group";
        let value = args.get(1).ok_or_else(|| {
            let name = if is_group { "<PID>" } else { "<PATH>" };
            format!("missing {name} after {flag}")
        })?;
        let is_repeated = if is_group {
            let pid = value
                .parse::<u32>()
                .ok()
                .filter(|pid| *pid > 0 && value.bytes().all(|byte| byte.is_ascii_digit()))
                .ok_or_else(|| format!("invalid --cluster-group: {value}"))?;
            group.replace(pid).is_some()
        } else {
            ready_file.replace(PathBuf::from(value)).is_some()
        };
        if is_repeated {
            return Err(format!("{flag} given twice"));
        }
        args = &args[2..];
    }
    if ready_file.is_some() && group.is_none() {
        return Err("--cluster-ready requires --cluster-group".into());
    }
    let separator = args.iter().position(|arg| arg == "--");
    let (positionals, extra) = match separator {
        Some(index) => (&args[..index], &args[index + 1..]),
        None => (args, &[][..]),
    };
    let (program, agent_name) = match positionals {
        [program] => (program, None),
        [program, agent_name] => (program, Some(agent_name.clone())),
        [] => return Err("missing <AGENT_CLI>".into()),
        _ => return Err("pass agent arguments after `--`".into()),
    };
    if program.starts_with('-')
        || agent_name
            .as_deref()
            .is_some_and(|name| name.starts_with('-'))
    {
        return Err("pass agent arguments after `--`".into());
    }
    let mut argv = vec![program.clone()];
    argv.extend_from_slice(extra);
    Ok(SwitchCommand::Launch {
        argv,
        agent_name,
        member: group.map(|group| ClusterMember { group, ready_file }),
    })
}

/// Session name for one launch directory: readable prefix plus a hash of the canonical path.
fn switch_session_name(canonical_cwd: &Path, with_slug: bool) -> String {
    let hash = path_hash(canonical_cwd);
    let slug = path_slug(canonical_cwd);
    if with_slug && !slug.is_empty() {
        format!("{SESSION_PREFIX}{slug}-{hash}")
    } else {
        format!("{SESSION_PREFIX}{hash}")
    }
}

fn switch_session_label(session: &str) -> String {
    let name = session.strip_prefix(SESSION_PREFIX).unwrap_or(session);
    name.rsplit_once('-')
        .filter(|(_, hash)| {
            hash.len() == HASH_HEX_LEN && hash.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .map_or(name, |(slug, _)| slug)
        .to_owned()
}

/// argv run through the user's interactive shell, so wrapper functions such as
/// `codex-personal` work; other shells run argv directly.
#[cfg(unix)]
fn launch_argv(shell: Option<&str>, argv: Vec<String>) -> Vec<String> {
    let Some(shell) = shell.filter(|shell| {
        Path::new(shell)
            .file_name()
            .is_some_and(|name| WRAPPER_SHELLS.iter().any(|wrapper| name == *wrapper))
    }) else {
        return argv;
    };
    // The interactive shell loads wrapper functions and must not save history.
    // Its startup files can prepend PATH entries, so the shims go first again.
    let mut launch = vec![
        shell.to_owned(),
        "-ic".to_owned(),
        format!(
            "unset HISTFILE; PATH=\"${}:$PATH\"; eval \"$(printf '%q ' \"$@\")\"",
            shim::SHIM_DIR_ENV
        ),
        SWITCH_BINARY_NAME.to_owned(),
    ];
    launch.extend(argv);
    launch
}

/// Puts the shim directory first on PATH and marks the launch for the shims.
#[cfg(unix)]
fn with_shims(mut env: Vec<(String, String)>, shim_dir: &str) -> Vec<(String, String)> {
    let path = env
        .iter()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.clone())
        .unwrap_or_default();
    let rest = shim::path_without(OsString::from(path).as_os_str(), Path::new(shim_dir));
    let path = match rest.to_str() {
        Some("") | None => shim_dir.to_owned(),
        Some(rest) => format!("{shim_dir}:{rest}"),
    };
    env.retain(|(key, _)| key != "PATH" && key != shim::SHIM_DIR_ENV);
    env.push(("PATH".to_owned(), path));
    env.push((shim::SHIM_DIR_ENV.to_owned(), shim_dir.to_owned()));
    env
}

/// Runs the agent through the user's shell with the agent home shims on PATH.
/// Returns the requested program name for the tab.
#[cfg(unix)]
fn shim_launch(
    argv: &mut Vec<String>,
    env: &mut Vec<(String, String)>,
) -> Result<Option<String>, String> {
    // macOS can report a `herdr-switch` symlink as current_exe.
    let exe = std::env::current_exe()
        .and_then(|exe| exe.canonicalize())
        .map_err(|err| format!("cannot resolve the herdr executable: {err}"))?;
    let shim_dir = shim::install_shims(&shim::switch_root()?, &exe)?;
    let shim_dir = shim_dir
        .to_str()
        .ok_or("the herdr switch directory is not valid UTF-8")?;
    let program_name = argv
        .first()
        .and_then(|program| Path::new(program).file_name())
        .map(|name| name.to_string_lossy().into_owned());
    let shell = env
        .iter()
        .find(|(key, _)| key == "SHELL")
        .map(|(_, value)| value.clone());
    *argv = launch_argv(shell.as_deref(), std::mem::take(argv));
    *env = with_shims(std::mem::take(env), shim_dir);
    Ok(program_name)
}

/// Switch launches are Unix-only for now.
#[cfg(not(unix))]
fn shim_launch(
    _argv: &mut Vec<String>,
    _env: &mut Vec<(String, String)>,
) -> Result<Option<String>, String> {
    Ok(None)
}

/// First hex characters of the path's SHA-256.
fn path_hash(path: &Path) -> String {
    Sha256::digest(path.as_os_str().as_encoded_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        .chars()
        .take(HASH_HEX_LEN)
        .collect()
}

/// The path's file name, lowercased and limited to file-name-safe characters.
fn path_slug(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_lowercase())
        .unwrap_or_default()
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-') {
                ch
            } else {
                '-'
            }
        })
        .take(MAX_SLUG_LEN)
        .collect()
}

/// The client environment for the agent, minus Herdr variables inherited from an outer pane.
fn agent_env(vars: impl IntoIterator<Item = (OsString, OsString)>) -> Vec<(String, String)> {
    vars.into_iter()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(key, _)| !INHERITED_HERDR_ENV.contains(&key.as_str()))
        .collect()
}

#[cfg(test)]
#[path = "switch_tests.rs"]
mod tests;
