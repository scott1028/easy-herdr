//! PATH shims for `herdr switch`.
//!
//! Agent wrappers such as shell functions end with `command codex`. The shim
//! directory comes first on PATH, so that lookup starts Herdr, which links the
//! program's home under `~/.herdr-switch/` and then runs the real program.

use std::ffi::{OsStr, OsString};
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::agent_home::{create_private_dir, ensure_link, link_agent_home};

/// Set for processes started through a shim; holds the shim directory.
pub(super) const SHIM_DIR_ENV: &str = "HERDR_SWITCH_SHIM_DIR";
const ROOT_DIR: &str = ".herdr-switch";
const SHIMS_DIR: &str = "shims";
const CODEX: &str = "codex";
const CLAUDE: &str = "claude";
const OPENCODE: &str = "opencode";
const SHIMMED_PROGRAMS: [&str; 3] = [CODEX, CLAUDE, OPENCODE];
const CODEX_NO_DAEMON: &str = "--no-daemon";
/// Codex subcommands that need the shared daemon.
const CODEX_DAEMON_SUBCOMMANDS: [&str; 2] = ["agents", "remote-control"];
const OPENCODE_STANDALONE: &str = "--standalone";
const OPENCODE_NO_STANDALONE: &str = "--no-standalone";
const OPENCODE_SERVER: &str = "--server";
/// opencode interface options whose value is the next argument.
/// opencode v2.0.30 subcommands; they win over a directory of the same name.
const OPENCODE_SUBCOMMANDS: [&str; 18] = [
    "upgrade",
    "update",
    "uninstall",
    "acp",
    "api",
    "debug",
    "auth",
    "mcp",
    "plugin",
    "models",
    "stats",
    "mini",
    "run",
    "session",
    "service",
    "reload",
    "pair",
    "serve",
];
const OPENCODE_VALUE_OPTIONS: [&str; 5] = [
    "--session",
    "-s",
    "--prompt",
    "--log-level",
    "--completions",
];

/// `~/.herdr-switch`.
pub(super) fn switch_root() -> Result<PathBuf, String> {
    crate::integration::home_dir()
        .map(|home| home.join(ROOT_DIR))
        .map_err(|err| format!("cannot find the home directory: {err}"))
}

/// Links each shimmed program to `herdr_exe` and returns the shim directory.
/// Each executable gets its own directory, so stable and dev builds never swap shims.
pub(super) fn install_shims(root: &Path, herdr_exe: &Path) -> Result<PathBuf, String> {
    let dir = root.join(SHIMS_DIR).join(super::path_hash(herdr_exe));
    create_private_dir(&dir)?;
    for program in SHIMMED_PROGRAMS {
        ensure_link(herdr_exe, &dir.join(program))?;
    }
    Ok(dir)
}

/// The shimmed program this process runs as, when started inside a switch launch.
pub(super) fn shim_program(argv0: &str, shim_dir: Option<&OsStr>) -> Option<&'static str> {
    shim_dir.filter(|dir| !dir.is_empty())?;
    let name = Path::new(argv0).file_name()?;
    SHIMMED_PROGRAMS
        .into_iter()
        .find(|program| name == *program)
}

/// `path` without any entry equal to `dir`.
pub(super) fn path_without(path: &OsStr, dir: &Path) -> OsString {
    let kept: Vec<PathBuf> = std::env::split_paths(path)
        .filter(|entry| entry != dir)
        .collect();
    // Entries came from a valid PATH, so they contain no separator.
    std::env::join_paths(kept).unwrap_or_else(|_| path.to_owned())
}

/// Replaces this shim process with the real program; returns only on failure.
pub(super) fn run_shim(program: &str, args: &[String]) -> i32 {
    let real_home = match program {
        CODEX => crate::integration::codex_dir().map(Some),
        CLAUDE => crate::integration::claude_dir().map(Some),
        // opencode keeps its state in several XDG directories; there is no one home to link.
        _ => Ok(None),
    };
    let shim_dir = PathBuf::from(std::env::var_os(SHIM_DIR_ENV).unwrap_or_default());
    let path = std::env::var_os("PATH").unwrap_or_default();
    let command = real_home
        .map_err(|err| format!("cannot find the {program} home: {err}"))
        .and_then(|real_home| {
            shim_command(
                program,
                args,
                real_home.as_deref(),
                &switch_root()?,
                &path,
                &shim_dir,
            )
        });
    match command {
        Ok(mut command) => {
            eprintln!("herdr-switch: cannot run {program}: {}", command.exec());
            127
        }
        Err(err) => {
            eprintln!("herdr-switch: {err}");
            1
        }
    }
}

/// The real program with its home, if any, linked under `root` and the shim off PATH.
fn shim_command(
    program: &str,
    args: &[String],
    real_home: Option<&Path>,
    root: &Path,
    path: &OsStr,
    shim_dir: &Path,
) -> Result<Command, String> {
    if let Some(real_home) = real_home {
        link_agent_home(program, real_home, root)?;
    }
    let mut command = Command::new(program);
    // Codex's shared app-server daemon runs tool calls with the environment of
    // whichever pane started it, so they would reach another pane's Herdr server.
    // Codex rejects `--no-daemon` with `--remote` and daemon subcommands; text after
    // `--` is a prompt. An option value equal to a subcommand name is taken as one,
    // which only keeps Codex's default.
    if program == CODEX
        && !args.iter().take_while(|arg| *arg != "--").any(|arg| {
            arg == CODEX_NO_DAEMON
                || arg.starts_with("--remote")
                || CODEX_DAEMON_SUBCOMMANDS.contains(&arg.as_str())
        })
    {
        command.arg(CODEX_NO_DAEMON);
    }
    // opencode runs tools in a background service that keeps the environment of the
    // pane that started it. Its interface gets a private server instead; subcommands
    // reject `--standalone` and keep opencode's default.
    if program == OPENCODE && opencode_opens_interface(args, |arg| Path::new(arg).is_dir()) {
        command.arg(OPENCODE_STANDALONE);
    }
    command
        .args(args)
        .env("PATH", path_without(path, shim_dir))
        .env_remove(SHIM_DIR_ENV);
    Ok(command)
}

/// True when `opencode <args>` opens the interface without a server choice:
/// no subcommand, at most a directory to open.
fn opencode_opens_interface(args: &[String], is_dir: impl Fn(&str) -> bool) -> bool {
    let is_option = |arg: &str, name: &str| {
        arg.strip_prefix(name)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('='))
    };
    if args.iter().any(|arg| {
        is_option(arg, OPENCODE_STANDALONE)
            || is_option(arg, OPENCODE_NO_STANDALONE)
            || is_option(arg, OPENCODE_SERVER)
    }) {
        return false;
    }
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if arg == "--" {
            // `--` ends subcommand parsing; what follows is the directory.
            return args.next().is_none_or(|dir| is_dir(dir));
        }
        if OPENCODE_VALUE_OPTIONS.contains(&arg.as_str()) {
            args.next();
        } else if !arg.starts_with('-') && !matches!(arg.as_str(), "true" | "false") {
            // A bare `true` or `false` is a boolean option's value, as in `--auto true`.
            return !OPENCODE_SUBCOMMANDS.contains(&arg.as_str()) && is_dir(arg);
        }
    }
    true
}

#[cfg(test)]
#[path = "shim_tests.rs"]
mod tests;
