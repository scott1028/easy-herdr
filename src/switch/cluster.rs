use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::api::client::{parse_response_value, ApiClient, ApiClientError};
use crate::api::schema::{
    AgentInfo, AgentPromptParams, AgentStatus, AgentTarget, Method, Request, ResponseResult,
};
use crate::protocol::passthrough::PassthroughStarted;

#[path = "cluster_watch.rs"]
mod cluster_watch;
use cluster_watch::{EndReason, Outcome, WatchConfig};
#[path = "cluster_background.rs"]
mod cluster_background;
#[path = "cluster_help.rs"]
mod cluster_help;
#[path = "cluster_log.rs"]
mod cluster_log;
#[path = "cluster_yolo.rs"]
mod cluster_yolo;

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(2 * 60 * 60);
const DEFAULT_IDLE_EXIT: Duration = Duration::from_secs(5 * 60);
const DEFAULT_YOLO_TIMEOUT: Duration = Duration::from_secs(60);
/// `--yolo` flags for the claude family, before its `--settings` JSON.
const CLAUDE_YOLO_ARGS: [&str; 3] = ["--permission-mode", "bypassPermissions", "--settings"];
const CODEX_YOLO_ARGS: [&str; 2] = ["--ask-for-approval", "never"];
#[derive(Debug, PartialEq, Eq)]
enum ClusterCommand {
    Help,
    Launch {
        agents: Vec<String>,
        prompt: Option<String>,
        options: ClusterOptions,
    },
    /// Hidden `--supervise`: the detached process a background launch starts.
    /// Its prompt arrives on stdin, so it never shows in argv.
    Supervise {
        agents: Vec<String>,
        options: ClusterOptions,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum YoloMode {
    /// `--yolo`: agent flags plus the watcher.
    Agent,
    /// `--yolo-herdr`: the watcher only.
    Herdr,
}

#[derive(Debug, PartialEq, Eq)]
struct ClusterOptions {
    /// Set by `--exit-when-done` and implied by `--background`.
    exit_when_done: bool,
    background: bool,
    /// `None` when `--timeout -1` removed the deadline.
    timeout: Option<Duration>,
    idle_exit: Duration,
    /// How the first agent's dialogs are answered.
    yolo: Option<YoloMode>,
    /// How long a blocked first agent waits before it is answered; zero answers at once.
    yolo_timeout: Duration,
}

impl Default for ClusterOptions {
    fn default() -> Self {
        Self {
            exit_when_done: false,
            background: false,
            timeout: Some(DEFAULT_TIMEOUT),
            idle_exit: DEFAULT_IDLE_EXIT,
            yolo: None,
            yolo_timeout: DEFAULT_YOLO_TIMEOUT,
        }
    }
}

fn parse_cluster_args(
    args: &[String],
    is_available: impl Fn(&str) -> bool,
) -> Result<ClusterCommand, String> {
    if args.len() == 1 && matches!(args[0].as_str(), "-h" | "--help" | "help") {
        return Ok(ClusterCommand::Help);
    }
    let (is_supervisor, args) = match args {
        [flag, rest @ ..] if flag == "--supervise" => (true, rest),
        _ => (false, args),
    };
    let (options, args) = parse_cluster_options(args)?;
    if is_supervisor && !options.background {
        return Err("--supervise requires --background".into());
    }
    let separator = args.iter().position(|arg| arg == "--");
    let (agents, prompt) = if let Some(index) = separator {
        if index + 1 == args.len() {
            return Err("missing <PROMPT> after --".into());
        }
        (&args[..index], Some(args[index + 1..].join(" ")))
    } else if args
        .last()
        .is_some_and(|arg| !super::shortcut::get_is_plain_name(arg) && !arg.starts_with('-'))
    {
        (&args[..args.len() - 1], args.last().cloned())
    } else {
        (args, None)
    };
    if agents.is_empty() {
        return Err("missing <AGENT_CLI>".into());
    }
    if prompt
        .as_ref()
        .is_some_and(|prompt| prompt.trim().is_empty())
    {
        return Err("missing <PROMPT>".into());
    }
    for (index, agent) in agents.iter().enumerate() {
        if agent.starts_with('-') {
            return Err(format!("unknown option: {agent}"));
        }
        if !super::shortcut::get_is_plain_name(agent) {
            if separator.is_none() {
                return Err("the prompt must be the last argument".into());
            }
            return Err(format!("unknown agent: {agent}"));
        }
        if !is_available(agent) {
            let hint = if index + 1 == args.len() {
                "; put a one-word prompt after --"
            } else {
                ""
            };
            return Err(format!("unknown agent: {agent}{hint}"));
        }
    }
    if is_supervisor && (prompt.is_some() || separator.is_some()) {
        return Err("--supervise reads the prompt from stdin".into());
    }
    if options.background && prompt.is_none() && !is_supervisor {
        return Err("--background requires a prompt".into());
    }
    let agents = agents.to_vec();
    Ok(if is_supervisor {
        ClusterCommand::Supervise { agents, options }
    } else {
        ClusterCommand::Launch {
            agents,
            prompt,
            options,
        }
    })
}

/// Parses the options before the first agent and returns the remaining args.
fn parse_cluster_options(args: &[String]) -> Result<(ClusterOptions, &[String]), String> {
    let mut exit_when_done = false;
    let mut background = false;
    let mut timeout: Option<Option<Duration>> = None;
    let mut idle_exit: Option<Duration> = None;
    let mut yolo = None;
    let mut yolo_timeout: Option<Duration> = None;
    let mut index = 0;
    while let Some(flag) = args
        .get(index)
        .filter(|arg| arg.starts_with('-') && *arg != "--")
    {
        let is_repeated = match flag.as_str() {
            "--exit-when-done" => std::mem::replace(&mut exit_when_done, true),
            "--background" => std::mem::replace(&mut background, true),
            "--yolo" | "--yolo-herdr" => {
                let mode = if flag == "--yolo" {
                    YoloMode::Agent
                } else {
                    YoloMode::Herdr
                };
                if yolo.is_some_and(|previous| previous != mode) {
                    return Err("--yolo and --yolo-herdr cannot be used together".into());
                }
                yolo.replace(mode).is_some()
            }
            "--timeout" | "--idle-exit" | "--yolo-timeout" => {
                index += 1;
                let text = args
                    .get(index)
                    .ok_or_else(|| format!("missing <DURATION> after {flag}"))?;
                match flag.as_str() {
                    "--timeout" => timeout.replace(parse_timeout(text)?).is_some(),
                    "--idle-exit" => idle_exit.replace(parse_duration(text)?).is_some(),
                    _ => yolo_timeout.replace(parse_yolo_timeout(text)?).is_some(),
                }
            }
            _ => return Err(format!("unknown option: {flag}")),
        };
        if is_repeated {
            return Err(format!("{flag} given twice"));
        }
        index += 1;
    }
    for (flag, is_given) in [
        ("--timeout", timeout.is_some()),
        ("--idle-exit", idle_exit.is_some()),
    ] {
        if is_given && !exit_when_done && !background {
            return Err(format!("{flag} requires --exit-when-done or --background"));
        }
    }
    if yolo_timeout.is_some() && yolo.is_none() {
        return Err("--yolo-timeout requires --yolo or --yolo-herdr".into());
    }
    let options = ClusterOptions {
        exit_when_done: exit_when_done || background,
        background,
        timeout: timeout.unwrap_or(Some(DEFAULT_TIMEOUT)),
        idle_exit: idle_exit.unwrap_or(DEFAULT_IDLE_EXIT),
        yolo,
        yolo_timeout: yolo_timeout.unwrap_or(DEFAULT_YOLO_TIMEOUT),
    };
    Ok((options, &args[index..]))
}

/// Parses `--timeout`: a duration, or `-1` for a cluster that never times out.
fn parse_timeout(text: &str) -> Result<Option<Duration>, String> {
    if text == "-1" {
        return Ok(None);
    }
    match parse_duration(text) {
        Ok(duration) => Ok(Some(duration)),
        Err(_) => Err(format!("invalid duration: {text} (use 30s, 90m, 2h or -1)")),
    }
}

/// Parses `--yolo-timeout`: a duration, or `0` to answer at once.
pub(super) fn parse_yolo_timeout(text: &str) -> Result<Duration, String> {
    if text == "0" {
        return Ok(Duration::ZERO);
    }
    parse_duration(text).map_err(|_| format!("invalid duration: {text} (use 0, 30s, 90m or 2h)"))
}

/// Formats a `--yolo-timeout` that `parse_yolo_timeout` reads back: zero is `0`, not `0s`.
fn format_yolo_timeout(yolo_timeout: Duration) -> String {
    if yolo_timeout.is_zero() {
        return "0".into();
    }
    format_duration(yolo_timeout)
}

/// Parses `<positive integer><s|m|h>`, for example `30s`, `90m` or `2h`.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let invalid = || format!("invalid duration: {text} (use 30s, 90m or 2h)");
    let unit_seconds = match text.chars().last() {
        Some('s') => 1,
        Some('m') => 60,
        Some('h') => 60 * 60,
        _ => return Err(invalid()),
    };
    // The unit is one ASCII byte, so this slice stays on a char boundary.
    let count = &text[..text.len() - 1];
    if count.is_empty() || !count.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(invalid());
    }
    count
        .parse::<u64>()
        .ok()
        .filter(|count| *count > 0)
        .and_then(|count| count.checked_mul(unit_seconds))
        .map(Duration::from_secs)
        .ok_or_else(invalid)
}

