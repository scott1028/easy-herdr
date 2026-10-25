//! End-to-end proof that a finished, timed-out or stopped cluster leaves no supervisor,
//! member, agent or herdr server behind.

#![cfg(target_os = "linux")]

pub mod support;

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt as _;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use support::{cleanup_test_base, herdr_server_pids_for_runtime_dir, register_runtime_dir};

const BIN: &str = env!("CARGO_BIN_EXE_easy-herdr");
/// Covers the supervisor's 2 s polls, member exit and the server's 5 s idle exit.
const GONE_WAIT: Duration = Duration::from_secs(40);

/// Named `pi`, so Herdr identifies it by process name and accepts `agent.prompt`.
/// `FAKE_MODE=finish` works for 4 s per prompt; `stuck` stays working. 4 s is longer
/// than the supervisor's 2 s poll: under load Herdr can identify the process late and
/// treat the first idle as startup, so the completion marker alone is not enough.
const FAKE_AGENT: &str = r#"#!/bin/sh
echo $$ >> "$FAKE_DIR/pids"
report() {
    "$FAKE_BIN" pane report-agent "$HERDR_PANE_ID" --source custom:fake --agent pi --state "$1" >/dev/null
}
echo "fake-agent-marker $HERDR_PANE_ID"
report idle
while IFS= read -r prompt; do
    printf '%s %s\n' "$HERDR_PANE_ID" "$prompt" >> "$FAKE_DIR/prompts"
    report working
    [ "$FAKE_MODE" = stuck ] && continue
    sleep 4
    report idle
done
"#;

/// Named `claude-x` and `codex-x`, the claude and codex families of `cluster --yolo`.
/// Records each launch as `name|arg|...`, reports itself blocked, then waits.
const FAMILY_AGENT: &str = r#"#!/bin/sh
echo $$ >> "$FAKE_DIR/pids"
{ printf '%s' "${0##*/}"; for arg in "$@"; do printf '|%s' "$arg"; done; echo; } >> "$FAKE_DIR/args"
"$FAKE_BIN" pane report-agent "$HERDR_PANE_ID" --source custom:fake --agent pi --state blocked >/dev/null
while IFS= read -r line; do :; done
"#;
/// `HERDR_SWITCH_TERMINAL`: records each member tab's argv as `arg|...|` and opens nothing.
const FAKE_TERMINAL: &str = r#"#!/bin/sh
{ for arg in "$@"; do printf '%s|' "$arg"; done; echo; } >> "$FAKE_DIR/tabs"
"#;
const CLAUDE_YOLO_ARGS: &str = r#"|--permission-mode|bypassPermissions|--settings|{"askUserQuestionTimeout":"60s","skipDangerousModePermissionPrompt":true}"#;
const CODEX_YOLO_ARGS: &str = "|--ask-for-approval|never";

/// One isolated cluster run: its own config, runtime, home, PATH and cwd.
struct Fixture {
    base: PathBuf,
    mode: &'static str,
    /// Every pid seen during the run, with its start time to tell a reused pid apart.
    observed: RefCell<HashMap<u32, u64>>,
}

