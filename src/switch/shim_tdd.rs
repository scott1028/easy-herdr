use super::*;
use std::collections::HashMap;
use std::fs;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT_SCRATCH: AtomicUsize = AtomicUsize::new(0);

/// A scratch directory removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "herdr-switch-shim-{}-{}",
            std::process::id(),
            NEXT_SCRATCH.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir.canonicalize().unwrap())
    }

    fn dir(&self, name: &str) -> PathBuf {
        let dir = self.0.join(name);
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The variables a command sets (`Some`) or removes (`None`).
fn command_env(command: &Command) -> HashMap<String, Option<String>> {
    command
        .get_envs()
        .map(|(key, value)| {
            (
                key.to_string_lossy().into_owned(),
                value.map(|value| value.to_string_lossy().into_owned()),
            )
        })
        .collect()
}

#[test]
fn shims_are_recognized_by_name_only_inside_a_switch_launch() {
    let launch = Some(OsStr::new("/home/u/.herdr-switch/shims/abc"));

    assert_eq!(shim_program("codex", launch), Some("codex"));
    assert_eq!(shim_program("/usr/bin/claude", launch), Some("claude"));
    assert_eq!(shim_program("herdr", launch), None);
    assert_eq!(shim_program("codex", None), None);
    assert_eq!(shim_program("codex", Some(OsStr::new(""))), None);
}

#[test]
fn path_without_drops_every_copy_of_the_shim_dir() {
    let path = OsStr::new("/shim:/usr/bin:/shim:/bin");

    assert_eq!(path_without(path, Path::new("/shim")), "/usr/bin:/bin");
}

#[test]
fn shims_link_to_the_running_herdr_in_a_directory_per_executable() {
    let scratch = Scratch::new();
    let root = scratch.dir("root");
    let stable = scratch.0.join("stable-herdr");
    let dev = scratch.0.join("dev-herdr");

    let stable_dir = install_shims(&root, &stable).unwrap();
    let dev_dir = install_shims(&root, &dev).unwrap();

    assert_ne!(stable_dir, dev_dir);
    for program in ["codex", "claude", "opencode"] {
        assert_eq!(fs::read_link(stable_dir.join(program)).unwrap(), stable);
        assert_eq!(fs::read_link(dev_dir.join(program)).unwrap(), dev);
    }
    assert_eq!(install_shims(&root, &stable).unwrap(), stable_dir);
}

fn args(command: &Command) -> Vec<&OsStr> {
    command.get_args().collect()
}

#[test]
fn codex_shim_runs_codex_without_the_shared_daemon_from_its_real_home() {
    let scratch = Scratch::new();
    let real = scratch.dir("real-codex");
    let root = scratch.dir("root");
    let path = OsStr::new("/shim:/usr/bin");

    let command = shim_command(
        "codex",
        &["--model".into(), "x".into()],
        Some(&real),
        &root,
        path,
        Path::new("/shim"),
    )
    .unwrap();

    let env = command_env(&command);
    assert!(!env.contains_key("CODEX_HOME"), "{env:?}");
    assert_eq!(env["PATH"].as_deref(), Some("/usr/bin"));
    assert_eq!(env[SHIM_DIR_ENV], None);
    assert_eq!(command.get_program(), "codex");
    assert_eq!(
        args(&command),
        [
            OsStr::new("--no-daemon"),
            OsStr::new("--model"),
            OsStr::new("x")
        ]
    );
    let links: Vec<_> = fs::read_dir(root.join("codex"))
        .unwrap()
        .map(|entry| fs::read_link(entry.unwrap().path()).unwrap())
        .collect();
    assert_eq!(links, [real]);
}

#[test]
fn codex_shim_leaves_daemon_choices_the_caller_already_made() {
    let scratch = Scratch::new();
    let real = scratch.dir("real-codex");
    let root = scratch.dir("root");

    for caller_args in [
        vec!["--no-daemon".to_owned()],
        vec!["--remote".to_owned(), "ws://host".to_owned()],
        vec!["--remote=ws://host".to_owned()],
        vec!["agents".to_owned()],
        vec!["remote-control".to_owned(), "pair".to_owned()],
    ] {
        let command = shim_command(
            "codex",
            &caller_args,
            Some(&real),
            &root,
            OsStr::new("/usr/bin"),
            Path::new("/shim"),
        )
        .unwrap();

        assert_eq!(
            args(&command),
            caller_args.iter().map(OsStr::new).collect::<Vec<_>>()
        );
    }
}

#[test]
fn claude_shim_links_its_home_and_runs_claude_from_the_real_home() {
    let scratch = Scratch::new();
    let real = scratch.dir("real-claude");
    let root = scratch.dir("root");

    let command = shim_command(
        "claude",
        &[],
        Some(&real),
        &root,
        OsStr::new("/shim:/usr/bin"),
        Path::new("/shim"),
    )
    .unwrap();

    let env = command_env(&command);
    assert!(!env.contains_key("CLAUDE_CONFIG_DIR"));
    assert_eq!(env["PATH"].as_deref(), Some("/usr/bin"));
    let links: Vec<_> = fs::read_dir(root.join("claude"))
        .unwrap()
        .map(|entry| fs::read_link(entry.unwrap().path()).unwrap())
        .collect();
    assert_eq!(links, [real]);
}

#[test]
fn codex_shim_reads_daemon_choices_only_before_the_prompt_separator() {
    let scratch = Scratch::new();
    let caller_args = vec!["--".to_owned(), "--remote explained".to_owned()];

    let command = shim_command(
        "codex",
        &caller_args,
        Some(&scratch.dir("real-codex")),
        &scratch.dir("root"),
        OsStr::new("/usr/bin"),
        Path::new("/shim"),
    )
    .unwrap();

    assert_eq!(
        args(&command),
        [
            OsStr::new("--no-daemon"),
            OsStr::new("--"),
            OsStr::new("--remote explained")
        ]
    );
}

/// The arguments the opencode shim passes on for `caller_args`.
fn opencode_args(caller_args: &[&str]) -> Vec<String> {
    let scratch = Scratch::new();
    let caller_args: Vec<String> = caller_args.iter().map(|arg| (*arg).to_owned()).collect();
    let command = shim_command(
        "opencode",
        &caller_args,
        None,
        &scratch.dir("root"),
        OsStr::new("/usr/bin"),
        Path::new("/shim"),
    )
    .unwrap();
    args(&command)
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

#[test]
fn opencode_shim_runs_its_interface_with_a_private_server() {
    let scratch = Scratch::new();
    let project = scratch.dir("project");
    let project = project.to_str().unwrap();

    for caller_args in [
        vec![],
        vec!["--log-level", "debug"],
        vec!["-s", "models", "--prompt", "mcp"],
        vec!["--auto", project],
    ] {
        let mut expected = vec!["--standalone"];
        expected.extend(&caller_args);
        assert_eq!(opencode_args(&caller_args), expected);
    }
}

#[test]
fn opencode_shim_leaves_subcommands_and_server_choices_alone() {
    for caller_args in [
        vec!["mcp", "list"],
        vec!["--log-level", "debug", "service", "status"],
        vec!["unknown-subcommand"],
        vec!["--standalone"],
        vec!["--standalone=false"],
        vec!["--no-standalone"],
        vec!["--server", "http://127.0.0.1:4096"],
        vec!["--server=http://127.0.0.1:4096"],
    ] {
        assert_eq!(opencode_args(&caller_args), caller_args);
    }
}

#[test]
fn opencode_subcommands_win_over_directories_of_the_same_name() {
    let caller_args = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let every_name_is_a_directory = |_: &str| true;

    assert!(!opencode_opens_interface(
        &caller_args(&["service", "status"]),
        every_name_is_a_directory
    ));
    assert!(opencode_opens_interface(
        &caller_args(&["my-project"]),
        every_name_is_a_directory
    ));
}

#[test]
fn opencode_reads_its_directory_after_the_separator_and_boolean_values() {
    let caller_args = |args: &[&str]| args.iter().map(|arg| (*arg).to_owned()).collect::<Vec<_>>();
    let every_name_is_a_directory = |_: &str| true;
    let no_directories = |_: &str| false;

    assert!(opencode_opens_interface(
        &caller_args(&["--", "service"]),
        every_name_is_a_directory
    ));
    assert!(opencode_opens_interface(
        &caller_args(&["--auto", "true"]),
        no_directories
    ));
    assert!(opencode_opens_interface(
        &caller_args(&["--continue", "false", "my-project"]),
        every_name_is_a_directory
    ));
}