/// Formats whole seconds with the largest exact unit, for example `90m`.
fn format_duration(duration: Duration) -> String {
    let seconds = duration.as_secs();
    match seconds {
        0 => "0s".into(),
        _ if seconds.is_multiple_of(60 * 60) => format!("{}h", seconds / (60 * 60)),
        _ if seconds.is_multiple_of(60) => format!("{}m", seconds / 60),
        _ => format!("{seconds}s"),
    }
}

pub(super) fn run(args: &[String]) -> i32 {
    let (agents, prompt, options, is_supervisor) =
        match parse_cluster_args(args, super::shortcut::get_is_available) {
            Ok(ClusterCommand::Help) => {
                cluster_help::print_help();
                return 0;
            }
            Ok(ClusterCommand::Launch {
                agents,
                prompt,
                options,
            }) => (agents, prompt, options, false),
            Ok(ClusterCommand::Supervise { agents, options }) => (agents, None, options, true),
            Err(err) => {
                eprintln!("herdr-switch: {err}");
                return 2;
            }
        };
    let (cwd, session) = match super::resolve_session() {
        Ok(session) => session,
        Err(err) => return super::report_error(&err),
    };
    let exe = match get_executable_path(crate::server::autodetect::get_spawn_exe()) {
        Ok(exe) => exe,
        Err(err) => return super::report_error(&err),
    };
    if is_supervisor {
        return cluster_background::supervise(&exe, &cwd, &session, &agents, &options);
    }
    if let Some(prompt) = prompt.as_deref().filter(|_| options.background) {
        return cluster_background::launch(&exe, &cwd, &session, &agents, prompt, &options);
    }
    let group = std::process::id();
    let ready_dir = options
        .exit_when_done
        .then(|| crate::session::data_dir_for(Some(&session)).join(format!("cluster-{group}")));
    if let Some(dir) = &ready_dir {
        if let Err(err) = create_ready_dir(dir) {
            return super::report_error(&err);
        }
    }
    for (index, agent) in agents.iter().enumerate().skip(1) {
        let ready_file = ready_dir
            .as_ref()
            .map(|dir| dir.join(format!("{index}.ready")).display().to_string());
        let spec = crate::platform::TerminalWindowSpec {
            argv: get_tab_argv(&exe, group, ready_file.as_deref(), None, agent),
            cwd: cwd.clone(),
        };
        if let Err(err) = crate::platform::open_terminal_tab(&spec) {
            eprintln!("herdr-switch: cannot open a terminal for {agent}: {err}");
        }
    }
    let outcome = OutcomeSlot::default();
    let leader = LeaderConfig {
        group,
        prompt,
        yolo: options.yolo.map(|mode| (mode, options.yolo_timeout)),
        watch: ready_dir.clone().map(|ready_dir| {
            let config = WatchConfig {
                ready_dir,
                leader_pane: None,
                timeout: options.timeout,
                idle_exit: options.idle_exit,
                capture_on_timeout: false,
            };
            (config, outcome.clone())
        }),
    };
    let code = super::launch(
        get_leader_argv(&agents[0], &options),
        None,
        Some(super::ClusterLaunch::Leader(leader)),
    )
    .unwrap_or_else(|err| super::report_error(&err));
    let outcome = outcome
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .take();
    let code = match outcome {
        Some(outcome) => {
            tracing::debug!(
                group,
                members = ?outcome.members,
                events = outcome.timeline.len(),
                "cluster supervisor ended"
            );
            let (message, code) = get_end_message(group, &options, &outcome);
            println!("{message}");
            code
        }
        // The leader agent exited by itself; members follow through their leader watch.
        None => code,
    };
    if let Some(dir) = &ready_dir {
        if let Err(err) = std::fs::remove_dir_all(dir) {
            tracing::warn!(path = %dir.display(), %err, "cannot remove cluster ready directory");
        }
    }
    code
}

