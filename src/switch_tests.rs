use super::*;

#[cfg(unix)]
#[test]
fn shortcut_names_and_markers_accept_only_supported_commands() {
    for name in ["codex", "agent-personal", "a_b.c+1", ".tool"] {
        assert!(shortcut::get_is_plain_name(name));
    }
    for name in ["", "-tool", "a/b", "a b", "a;b", "$(tool)", "工具"] {
        assert!(!shortcut::get_is_plain_name(name));
    }
    for shell in ["bash", "zsh"] {
        for kind in [
            "file", "command", "function", "alias", "hashed", "builtin", "keyword", "none", "",
        ] {
            let (marker, accepted) = if shell == "bash" {
                (
                    kind.to_owned(),
                    matches!(kind, "file" | "function" | "alias"),
                )
            } else {
                (
                    format!("tool: {kind}"),
                    matches!(kind, "command" | "function" | "alias" | "hashed"),
                )
            };
            let output = format!("noisy rc output\n__EASY_HERDR_TYPE__{marker}\nmore noise\n");
            assert_eq!(
                shortcut::get_is_supported_kind(shell, &output),
                accepted,
                "{shell}: {kind}"
            );
        }
        assert!(!shortcut::get_is_supported_kind(
            shell,
            "function\nnoise __EASY_HERDR_TYPE__alias\n"
        ));
    }
    let args = ["codex", "rev", "--", "--model", "x"].map(str::to_owned);
    assert_eq!(
        parse_switch_args(&args).unwrap(),
        SwitchCommand::Launch {
            argv: vec!["codex".into(), "--model".into(), "x".into()],
            agent_name: Some("rev".into()),
            member: None,
        }
    );
}

#[cfg(unix)]
#[test]
fn shortcut_argv_passes_agent_arguments_verbatim() {
    for (args, expected) in [
        (
            vec!["agent-switch", "opencode", "--url", "U", "--api-key", "K"],
            vec!["agent-switch", "opencode", "--url", "U", "--api-key", "K"],
        ),
        (
            vec!["codex", "--", "--model", "x"],
            vec!["codex", "--model", "x"],
        ),
        (
            vec!["codex", "reviewer", "--", "--model", "x"],
            vec!["codex", "reviewer", "--", "--model", "x"],
        ),
        (
            vec!["opencode", "--session", "s", "--help"],
            vec!["opencode", "--session", "s", "--help"],
        ),
        (vec!["codex"], vec!["codex"]),
    ] {
        let args: Vec<String> = args.into_iter().map(str::to_owned).collect();
        assert_eq!(get_shortcut_argv(&args), expected, "{args:?}");
    }
}

#[cfg(unix)]
#[test]
fn launch_argv_uses_eval_and_preserves_other_shells() {
    let argv = vec!["codex-personal".into(), "--model".into(), "x".into()];
    for shell in ["/bin/bash", "/usr/bin/zsh"] {
        let launch = launch_argv(Some(shell), argv.clone());
        assert_eq!(launch[0], shell);
        assert_eq!(launch[1], "-ic");
        assert_eq!(launch[2], "unset HISTFILE; PATH=\"$HERDR_SWITCH_SHIM_DIR:$PATH\"; eval \"$(printf '%q ' \"$@\")\"");
        assert_eq!(launch[3], "herdr-switch");
        assert_eq!(&launch[4..], argv);
    }
    assert_eq!(launch_argv(Some("/usr/bin/fish"), argv.clone()), argv);
    assert_eq!(launch_argv(None, argv.clone()), argv);
}

