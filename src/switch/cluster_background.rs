//! `easy-herdr cluster --background`: a detached supervisor runs every agent in a hidden PTY.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, Child, CommandBuilder, MasterPty, PtySize};

use super::cluster_log::{self, LogInfo};
use super::cluster_watch::{EndReason, WatchConfig};
use super::{format_duration, format_yolo_timeout, ClusterOptions};
use crate::switch::{ensure_server, report_error};

const PTY_SIZE: PtySize = PtySize {
    rows: 50,
    cols: 200,
    pixel_width: 0,
    pixel_height: 0,
};
/// How long the first agent may take to start before its prompt is dropped.
const READY_WAIT: Duration = Duration::from_secs(120);
/// How long members get to exit after the outcome before they are killed.
const EXIT_WAIT: Duration = Duration::from_secs(10);
/// Exit code after SIGTERM, SIGINT or SIGHUP, as for a process ended by SIGTERM.
const STOPPED_EXIT_CODE: i32 = 143;

/// Starts the supervisor in its own session and returns without waiting for it.
/// The prompt goes through the supervisor's stdin, never its argv or environment.
pub(super) fn launch(
    exe: &str,
    cwd: &Path,
    session: &str,
    agents: &[String],
    prompt: &str,
    options: &ClusterOptions,
) -> i32 {
    // A `-1` timeout never writes the timeout log, so a read-only cwd is fine.
    if options.timeout.is_some() {
        if let Err(err) = check_writable_dir(cwd) {
            tracing::debug!(%err, "background cluster cwd is not writable");
            return report_error(&format!(
                "background mode needs a writable working directory for its timeout log: {}",
                cwd.display()
            ));
        }
    }
    if let Err(err) =
        crate::session::select_session_for_process(session).and_then(|()| ensure_server())
    {
        return report_error(&err);
    }
    let mut command = std::process::Command::new(exe);
    command
        .args(get_supervisor_argv(agents, options))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    crate::platform::start_in_new_session(&mut command);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(err) => return report_error(&format!("cannot start the cluster supervisor: {err}")),
    };
    // Dropping stdin after the write closes the pipe, so the supervisor reads to EOF.
    if let Some(Err(err)) = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(prompt.as_bytes()))
    {
        return report_error(&format!(
            "cannot pass the prompt to the cluster supervisor: {err}"
        ));
    }
    // Not waited for: this process exits now, and init reaps the supervisor.
    print!(
        "{}",
        get_launch_message(child.id(), session, options.timeout)
    );
    0
}

/// Creates and removes a probe file, so the timeout log will not fail later.
fn check_writable_dir(dir: &Path) -> std::io::Result<()> {
    let probe = dir.join(format!(".easy-herdr-cluster-{}.probe", std::process::id()));
    std::fs::File::create_new(&probe)?;
    std::fs::remove_file(probe)
}

/// `cluster --supervise` with the parsed options and agents, but no prompt.
pub(super) fn get_supervisor_argv(agents: &[String], options: &ClusterOptions) -> Vec<String> {
    let timeout = match options.timeout {
        Some(duration) => format_duration(duration),
        None => "-1".into(),
    };
    let mut argv = ["cluster", "--supervise", "--background", "--timeout"]
        .map(String::from)
        .to_vec();
    argv.extend([
        timeout,
        "--idle-exit".into(),
        format_duration(options.idle_exit),
    ]);
    if options.yolo {
        argv.extend([
            "--yolo".into(),
            "--yolo-timeout".into(),
            format_yolo_timeout(options.yolo_timeout),
        ]);
    }
    argv.extend_from_slice(agents);
    argv
}

pub(super) fn get_launch_message(pid: u32, session: &str, timeout: Option<Duration>) -> String {
    let log = match timeout {
        Some(_) => format!("./easy-herdr-cluster-*-{pid}.log is written only if it times out"),
        None => "none: this cluster never times out".into(),
    };
    format!(
        "easy-herdr cluster {pid} started in the background\n\
         watch: easy-herdr --session {session} agent list\n\
         stop:  kill {pid}\n\
         log:   {log}\n"
    )
}

