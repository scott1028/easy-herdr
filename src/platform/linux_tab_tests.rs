use super::*;

#[test]
fn terminal_tabs_follow_host_and_override() {
    let spec = TerminalWindowSpec {
        argv: vec!["/tool path".into(), "switch".into(), "codex".into()],
        cwd: "/work dir".into(),
    };
    for (key, value, prefix) in [
        (
            "KONSOLE_VERSION",
            "1",
            "konsole|--new-tab|--workdir|/work dir|-e",
        ),
        (
            "KITTY_WINDOW_ID",
            "1",
            "kitty|@|launch|--type=tab|--cwd|/work dir",
        ),
        ("WEZTERM_PANE", "1", "wezterm|cli|spawn|--cwd|/work dir|--"),
        (
            "GNOME_TERMINAL_SCREEN",
            "1",
            "gnome-terminal|--tab|--working-directory=/work dir|--",
        ),
    ] {
        assert_eq!(
            terminal_tab_command(&spec, |name| {
                if name == "KITTY_LISTEN_ON" && key == "KITTY_WINDOW_ID" {
                    Some("unix:/tmp/kitty.sock".into())
                } else {
                    (name == key).then(|| value.into())
                }
            })
            .map(|argv| argv.join("|")),
            Some(format!("{prefix}|/tool path|switch|codex"))
        );
    }
    for (key, value) in [
        ("TERM_PROGRAM", "ghostty"),
        ("ALACRITTY_WINDOW_ID", "1"),
        ("TERM", "foot"),
        ("TERM_PROGRAM", "unknown"),
    ] {
        assert_eq!(
            terminal_tab_command(&spec, |name| (name == key).then(|| value.into())),
            None
        );
    }
    assert_eq!(terminal_tab_command(&spec, |_| None), None);
    assert_eq!(
        terminal_tab_command(&spec, |key| match key {
            TERMINAL_OVERRIDE_ENV => Some("custom --new".into()),
            "KONSOLE_VERSION" => Some("1".into()),
            _ => None,
        }),
        None
    );
    assert_eq!(
        terminal_tab_command(&spec, |key| match key {
            "KONSOLE_VERSION" | "KITTY_WINDOW_ID" => Some("1".into()),
            _ => None,
        })
        .unwrap()[0],
        "konsole"
    );
    assert_eq!(
        terminal_tab_command(&spec, |key| match key {
            "TERM_PROGRAM" => Some("ghostty".into()),
            "KONSOLE_VERSION" => Some("1".into()),
            _ => None,
        }),
        None
    );
}

#[test]
fn kitty_tabs_require_a_listen_socket() {
    let spec = TerminalWindowSpec {
        argv: vec!["herdr".into(), "switch".into(), "codex".into()],
        cwd: "/work dir".into(),
    };
    for socket in [None, Some("unix:/tmp/kitty.sock")] {
        assert_eq!(
            terminal_tab_command(&spec, |key| match key {
                "KITTY_WINDOW_ID" => Some("1".into()),
                "KITTY_LISTEN_ON" => socket.map(str::to_owned),
                _ => None,
            }),
            socket.map(|_| vec![
                "kitty",
                "@",
                "launch",
                "--type=tab",
                "--cwd",
                "/work dir",
                "herdr",
                "switch",
                "codex",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect())
        );
    }
}

#[test]
fn blank_terminal_override_preserves_host_tabs() {
    let spec = TerminalWindowSpec {
        argv: vec!["herdr".into(), "switch".into()],
        cwd: "/work dir".into(),
    };
    let expected =
        terminal_tab_command(&spec, |key| (key == "KONSOLE_VERSION").then(|| "1".into()));
    for prefix in ["", " \t\n "] {
        assert_eq!(
            terminal_tab_command(&spec, |key| match key {
                TERMINAL_OVERRIDE_ENV => Some(prefix.into()),
                "KONSOLE_VERSION" => Some("1".into()),
                _ => None,
            }),
            expected
        );
    }
}