#[cfg(unix)]
#[test]
fn launch_script_runs_shell_functions_and_aliases() {
    let home = std::env::temp_dir().join(format!("easy-shell-{}", std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let rc = "fakeagent(){ printf 'FAKE_FN %s\\n' \"$@\"; }\nalias fakealias='echo FAKE_ALIAS'\n";
    std::fs::write(home.join(".bashrc"), rc).unwrap();
    std::fs::write(home.join(".zshrc"), rc).unwrap();
    for shell in ["/bin/bash", "/bin/zsh", "/usr/bin/zsh"] {
        if !Path::new(shell).is_file() {
            continue;
        }
        for (program, expected) in [
            ("fakeagent", "FAKE_FN hello"),
            ("fakealias", "FAKE_ALIAS hello"),
        ] {
            let argv = launch_argv(Some(shell), vec![program.into(), "hello".into()]);
            let output = std::process::Command::new(&argv[0])
                .args(&argv[1..])
                .env("HOME", &home)
                .env("ZDOTDIR", &home)
                .env("HERDR_SWITCH_SHIM_DIR", &home)
                .stdin(std::process::Stdio::null())
                .output()
                .unwrap();
            assert!(output.status.success(), "{shell} {program}: {output:?}");
            assert!(String::from_utf8_lossy(&output.stdout).contains(expected));
        }
    }
    std::fs::remove_dir_all(home).unwrap();
}

#[test]
fn switch_session_labels_keep_the_directory_slug_or_bare_hash() {
    for (session, label) in [
        ("sw-herdr-0123456789ab", "herdr"),
        ("sw-agent-switch-0123456789ab", "agent-switch"),
        ("sw-0123456789ab", "0123456789ab"),
    ] {
        assert_eq!(switch_session_label(session), label);
    }
}

#[cfg(unix)]
#[test]
fn window_sessions_require_a_switch_directory_under_sessions() {
    assert_eq!(
        get_window_session(Path::new("/config/sessions/sw-repo/herdr.sock")),
        Some(("sw-repo".into(), "/config/sessions/sw-repo".into()))
    );
    for socket in [
        "/config/sessions/regular/herdr.sock",
        "/config/sw-repo/herdr.sock",
        "/config/herdr.sock",
    ] {
        assert_eq!(get_window_session(Path::new(socket)), None);
    }
}

#[test]
fn parse_switch_args_rejects_invalid_window_shell_flags() {
    for flags in [
        vec!["--ready-file", "ready"],
        vec!["--session", "sw-test"],
        vec!["--session", "regular", "--ready-file", "ready"],
        vec!["--session", "sw-../bad", "--ready-file", "ready"],
    ] {
        let args: Vec<_> = ["--window-shell"]
            .into_iter()
            .chain(flags)
            .map(str::to_owned)
            .collect();
        assert!(parse_switch_args(&args).is_err(), "{args:?}");
    }
}

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
            member: None,
        })
    );
    assert_eq!(
        parse_switch_args(&strings(&["opencode", "reviewer", "--", "--model", "x"])),
        Ok(SwitchCommand::Launch {
            argv: strings(&["opencode", "--model", "x"]),
            agent_name: Some("reviewer".to_owned()),
            member: None,
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
fn parse_switch_args_reads_a_leading_cluster_group() {
    assert_eq!(
        parse_switch_args(&strings(&["--cluster-group", "12345", "codex"])),
        Ok(SwitchCommand::Launch {
            argv: strings(&["codex"]),
            agent_name: None,
            member: Some(ClusterMember {
                group: 12345,
                ready_file: None,
            }),
        })
    );
    assert_eq!(
        parse_switch_args(&strings(&[
            "--cluster-group",
            "7",
            "opencode",
            "reviewer",
            "--",
            "--model",
            "x",
        ])),
        Ok(SwitchCommand::Launch {
            argv: strings(&["opencode", "--model", "x"]),
            agent_name: Some("reviewer".to_owned()),
            member: Some(ClusterMember {
                group: 7,
                ready_file: None,
            }),
        })
    );
}

#[test]
fn parse_switch_args_rejects_invalid_cluster_groups() {
    for (args, error) in [
        (
            vec!["--cluster-group"],
            "missing <PID> after --cluster-group",
        ),
        (
            vec!["--cluster-group", "0", "codex"],
            "invalid --cluster-group: 0",
        ),
        (
            vec!["--cluster-group", "abc", "codex"],
            "invalid --cluster-group: abc",
        ),
        (
            vec!["--cluster-group", "+5", "codex"],
            "invalid --cluster-group: +5",
        ),
        (
            vec!["--cluster-group", "-5", "codex"],
            "invalid --cluster-group: -5",
        ),
        (
            vec!["--cluster-group", "4294967296", "codex"],
            "invalid --cluster-group: 4294967296",
        ),
        (
            vec!["--cluster-group", "1", "--cluster-group", "2", "codex"],
            "--cluster-group given twice",
        ),
        (vec!["--cluster-group", "1"], "missing <AGENT_CLI>"),
        (
            vec!["codex", "--cluster-group", "1"],
            "pass agent arguments after `--`",
        ),
    ] {
        assert_eq!(
            parse_switch_args(&strings(&args)),
            Err(error.to_owned()),
            "{args:?}"
        );
    }
}

#[test]
fn parse_switch_args_reads_a_ready_file_only_with_a_cluster_group() {
    for args in [
        [
            "--cluster-group",
            "7",
            "--cluster-ready",
            "/d/1.ready",
            "codex",
        ],
        [
            "--cluster-ready",
            "/d/1.ready",
            "--cluster-group",
            "7",
            "codex",
        ],
    ] {
        assert_eq!(
            parse_switch_args(&strings(&args)),
            Ok(SwitchCommand::Launch {
                argv: strings(&["codex"]),
                agent_name: None,
                member: Some(ClusterMember {
                    group: 7,
                    ready_file: Some(PathBuf::from("/d/1.ready")),
                }),
            }),
            "{args:?}"
        );
    }
    for (args, error) in [
        (
            vec!["--cluster-ready", "/d/1.ready", "codex"],
            "--cluster-ready requires --cluster-group",
        ),
        (
            vec!["--cluster-group", "7", "--cluster-ready"],
            "missing <PATH> after --cluster-ready",
        ),
        (
            vec![
                "--cluster-group",
                "7",
                "--cluster-ready",
                "a",
                "--cluster-ready",
                "b",
                "codex",
            ],
            "--cluster-ready given twice",
        ),
        (
            vec!["--cluster-group", "7", "codex", "--cluster-ready", "a"],
            "pass agent arguments after `--`",
        ),
    ] {
        assert_eq!(
            parse_switch_args(&strings(&args)),
            Err(error.to_owned()),
            "{args:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn parse_switch_args_adds_yolo_flags_only_with_cluster_yolo() {
    let claude = [
        "--permission-mode",
        "bypassPermissions",
        "--settings",
        r#"{"askUserQuestionTimeout":"60s","skipDangerousModePermissionPrompt":true}"#,
    ];
    let codex = ["--ask-for-approval", "never"];
    for (args, argv, agent_name) in [
        (
            vec!["--cluster-group", "7", "claude-personal"],
            vec!["claude-personal"],
            None,
        ),
        (
            vec!["--cluster-group", "7", "--cluster-yolo", "claude-personal"],
            [&["claude-personal"][..], &claude].concat(),
            None,
        ),
        (
            vec!["--cluster-yolo", "--cluster-group", "7", "claude"],
            [&["claude"][..], &claude].concat(),
            None,
        ),
        (
            vec!["--cluster-group", "7", "codex-personal"],
            vec!["codex-personal"],
            None,
        ),
        (
            vec![
                "--cluster-group",
                "7",
                "--cluster-ready",
                "/d/0.ready",
                "--cluster-yolo",
                "codex-personal",
            ],
            [&["codex-personal"][..], &codex].concat(),
            None,
        ),
        (
            vec![
                "--cluster-group",
                "7",
                "--cluster-yolo",
                "codex",
                "reviewer",
                "--",
                "--model",
                "x",
            ],
            [&["codex"][..], &codex, &["--model", "x"]].concat(),
            Some("reviewer".to_owned()),
        ),
        (
            vec!["--cluster-group", "7", "--cluster-yolo", "opencode"],
            vec!["opencode"],
            None,
        ),
        (
            vec!["--cluster-group", "7", "--cluster-yolo", "claudette"],
            vec!["claudette"],
            None,
        ),
    ] {
        let Ok(SwitchCommand::Launch {
            argv: parsed_argv,
            agent_name: parsed_agent_name,
            member: Some(member),
        }) = parse_switch_args(&strings(&args))
        else {
            panic!("{args:?} must launch a cluster member");
        };
        assert_eq!(parsed_argv, strings(&argv), "{args:?}");
        assert_eq!(parsed_agent_name, agent_name, "{args:?}");
        assert_eq!(member.group, 7, "{args:?}");
    }
}

#[test]
fn parse_switch_args_rejects_invalid_cluster_yolo() {
    for (args, error) in [
        (
            vec!["--cluster-yolo", "claude"],
            "--cluster-yolo requires --cluster-group",
        ),
        (
            vec!["--cluster-yolo", "--cluster-ready", "/d/0.ready", "claude"],
            "--cluster-ready requires --cluster-group",
        ),
        (
            vec![
                "--cluster-group",
                "7",
                "--cluster-yolo",
                "--cluster-yolo",
                "claude",
            ],
            "--cluster-yolo given twice",
        ),
        (
            vec!["--cluster-group", "7", "--cluster-yolo"],
            "missing <AGENT_CLI>",
        ),
        (
            vec!["--cluster-group", "7", "claude", "--cluster-yolo"],
            "pass agent arguments after `--`",
        ),
    ] {
        assert_eq!(
            parse_switch_args(&strings(&args)),
            Err(error.to_owned()),
            "{args:?}"
        );
    }
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