/// One agent's PTY; the master stays open until the member exits.
struct PtyMember {
    child: Box<dyn Child + Send + Sync>,
    _master: Box<dyn MasterPty + Send>,
}

/// Runs every agent as a PTY member, supervises the group, and returns the exit code.
/// Diagnostics go to `herdr-cluster.log` in the session data dir.
pub(super) fn supervise(
    exe: &str,
    cwd: &Path,
    session: &str,
    agents: &[String],
    options: &ClusterOptions,
) -> i32 {
    let started = cluster_log::now();
    let group = std::process::id();
    if let Err(err) = crate::session::select_session_for_process(session) {
        return report_error(&err);
    }
    crate::logging::init_file_logging("herdr-cluster.log");
    let mut prompt = String::new();
    if let Err(err) = std::io::stdin().read_to_string(&mut prompt) {
        tracing::error!(group, %err, "cannot read the cluster prompt");
        return 1;
    }
    if prompt.trim().is_empty() {
        tracing::error!(group, "the cluster prompt is blank");
        return 2;
    }
    let is_stopped = Arc::new(AtomicBool::new(false));
    let stop_flag = is_stopped.clone();
    if let Err(err) = ctrlc::set_handler(move || stop_flag.store(true, Ordering::SeqCst)) {
        tracing::warn!(group, %err, "cannot handle cluster stop signals");
    }
    let ready_dir = crate::session::data_dir_for(Some(session)).join(format!("cluster-{group}"));
    if let Err(err) = super::create_ready_dir(&ready_dir) {
        tracing::error!(group, %err, "cannot create the cluster ready directory");
        return 1;
    }
    let spawned = spawn_all(
        agents.len(),
        |index| {
            let argv = get_member_argv(exe, group, &ready_dir, index, &agents[index], options);
            spawn_member(&argv, cwd).map_err(|err| format!("cannot start {}: {err}", agents[index]))
        },
        |mut started| stop_members(&mut started, Duration::ZERO),
    );
    let mut members = match spawned {
        Ok(members) => members,
        Err(err) => {
            tracing::error!(group, %err, "background cluster aborted; started agents were stopped");
            remove_ready_dir(&ready_dir);
            return 1;
        }
    };
    tracing::info!(group, agents = agents.len(), "background cluster started");
    spawn_leader_work_when_ready(
        ready_dir.join("0.ready"),
        prompt.clone(),
        options.yolo.then_some(options.yolo_timeout),
        is_stopped.clone(),
    );
    let config = WatchConfig {
        ready_dir: ready_dir.clone(),
        leader_pane: None,
        timeout: options.timeout,
        idle_exit: options.idle_exit,
        capture_on_timeout: true,
    };
    let outcome = super::run_supervisor(&config, || {
        if is_stopped.load(Ordering::SeqCst) {
            return Some(EndReason::Stopped);
        }
        let is_running = |member: &mut PtyMember| matches!(member.child.try_wait(), Ok(None));
        (!members.iter_mut().any(is_running)).then_some(EndReason::AgentsExited)
    });
    tracing::info!(group, reason = ?outcome.reason, closed = outcome.closed, "background cluster ended");
    // A `-1` timeout has no deadline, so only a set timeout can produce a log.
    if let Some(timeout) = options
        .timeout
        .filter(|_| outcome.reason == EndReason::Timeout)
    {
        let log = cluster_log::render(&LogInfo {
            group,
            timeout,
            started,
            ended: cluster_log::now(),
            cwd,
            prompt: &prompt,
            outcome: &outcome,
        });
        let path = cwd.join(format!(
            "easy-herdr-cluster-{}-{group}.log",
            cluster_log::format_file_time(started)
        ));
        if let Err(err) = std::fs::write(&path, log) {
            tracing::error!(path = %path.display(), %err, "cannot write cluster timeout log");
        }
    }
    stop_members(&mut members, EXIT_WAIT);
    remove_ready_dir(&ready_dir);
    match outcome.reason {
        EndReason::Timeout => 124,
        EndReason::Stopped => STOPPED_EXIT_CODE,
        _ => 0,
    }
}