/// Creates an empty ready directory; files left by an earlier cluster with this pid go.
fn create_ready_dir(dir: &Path) -> Result<(), String> {
    if dir.to_str().is_none() {
        return Err("the herdr session directory is not valid UTF-8".into());
    }
    match std::fs::remove_dir_all(dir) {
        Err(err) if err.kind() != std::io::ErrorKind::NotFound => {
            return Err(format!("cannot clear {}: {err}", dir.display()));
        }
        _ => {}
    }
    std::fs::create_dir_all(dir).map_err(|err| format!("cannot create {}: {err}", dir.display()))
}

/// A cluster tab runs `switch` tagged with the leader's group, plus its ready file
/// with `--exit-when-done`, and `--cluster-yolo <D>` for a background `--yolo` leader.
fn get_tab_argv(
    exe: &str,
    group: u32,
    ready_file: Option<&str>,
    yolo_timeout: Option<Duration>,
    agent: &str,
) -> Vec<String> {
    let mut argv = vec![
        exe.into(),
        "switch".into(),
        "--cluster-group".into(),
        group.to_string(),
    ];
    if let Some(ready_file) = ready_file {
        argv.extend(["--cluster-ready".into(), ready_file.into()]);
    }
    if let Some(yolo_timeout) = yolo_timeout {
        argv.extend(["--cluster-yolo".into(), format_yolo_timeout(yolo_timeout)]);
    }
    argv.push(agent.into());
    argv
}