impl Fixture {
    fn new(mode: &'static str) -> Self {
        static COUNT: AtomicUsize = AtomicUsize::new(0);
        // Short, so session socket paths stay under the Unix limit.
        let base = PathBuf::from(format!(
            "/tmp/hcx-{}-{}",
            std::process::id(),
            COUNT.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&base);
        for dir in ["config", "home", "bin", "agents", "work"] {
            fs::create_dir_all(base.join(dir)).unwrap();
        }
        register_runtime_dir(&base.join("runtime"));
        for (name, script) in [
            ("pi", FAKE_AGENT),
            ("claude-x", FAMILY_AGENT),
            ("codex-x", FAMILY_AGENT),
            ("fake-terminal", FAKE_TERMINAL),
        ] {
            let path = base.join("bin").join(name);
            fs::write(&path, script).unwrap();
            fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            base,
            mode,
            observed: RefCell::default(),
        }
    }

    fn work(&self) -> PathBuf {
        self.base.join("work")
    }

    /// A cleared environment, so the pane these tests run in (APPIMAGE, HERDR_*) does not leak in.
    fn env(&self) -> Vec<(&'static str, String)> {
        let path = |dir: &str| self.base.join(dir).display().to_string();
        vec![
            ("PATH", format!("{}:/usr/bin:/bin", path("bin"))),
            ("HOME", path("home")),
            ("XDG_CONFIG_HOME", path("config")),
            ("XDG_RUNTIME_DIR", path("runtime")),
            ("SHELL", "/bin/sh".into()),
            ("FAKE_BIN", BIN.into()),
            ("FAKE_DIR", path("agents")),
            ("FAKE_MODE", self.mode.into()),
            ("HERDR_SWITCH_TERMINAL", path("bin/fake-terminal")),
        ]
    }

    fn cluster(&self, args: &[&str]) -> std::process::Output {
        Command::new(BIN)
            .arg("cluster")
            .args(args)
            .env_clear()
            .envs(self.env())
            .current_dir(self.work())
            .output()
            .unwrap()
    }

    /// Runs a foreground cluster in a PTY and returns its exit code and output.
    fn cluster_in_pty(&self, args: &[&str], timeout: Duration) -> (u32, String) {
        let pair = native_pty_system()
            .openpty(PtySize {
                rows: 24,
                cols: 100,
                pixel_width: 0,
                pixel_height: 0,
            })
            .unwrap();
        let mut command = CommandBuilder::new(BIN);
        command.arg("cluster");
        command.args(args);
        command.env_clear();
        for (key, value) in self.env() {
            command.env(key, value);
        }
        command.env("TERM", "xterm-256color");
        command.cwd(self.work());
        let mut child = pair.slave.spawn_command(command).unwrap();
        drop(pair.slave);
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut reader = pair.master.try_clone_reader().unwrap();
        let sink = output.clone();
        std::thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(read @ 1..) = reader.read(&mut buffer) {
                sink.lock().unwrap().extend_from_slice(&buffer[..read]);
            }
        });
        let deadline = Instant::now() + timeout;
        let status = loop {
            self.observe_run();
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!(
                    "cluster did not exit in {timeout:?}\n{}",
                    self.diagnostics()
                );
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        // Let the reader drain what the cluster printed last.
        std::thread::sleep(Duration::from_millis(200));
        let text = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
        (status.exit_code(), text)
    }

    fn agent_file(&self, name: &str) -> String {
        fs::read_to_string(self.base.join("agents").join(name)).unwrap_or_default()
    }

    fn agent_pids(&self) -> Vec<u32> {
        self.agent_file("pids")
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect()
    }