/// The `switch` argv of agent `index`; with `--yolo`, only the first agent gets
/// `--cluster-yolo <D>`.
pub(super) fn get_member_argv(
    exe: &str,
    group: u32,
    ready_dir: &Path,
    index: usize,
    agent: &str,
    options: &ClusterOptions,
) -> Vec<String> {
    let ready_file = ready_dir
        .join(format!("{index}.ready"))
        .display()
        .to_string();
    super::get_tab_argv(
        exe,
        group,
        Some(&ready_file),
        (options.yolo && index == 0).then_some(options.yolo_timeout),
        agent,
    )
}

/// Starts all `count` members or none: after a failure, `stop` gets the started ones.
pub(super) fn spawn_all<M>(
    count: usize,
    mut spawn: impl FnMut(usize) -> Result<M, String>,
    stop: impl FnOnce(Vec<M>),
) -> Result<Vec<M>, String> {
    let mut members = Vec::with_capacity(count);
    for index in 0..count {
        match spawn(index) {
            Ok(member) => members.push(member),
            Err(err) => {
                stop(members);
                return Err(err);
            }
        }
    }
    Ok(members)
}

/// Spawns argv in a new PTY whose output is read and dropped, so the member never blocks.
fn spawn_member(argv: &[String], cwd: &Path) -> Result<PtyMember, String> {
    let pair = native_pty_system()
        .openpty(PTY_SIZE)
        .map_err(|err| err.to_string())?;
    // Clone the reader first, so no later failure leaves a child behind.
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|err| err.to_string())?;
    let mut command = CommandBuilder::new(&argv[0]);
    command.args(&argv[1..]);
    command.cwd(cwd);
    if command.get_env("TERM").is_none() {
        command.env("TERM", "xterm-256color");
    }
    let child = pair
        .slave
        .spawn_command(command)
        .map_err(|err| err.to_string())?;
    // Only the child keeps the slave open, so the reader sees EOF when it exits.
    drop(pair.slave);
    let mut member = PtyMember {
        child,
        _master: pair.master,
    };
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-pty", move || {
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    }) {
        stop_members(std::slice::from_mut(&mut member), Duration::ZERO);
        return Err(format!("cannot start its PTY reader: {err}"));
    }
    Ok(member)
}

/// Submits the prompt to the first agent once its ready file names its pane, and
/// with `--yolo` starts watching it for dialogs.
fn spawn_leader_work_when_ready(
    ready_file: PathBuf,
    prompt: String,
    yolo: Option<Duration>,
    stop: Arc<AtomicBool>,
) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-ready", move || {
        let deadline = Instant::now() + READY_WAIT;
        while Instant::now() < deadline {
            if let Ok(pane_id) = std::fs::read_to_string(&ready_file) {
                let pane_id = pane_id.trim().to_owned();
                if let Some(grace) = yolo {
                    super::cluster_yolo::spawn_watch(pane_id.clone(), grace, stop);
                }
                super::spawn_prompt(pane_id, prompt);
                return;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        tracing::warn!("timed out waiting for the first background cluster agent");
    }) {
        tracing::warn!(%err, "cannot start cluster prompt wait");
    }
}

/// Waits up to `wait` for members to exit, then kills the rest; every child is reaped.
fn stop_members(members: &mut [PtyMember], wait: Duration) {
    let deadline = Instant::now() + wait;
    while Instant::now() < deadline
        && members
            .iter_mut()
            .any(|member| matches!(member.child.try_wait(), Ok(None)))
    {
        std::thread::sleep(Duration::from_millis(100));
    }
    for member in members {
        if matches!(member.child.try_wait(), Ok(None)) {
            if let Err(err) = member.child.kill() {
                tracing::warn!(%err, "cannot kill background cluster agent");
            }
        }
        if let Err(err) = member.child.wait() {
            tracing::warn!(%err, "cannot reap background cluster agent");
        }
    }
}

fn remove_ready_dir(ready_dir: &Path) {
    if let Err(err) = std::fs::remove_dir_all(ready_dir) {
        tracing::warn!(path = %ready_dir.display(), %err, "cannot remove cluster ready directory");
    }
}