/// The first agent's argv in this terminal; `--yolo` adds its family's flags.
fn get_leader_argv(agent: &str, options: &ClusterOptions) -> Vec<String> {
    let mut argv = vec![agent.to_owned()];
    if options.yolo == Some(YoloMode::Agent) {
        argv.extend(get_yolo_args(agent, options.yolo_timeout));
    }
    argv
}

/// The flags `--yolo` adds for the agent's CLI family, none for other agents.
/// A family is its base name, or any name that starts with the base name and `-` or `_`.
pub(super) fn get_yolo_args(program: &str, yolo_timeout: Duration) -> Vec<String> {
    let get_is_family = |base: &str| {
        program
            .strip_prefix(base)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with(['-', '_']))
    };
    if get_is_family("claude") {
        let mut args = CLAUDE_YOLO_ARGS.map(String::from).to_vec();
        // One argv element; the timeout is one of a fixed set, so the JSON stays valid.
        args.push(format!(
            r#"{{"askUserQuestionTimeout":"{}","skipDangerousModePermissionPrompt":true}}"#,
            get_question_timeout(yolo_timeout)
        ));
        return args;
    }
    let args: &[&str] = if get_is_family("codex") {
        &CODEX_YOLO_ARGS
    } else {
        &[]
    };
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

/// Claude's `askUserQuestionTimeout` for a `--yolo-timeout`: rounded up to 60s or 5m,
/// at most 10m. `never` would turn the timeout off, so it is never used.
fn get_question_timeout(yolo_timeout: Duration) -> &'static str {
    match yolo_timeout.as_secs() {
        0..=60 => "60s",
        61..=300 => "5m",
        _ => "10m",
    }
}