    /// Every live process of this build whose runtime dir is this fixture's.
    fn herdr_pids(&self) -> Vec<u32> {
        let runtime = format!("XDG_RUNTIME_DIR={}", self.base.join("runtime").display());
        let binary = fs::canonicalize(BIN).unwrap();
        let Ok(entries) = fs::read_dir("/proc") else {
            return Vec::new();
        };
        entries
            .filter_map(|entry| entry.ok()?.file_name().to_str()?.parse::<u32>().ok())
            .filter(|pid| fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|exe| exe == binary))
            .filter(|pid| {
                fs::read(format!("/proc/{pid}/environ")).is_ok_and(|environ| {
                    environ
                        .split(|byte| *byte == 0)
                        .any(|entry| entry == runtime.as_bytes())
                })
            })
            .collect()
    }

    fn observe(&self, pids: impl IntoIterator<Item = u32>) {
        let mut observed = self.observed.borrow_mut();
        for pid in pids {
            if let Some(start) = start_time(pid) {
                observed.entry(pid).or_insert(start);
            }
        }
    }

    /// Records the agents and every easy-herdr process of this run while they live.
    fn observe_run(&self) {
        self.observe(self.agent_pids().into_iter().chain(self.herdr_pids()));
    }

    /// Observed pids whose `/proc` entry still exists, zombies included.
    fn unreaped(&self) -> Vec<u32> {
        let observed = self.observed.borrow();
        let mut pids = observed
            .iter()
            .filter(|(pid, start)| start_time(**pid) == Some(**start))
            .map(|(pid, _)| *pid)
            .collect::<Vec<_>>();
        pids.sort_unstable();
        pids
    }

    /// Waits until the agents, every easy-herdr process and every server of this run are
    /// reaped: their `/proc` entries are gone, so a zombie does not count as gone.
    fn assert_all_gone(&self, extra: &[u32]) {
        let deadline = Instant::now() + GONE_WAIT;
        loop {
            self.observe(extra.iter().copied());
            self.observe_run();
            let agents = self.agent_pids();
            let left = self.unreaped();
            // An unreadable /proc entry of another test's exiting server is an Err: poll again.
            let servers = herdr_server_pids_for_runtime_dir(&self.base.join("runtime"));
            if left.is_empty() && servers.as_ref().is_ok_and(Vec::is_empty) {
                assert!(
                    !agents.is_empty(),
                    "no agent ever started\n{}",
                    self.diagnostics()
                );
                // The check covered easy-herdr processes too, not only the agents.
                assert!(self.observed.borrow().len() > agents.len());
                return;
            }
            if Instant::now() > deadline {
                panic!(
                    "still running after {GONE_WAIT:?}: {left:?}, servers {servers:?}\n{}",
                    self.diagnostics()
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    fn wait_for(&self, what: &str, mut condition: impl FnMut() -> bool) {
        if !support::wait_until(Duration::from_secs(20), Duration::from_millis(50), || {
            self.observe_run();
            condition()
        }) {
            panic!("timed out waiting for {what}\n{}", self.diagnostics());
        }
    }

    /// Files under the config dir with these names, e.g. a session's ready dir or logs.
    fn find(&self, name: &str) -> Vec<PathBuf> {
        let mut found = Vec::new();
        let mut dirs = vec![self.base.join("config")];
        while let Some(dir) = dirs.pop() {
            for entry in fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path
                    .file_name()
                    .is_some_and(|file| file.to_string_lossy().starts_with(name))
                {
                    found.push(path.clone());
                }
                if path.is_dir() {
                    dirs.push(path);
                }
            }
        }
        found
    }

    /// The pane id that the running cluster's `<index>.ready` file names.
    fn ready_pane(&self, index: usize) -> String {
        let name = format!("{index}.ready");
        let mut pane = String::new();
        self.wait_for(&name, || {
            pane = self
                .find(&name)
                .iter()
                .find(|path| path.file_name().is_some_and(|file| file == name.as_str()))
                .and_then(|path| fs::read_to_string(path).ok())
                .unwrap_or_default()
                .trim()
                .to_owned();
            !pane.is_empty()
        });
        pane
    }

    /// Every log file with this name under the config dir, joined.
    fn log(&self, name: &str) -> String {
        self.find(name)
            .iter()
            .map(|path| fs::read_to_string(path).unwrap_or_default())
            .collect()
    }

    fn timeout_logs(&self) -> Vec<PathBuf> {
        fs::read_dir(self.work())
            .unwrap()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.to_string_lossy().ends_with(".log"))
            .collect()
    }

    fn diagnostics(&self) -> String {
        let mut text = format!(
            "agent pids: {:?}\nprompts: {:?}\n",
            self.agent_pids(),
            self.agent_file("prompts")
        );
        for session in self.find("sw-").iter().filter_map(|path| path.file_name()) {
            let output = Command::new(BIN)
                .arg("--session")
                .arg(session)
                .args(["agent", "list"])
                .env_clear()
                .envs(self.env())
                .output();
            if let Ok(output) = output {
                text.push_str(&format!(
                    "== agent list {}\n{}{}\n",
                    session.to_string_lossy(),
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                ));
            }
        }
        for name in ["herdr-server.log", "herdr-cluster.log", "herdr-client.log"] {
            for path in self.find(name) {
                let log = fs::read_to_string(&path).unwrap_or_default();
                let tail = log.lines().rev().take(60).collect::<Vec<_>>();
                text.push_str(&format!("== {}\n", path.display()));
                for line in tail.into_iter().rev() {
                    text.push_str(line);
                    text.push('\n');
                }
            }
        }
        text
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        // Kill whatever a failed test left, then remove the base and its servers.
        let left = self.unreaped();
        for pid in self.herdr_pids().into_iter().chain(left) {
            unsafe {
                libc::kill(pid as libc::pid_t, libc::SIGKILL);
            }
        }
        let _ = fs::set_permissions(self.work(), fs::Permissions::from_mode(0o755));
        cleanup_test_base(&self.base);
    }
}

/// Field 22 of `/proc/<pid>/stat`; `None` once the process is reaped.
fn start_time(pid: u32) -> Option<u64> {
    let stat = fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")?.1.split(' ').nth(19)?.parse().ok()
}

/// Starts a background cluster and returns the supervisor pid it printed.
fn launch_background(fixture: &Fixture, args: &[&str]) -> u32 {
    let started = Instant::now();
    let output = fixture.cluster(&[&["--background"], args].concat());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "launcher took {:?}",
        started.elapsed()
    );
    stdout
        .strip_prefix("easy-herdr cluster ")
        .and_then(|rest| rest.split_once(" started in the background"))
        .and_then(|(pid, _)| pid.parse().ok())
        .unwrap_or_else(|| panic!("no supervisor pid in {stdout:?}"))
}

