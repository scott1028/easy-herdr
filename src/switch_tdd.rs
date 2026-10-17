use super::*;

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[test]
fn switch_args_detects_binary_name_and_subcommand() {
    let via_link = strings(&["/home/u/.local/bin/herdr-switch", "codex", "rev"]);
    let via_subcommand = strings(&["herdr", "switch", "codex"]);
    let other = strings(&["herdr", "agent", "list"]);

    assert_eq!(switch_args(&via_link), Some(&via_link[1..]));
    assert_eq!(switch_args(&via_subcommand), Some(&via_subcommand[2..]));
    assert_eq!(switch_args(&other), None);
}

#[test]
fn parse_switch_args_reads_cli_name_and_trailing_args() {
    assert_eq!(
        parse_switch_args(&strings(&["codex"])),
        Ok(SwitchCommand::Launch {
            argv: strings(&["codex"]),
            agent_name: None,
        })
    );
    assert_eq!(
        parse_switch_args(&strings(&["opencode", "reviewer", "--", "--model", "x"])),
        Ok(SwitchCommand::Launch {
            argv: strings(&["opencode", "--model", "x"]),
            agent_name: Some("reviewer".to_owned()),
        })
    );
    assert_eq!(
        parse_switch_args(&strings(&["--help"])),
        Ok(SwitchCommand::Help)
    );
    assert_eq!(
        parse_switch_args(&strings(&["--session-name"])),
        Ok(SwitchCommand::SessionName)
    );
}

#[test]
fn parse_switch_args_rejects_ambiguous_input() {
    assert!(parse_switch_args(&[]).is_err());
    assert!(parse_switch_args(&strings(&["codex", "rev", "extra"])).is_err());
    assert!(parse_switch_args(&strings(&["--model", "codex"])).is_err());
    assert!(parse_switch_args(&strings(&["--", "codex"])).is_err());
}

#[test]
fn session_name_is_stable_per_directory_and_distinct_across_directories() {
    let project_a = switch_session_name(Path::new("/work/project-a"), true);
    let project_b = switch_session_name(Path::new("/work/project-b"), true);

    assert_eq!(
        project_a,
        switch_session_name(Path::new("/work/project-a"), true)
    );
    assert_ne!(project_a, project_b);
    assert!(project_a.starts_with("sw-project-a-"), "{project_a}");
    assert!(crate::session::validate_name(&project_a).is_ok());
}

#[test]
fn session_name_sanitizes_and_shortens_the_directory_slug() {
    let name = switch_session_name(
        Path::new("/work/My Repo (中文) with a very long name"),
        true,
    );
    let without_slug = switch_session_name(Path::new("/"), true);

    assert!(crate::session::validate_name(&name).is_ok(), "{name}");
    assert!(name.starts_with("sw-my-repo-"), "{name}");
    assert!(
        name.len() <= SESSION_PREFIX.len() + MAX_SLUG_LEN + 1 + 12,
        "{name}"
    );
    assert_eq!(without_slug.len(), SESSION_PREFIX.len() + 12);
    assert_eq!(
        switch_session_name(Path::new("/work/x"), false).len(),
        SESSION_PREFIX.len() + 12
    );
}

#[test]
fn agent_env_drops_herdr_variables_from_an_outer_pane() {
    let env = agent_env([
        (OsString::from("PATH"), OsString::from("/bin")),
        (OsString::from("TERM"), OsString::from("xterm-kitty")),
        (OsString::from("HERDR_ENV"), OsString::from("1")),
        (OsString::from("HERDR_PANE_ID"), OsString::from("w1:p1")),
        (
            OsString::from("HERDR_SOCKET_PATH"),
            OsString::from("/tmp/x"),
        ),
        (OsString::from("HERDR_SESSION"), OsString::from("work")),
        (OsString::from("HERDR_AGENT"), OsString::from("codex")),
    ]);

    assert_eq!(
        env,
        [
            ("PATH".to_owned(), "/bin".to_owned()),
            ("TERM".to_owned(), "xterm-kitty".to_owned()),
            ("HERDR_AGENT".to_owned(), "codex".to_owned()),
        ]
    );
}

#[test]
fn launch_retries_when_the_server_check_fails_while_it_exits() {
    let mut checks = 0;
    let mut runs = 0;

    let result = launch_with_retries(
        || {
            checks += 1;
            if checks == 1 {
                Err("a herdr server is listening, but its status API is unavailable".into())
            } else {
                Ok(())
            }
        },
        || {
            runs += 1;
            Ok(5)
        },
    );

    assert_eq!(result, Ok(5));
    assert_eq!((checks, runs), (2, 1));
}

#[test]
fn launch_reports_the_last_error_after_all_attempts_fail() {
    let mut checks = 0;

    let result = launch_with_retries(
        || {
            checks += 1;
            Err(format!("incompatible server {checks}"))
        },
        || Ok(0),
    );

    assert_eq!(result, Err("incompatible server 3".to_owned()));
}

#[cfg(unix)]
#[test]
fn shims_come_first_on_path_once_and_mark_the_launch() {
    let env = vec![
        ("HOME".to_owned(), "/home/u".to_owned()),
        ("PATH".to_owned(), "/shims:/usr/bin".to_owned()),
    ];

    let env = with_shims(env, "/shims");

    let value = |key: &str| {
        env.iter()
            .filter(|(name, _)| name == key)
            .map(|(_, value)| value.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(value("PATH"), ["/shims:/usr/bin"]);
    assert_eq!(value(shim::SHIM_DIR_ENV), ["/shims"]);
    assert_eq!(value("HOME"), ["/home/u"]);
}

#[cfg(unix)]
#[test]
fn shims_alone_make_the_path_when_none_was_set() {
    let env = with_shims(vec![], "/shims");

    assert!(
        env.contains(&("PATH".to_owned(), "/shims".to_owned())),
        "{env:?}"
    );
}