/// The final stdout line and exit code for a supervisor outcome.
fn get_end_message(group: u32, options: &ClusterOptions, outcome: &Outcome) -> (String, i32) {
    let closed = outcome.closed;
    let (text, code) = match outcome.reason {
        EndReason::Done => (
            format!(
                "all agents idle for {}; closed {closed} agent panes",
                format_duration(options.idle_exit)
            ),
            0,
        ),
        // Unreachable with `--timeout -1`: no deadline means no timeout.
        EndReason::Timeout => (
            format!(
                "timed out after {}; closed {closed} agent panes",
                options
                    .timeout
                    .map_or_else(|| "never".into(), format_duration)
            ),
            124,
        ),
        EndReason::AgentsExited => ("all agents exited".into(), 0),
        EndReason::ServerGone => ("lost the herdr server".into(), 1),
        // Only the background supervisor handles stop signals.
        EndReason::Stopped => (format!("stopped; closed {closed} agent panes"), 143),
    };
    (format!("easy-herdr cluster {group}: {text}"), code)
}

type OutcomeSlot = Arc<Mutex<Option<Outcome>>>;
type StartedCallback = Box<dyn FnOnce(&PassthroughStarted)>;

/// The cluster's first agent; cloned for each launch attempt.
#[derive(Clone)]
pub(super) struct LeaderConfig {
    pub(super) group: u32,
    prompt: Option<String>,
    /// The answer mode and how long a blocked leader waits.
    yolo: Option<(YoloMode, Duration)>,
    /// Set with `--exit-when-done`: the supervisor stores its outcome in the slot.
    watch: Option<(WatchConfig, OutcomeSlot)>,
}

/// Starts the cluster work of one launch attempt once its pane runs.
pub(super) fn get_started_callback(
    cluster: &super::ClusterLaunch,
    stop: &Arc<AtomicBool>,
) -> Option<StartedCallback> {
    let stop = stop.clone();
    match cluster.clone() {
        super::ClusterLaunch::Leader(LeaderConfig {
            prompt,
            yolo,
            watch,
            ..
        }) => {
            if prompt.is_none() && yolo.is_none() && watch.is_none() {
                return None;
            }
            Some(Box::new(move |started: &PassthroughStarted| {
                if let Some(prompt) = prompt {
                    spawn_prompt(started.pane_id.clone(), prompt);
                }
                if let Some((mode, grace)) = yolo {
                    cluster_yolo::spawn_watch(started.pane_id.clone(), mode, grace, stop.clone());
                }
                if let Some((config, outcome)) = watch {
                    let config = WatchConfig {
                        leader_pane: Some(started.pane_id.clone()),
                        ..config
                    };
                    spawn_supervisor(config, outcome, stop);
                }
            }))
        }
        super::ClusterLaunch::Member(member) => {
            let ready_file = member.ready_file?;
            Some(Box::new(move |started: &PassthroughStarted| {
                if let Err(err) = super::window::write_ready_file(&ready_file, &started.pane_id) {
                    tracing::warn!(path = %ready_file.display(), %err, "cannot write cluster ready file");
                }
                cluster_watch::spawn_leader_watch(member.group, stop);
            }))
        }
    }
}

