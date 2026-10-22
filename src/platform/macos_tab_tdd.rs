use super::*;

#[test]
fn terminal_tabs_follow_host_and_override() {
    let spec = TerminalWindowSpec {
        argv: vec!["/tool path".into(), "switch".into(), "a\"b\\c'".into()],
        cwd: "/work dir".into(),
    };
    for (program, prefix) in [
        ("WezTerm", "wezterm|cli|spawn|--cwd|/work dir|--"),
        ("kitty", "kitty|@|launch|--type=tab|--cwd|/work dir"),
    ] {
        assert_eq!(
            terminal_tab_command(&spec, |key| match key {
                "TERM_PROGRAM" => Some(program.into()),
                "KITTY_LISTEN_ON" if program == "kitty" => Some("unix:/tmp/kitty.sock".into()),
                _ => None,
            })
            .map(|argv| argv.join("|")),
            Some(format!("{prefix}|/tool path|switch|a\"b\\c'"))
        );
    }
    let command = terminal_tab_command(&spec, |key| {
        (key == "TERM_PROGRAM").then(|| "iTerm.app".into())
    })
    .unwrap();
    assert_eq!(&command[..2], ["osascript", "-e"]);
    assert!(command[2].contains("tell current window"));
    assert!(command[2].contains("create tab with default profile"));
    assert!(command[2].contains("write text \"cd '/work dir' && exec '/tool path' switch"));
    assert!(command[2].contains("a\\\"b\\\\c'\\\\''"));
    for program in ["Apple_Terminal", "ghostty", "Alacritty", "unknown"] {
        assert_eq!(
            terminal_tab_command(&spec, |key| (key == "TERM_PROGRAM").then(|| program.into())),
            None
        );
    }
    assert_eq!(terminal_tab_command(&spec, |_| None), None);
    assert_eq!(
        terminal_tab_command(&spec, |key| match key {
            TERMINAL_OVERRIDE_ENV => Some("custom --new".into()),
            "TERM_PROGRAM" => Some("iTerm.app".into()),
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
                "TERM_PROGRAM" => Some("kitty".into()),
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
    let expected = terminal_tab_command(&spec, |key| {
        (key == "TERM_PROGRAM").then(|| "WezTerm".into())
    });
    for prefix in ["", " \t\n "] {
        assert_eq!(
            terminal_tab_command(&spec, |key| match key {
                TERMINAL_OVERRIDE_ENV => Some(prefix.into()),
                "TERM_PROGRAM" => Some("WezTerm".into()),
                _ => None,
            }),
            expected
        );
    }
}