#[test]
fn background_cluster_closes_agents_and_server_when_all_idle() {
    let fixture = Fixture::new("finish");
    let supervisor = launch_background(
        &fixture,
        &[
            "--idle-exit",
            "1s",
            "--timeout",
            "60s",
            "pi",
            "pi",
            "pi",
            "--",
            "do the task",
        ],
    );
    let first = fixture.ready_pane(0);
    fixture.assert_all_gone(&[supervisor]);
    assert_eq!(fixture.agent_pids().len(), 3, "{}", fixture.diagnostics());
    // Exactly the first agent, the one in 0.ready, got the prompt, once.
    assert_eq!(
        fixture.agent_file("prompts"),
        format!("{first} do the task\n")
    );
    assert!(fixture.timeout_logs().is_empty());
    assert!(fixture.find("cluster-").is_empty(), "ready dir left");
}

#[test]
fn background_cluster_without_a_timeout_finishes_on_idle_exit() {
    let fixture = Fixture::new("finish");
    let supervisor = launch_background(
        &fixture,
        &[
            "--idle-exit",
            "1s",
            "--timeout",
            "-1",
            "pi",
            "--",
            "do the task",
        ],
    );
    fixture.assert_all_gone(&[supervisor]);
    assert!(fixture.timeout_logs().is_empty());
    assert!(fixture.find("cluster-").is_empty(), "ready dir left");
}

#[test]
fn background_cluster_times_out_and_writes_log() {
    let fixture = Fixture::new("stuck");
    let supervisor = launch_background(&fixture, &["--timeout", "3s", "pi", "pi", "--", "loop"]);
    fixture.assert_all_gone(&[supervisor]);
    let logs = fixture.timeout_logs();
    assert_eq!(logs.len(), 1, "{logs:?}\n{}", fixture.diagnostics());
    let log = fs::read_to_string(&logs[0]).unwrap();
    for line in [
        "result:   timeout (3s elapsed)",
        "## agents",
        "## timeline",
        "prompt:   loop",
    ] {
        assert!(log.contains(line), "missing {line:?}:\n{log}");
    }
    let screens = log
        .lines()
        .filter_map(|line| line.strip_prefix("## ")?.strip_suffix(" (last 200 lines)"))
        .collect::<Vec<_>>();
    assert_eq!(screens.len(), 2, "{log}");
    for screen in screens {
        let pane = screen.split(' ').next().unwrap();
        assert!(
            log.contains(&format!("fake-agent-marker {pane}")),
            "{pane}:\n{log}"
        );
    }
}