/// Supervises the group, stores the outcome, then stops the leader's client.
fn spawn_supervisor(config: WatchConfig, outcome: OutcomeSlot, stop: Arc<AtomicBool>) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-watch", move || {
        let result = run_supervisor(&config, || None);
        *outcome.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
        stop.store(true, Ordering::SeqCst);
    }) {
        tracing::warn!(%err, "cannot start cluster supervisor");
    }
}

/// Supervises the group through the active session's API.
fn run_supervisor(config: &WatchConfig, interrupt: impl FnMut() -> Option<EndReason>) -> Outcome {
    let client = ApiClient::local();
    cluster_watch::supervise(
        config,
        |method| {
            client
                .request_value_with_timeout(
                    &Request {
                        id: "cluster:watch".into(),
                        method,
                    },
                    Duration::from_secs(5),
                )
                .and_then(parse_response_value)
                .map(|response| response.result)
        },
        Instant::now,
        std::thread::sleep,
        interrupt,
    )
}

fn get_executable_path(exe: std::io::Result<std::path::PathBuf>) -> Result<String, String> {
    let exe = exe
        .and_then(|exe| exe.canonicalize())
        .map_err(|err| format!("cannot resolve the herdr executable: {err}"))?;
    exe.into_os_string()
        .into_string()
        .map_err(|_| "the herdr executable path is not valid UTF-8".into())
}

fn spawn_prompt(pane_id: String, prompt: String) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-prompt", move || {
        let client = ApiClient::local();
        prompt_when_ready(
            &pane_id,
            &prompt,
            Duration::from_secs(120),
            |method, timeout| {
                client
                    .request_value_with_timeout(
                        &Request {
                            id: "cluster:prompt".into(),
                            method,
                        },
                        timeout,
                    )
                    .and_then(parse_response_value)
                    .map(|response| response.result)
            },
        );
    }) {
        tracing::warn!(%err, "cannot start cluster prompt thread");
    }
}

fn get_is_prompt_ready(agent: &AgentInfo) -> bool {
    agent.agent.is_some() && matches!(agent.agent_status, AgentStatus::Idle | AgentStatus::Done)
}

fn prompt_when_ready(
    pane_id: &str,
    prompt: &str,
    timeout: Duration,
    mut request: impl FnMut(Method, Duration) -> Result<ResponseResult, ApiClientError>,
) {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match request(
            Method::AgentGet(AgentTarget {
                target: pane_id.into(),
            }),
            remaining,
        ) {
            Ok(ResponseResult::AgentInfo { agent }) if get_is_prompt_ready(&agent) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match request(
                    Method::AgentPrompt(AgentPromptParams {
                        target: pane_id.into(),
                        text: prompt.into(),
                        wait: None,
                    }),
                    remaining,
                ) {
                    Ok(ResponseResult::AgentPrompted { .. }) => return,
                    Ok(_) => tracing::warn!(pane_id, "unexpected cluster prompt response"),
                    Err(err) => {
                        tracing::warn!(pane_id, %err, "cannot submit cluster prompt");
                        // A lost reply may follow a successful submission.
                        if !matches!(err, ApiClientError::ErrorResponse(_)) {
                            return;
                        }
                    }
                }
            }
            Ok(ResponseResult::AgentInfo { .. }) => {}
            Ok(_) => tracing::warn!(pane_id, "unexpected cluster agent response"),
            Err(err) => {
                tracing::debug!(pane_id, %err, "cannot poll cluster agent");
                last_error = Some(err);
            }
        }
        std::thread::sleep(
            Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    tracing::warn!(pane_id, error = ?last_error, "timed out waiting to submit cluster prompt");
}

#[cfg(test)]
#[path = "cluster_tests.rs"]
mod tests;
