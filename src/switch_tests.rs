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