#[test]
fn foreground_cluster_exits_when_all_agents_idle() {
    let fixture = Fixture::new("finish");
    let (code, output) = fixture.cluster_in_pty(
        &["--exit-when-done", "--idle-exit", "1s", "pi", "--", "go"],
        Duration::from_secs(40),
    );
    assert_eq!(code, 0, "{output}\n{}", fixture.diagnostics());
    assert!(
        output.contains("all agents idle for 1s; closed 1 agent panes"),
        "{output}"
    );
    let prompts = fixture.agent_file("prompts");
    assert!(
        prompts.starts_with("w1:p") && prompts.ends_with(" go\n") && prompts.lines().count() == 1,
        "{prompts:?}"
    );
    fixture.assert_all_gone(&[]);
}

#[test]
fn foreground_cluster_timeout_prints_message_and_exits_124() {
    let fixture = Fixture::new("stuck");
    let (code, output) = fixture.cluster_in_pty(
        &["--exit-when-done", "--timeout", "3s", "pi", "--", "go"],
        Duration::from_secs(40),
    );
    assert_eq!(code, 124, "{output}\n{}", fixture.diagnostics());
    assert!(
        output.contains("timed out after 3s; closed 1 agent panes"),
        "{output}"
    );
    fixture.assert_all_gone(&[]);
}

/// Starts two stuck members, waits for both, then sends `signal` to the supervisor.
fn signal_running_background_cluster(fixture: &Fixture, signal: libc::c_int) -> u32 {
    let supervisor = launch_background(fixture, &["--timeout", "60s", "pi", "pi", "--", "hold"]);
    fixture.wait_for("both members to start", || {
        fixture.agent_pids().len() == 2 && fixture.find("cluster-").len() == 1
    });
    fixture.wait_for("the prompt", || !fixture.agent_file("prompts").is_empty());
    unsafe {
        libc::kill(supervisor as libc::pid_t, signal);
    }
    supervisor
}

#[test]
fn cluster_members_exit_when_the_leader_dies() {
    let fixture = Fixture::new("stuck");
    let supervisor = signal_running_background_cluster(&fixture, libc::SIGKILL);
    fixture.assert_all_gone(&[supervisor]);
}

#[test]
fn background_cluster_stops_cleanly_on_sigterm() {
    let fixture = Fixture::new("stuck");
    let supervisor = signal_running_background_cluster(&fixture, libc::SIGTERM);
    fixture.assert_all_gone(&[supervisor]);
    assert!(
        fixture.find("cluster-").is_empty(),
        "ready dir left\n{}",
        fixture.diagnostics()
    );
    assert!(fixture.timeout_logs().is_empty());
    let cluster_log = fixture
        .find("herdr-cluster.log")
        .iter()
        .map(|path| fs::read_to_string(path).unwrap_or_default())
        .collect::<String>();
    assert!(cluster_log.contains("reason=Stopped"), "{cluster_log}");
}

#[test]
fn background_cluster_rejects_a_read_only_cwd() {
    let fixture = Fixture::new("finish");
    fs::set_permissions(fixture.work(), fs::Permissions::from_mode(0o555)).unwrap();
    let output = fixture.cluster(&["--background", "pi", "--", "hi"]);
    assert_eq!(output.status.code(), Some(1), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("background mode needs a writable working directory for its timeout log"),
        "{stderr}"
    );
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(fixture.herdr_pids().is_empty());
    let runtime = fixture.base.join("runtime");
    assert!(support::wait_until(
        Duration::from_secs(5),
        Duration::from_millis(50),
        || herdr_server_pids_for_runtime_dir(&runtime).is_ok_and(|servers| servers.is_empty())
    ));
}

#[test]
fn background_cluster_allows_a_read_only_cwd_without_a_timeout() {
    let fixture = Fixture::new("finish");
    fs::set_permissions(fixture.work(), fs::Permissions::from_mode(0o555)).unwrap();
    let supervisor = launch_background(
        &fixture,
        &[
            "--idle-exit",
            "1s",
            "--timeout",
            "-1",
            "pi",
            "--",
            "do the task",
        ],
    );
    fixture.assert_all_gone(&[supervisor]);
    assert!(fixture.timeout_logs().is_empty());
}

/// Runs a background cluster of two family agents, then checks each agent's argv and
/// that only `--yolo` starts one watch, on the first agent's pane.
fn check_background_yolo(yolo: bool, agents: [&str; 2], leader_args: &str) {
    let fixture = Fixture::new("stuck");
    let flags: &[&str] = if yolo {
        &["--yolo", "--yolo-timeout", "0"]
    } else {
        &[]
    };
    let args = [&["--timeout", "3s"][..], flags, &agents, &["--", "go"]].concat();
    let supervisor = launch_background(&fixture, &args);
    let leader = fixture.ready_pane(0);
    fixture.assert_all_gone(&[supervisor]);
    let mut launches = fixture
        .agent_file("args")
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    launches.sort();
    let mut expected = vec![format!("{}{leader_args}", agents[0]), agents[1].to_owned()];
    expected.sort();
    assert_eq!(launches, expected, "{}", fixture.diagnostics());
    let log = fixture.log("herdr-cluster.log");
    let started = log
        .lines()
        .filter(|line| line.contains("cluster yolo watch started"))
        .collect::<Vec<_>>();
    assert_eq!(started.len(), usize::from(yolo), "{log}");
    let leader_field = format!("pane={leader}");
    assert!(
        started
            .iter()
            .all(|line| line.split_whitespace().any(|field| field == leader_field)),
        "{log}"
    );
    assert!(!log.contains("cluster yolo sent keys"), "{log}");
    // Only the supervisor watches; no agent's switch client starts a watch.
    let client_log = fixture.log("herdr-client.log");
    assert!(
        !client_log.contains("cluster yolo watch started"),
        "{client_log}"
    );
}

#[test]
fn background_yolo_adds_codex_flags_to_the_first_agent_only() {
    check_background_yolo(true, ["codex-x", "claude-x"], CODEX_YOLO_ARGS);
}

#[test]
fn background_yolo_adds_claude_flags_to_the_first_agent_only() {
    check_background_yolo(true, ["claude-x", "codex-x"], CLAUDE_YOLO_ARGS);
}

#[test]
fn background_cluster_without_yolo_adds_no_flags_and_no_watch() {
    check_background_yolo(false, ["claude-x", "codex-x"], "");
}

/// Runs a foreground cluster whose member tab only records its argv, then checks the
/// leader's argv, the tab's argv and whether the leader's client started a watch.
fn check_foreground_yolo(yolo: bool) {
    let fixture = Fixture::new("stuck");
    let flags: &[&str] = if yolo {
        &["--yolo", "--yolo-timeout", "0"]
    } else {
        &[]
    };
    let args = [
        &["--exit-when-done", "--timeout", "3s"][..],
        flags,
        &["claude-x", "codex-x", "--", "go"],
    ]
    .concat();
    let (code, output) = fixture.cluster_in_pty(&args, Duration::from_secs(40));
    assert_eq!(code, 124, "{output}\n{}", fixture.diagnostics());
    let leader_args = if yolo { CLAUDE_YOLO_ARGS } else { "" };
    assert_eq!(
        fixture.agent_file("args"),
        format!("claude-x{leader_args}\n"),
        "{}",
        fixture.diagnostics()
    );
    let tabs = fixture.agent_file("tabs");
    let tab = tabs.lines().collect::<Vec<_>>();
    assert_eq!(tab.len(), 1, "{tabs}");
    let tab = tab[0].split('|').collect::<Vec<_>>();
    assert_eq!(&tab[1..3], ["switch", "--cluster-group"], "{tabs}");
    assert!(tab.ends_with(&["codex-x", ""]), "{tabs}");
    assert!(!tab.contains(&"--cluster-yolo"), "{tabs}");
    let log = fixture.log("herdr-client.log");
    assert_eq!(
        log.matches("cluster yolo watch started").count(),
        usize::from(yolo),
        "{log}"
    );
    assert!(!log.contains("cluster yolo sent keys"), "{log}");
    fixture.assert_all_gone(&[]);
}

#[test]
fn foreground_yolo_adds_claude_flags_to_the_leader_but_not_its_member_tab() {
    check_foreground_yolo(true);
}

#[test]
fn foreground_cluster_without_yolo_adds_no_flags_and_no_watch() {
    check_foreground_yolo(false);
}
