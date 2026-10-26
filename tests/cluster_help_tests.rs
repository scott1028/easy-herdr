#![cfg(unix)]

use std::process::Command;

#[test]
fn cluster_help_describes_usage_prompt_rules_and_terminal_behavior() {
    let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    assert!(help.contains("easy-herdr cluster [OPTIONS] <agent-cli>... [prompt]"));
    assert!(help.contains("Run several agents in terminal tabs or windows"));

    for flag in ["--help", "-h", "help"] {
        let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
            .args(["cluster", flag])
            .output()
            .unwrap();
        assert!(output.status.success(), "{flag}");
        let help = String::from_utf8(output.stdout).unwrap();
        for phrase in [
            "one or more",
            "-- <prompt words...>",
            "one-word",
            "first agent",
            "idle",
            "tab",
            "window",
            "HERDR_SWITCH_TERMINAL",
            "listen_on",
            "KITTY_LISTEN_ON",
            "whitespace-only",
            "120",
        ] {
            assert!(help.contains(phrase), "{flag}: missing {phrase}");
        }
        assert!(output.stderr.is_empty(), "{flag}");
    }
}

#[test]
fn cluster_help_boxes_auto_exit_and_background_options_with_examples() {
    let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
        .args(["cluster", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for phrase in [
        "easy-herdr cluster [OPTIONS] <AGENT_CLI>... [prompt]",
        "easy-herdr cluster [OPTIONS] <AGENT_CLI>... -- <prompt words...>",
        "╭─ Close on its own: --exit-when-done and --background",
        "--exit-when-done",
        "--timeout <D>",
        "--idle-exit <D>",
        "--background",
        "Examples\n",
    ] {
        assert!(help.contains(phrase), "missing {phrase}");
    }
    assert!(!help.contains("--supervise"));
}

#[test]
fn cluster_help_boxes_yolo_options_after_the_auto_exit_box() {
    let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
        .args(["cluster", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    let lines = help.lines().collect::<Vec<_>>();
    let find = |text: &str| {
        lines
            .iter()
            .position(|line| line.contains(text))
            .unwrap_or_else(|| panic!("missing {text}: {help}"))
    };
    let options_box = find("╭─ Close on its own:");
    let yolo_box = find("╭─ Run without approval stops: --yolo or --yolo-herdr");
    let examples = find("Examples");
    assert!(options_box < yolo_box && yolo_box < examples, "{help}");
    // The yolo box follows the auto exit box after one blank line.
    assert!(lines[yolo_box - 1].is_empty(), "{help}");
    assert!(lines[yolo_box - 2].starts_with("╰───"), "{help}");
    assert!(!lines[options_box..yolo_box]
        .iter()
        .any(|line| line.contains("--yolo")));
    for example in [
        r#"  easy-herdr cluster --yolo claude codex -- "fix the bug""#,
        r#"  easy-herdr cluster --yolo-herdr --yolo-timeout 0 claude codex -- "fix the bug""#,
        "  easy-herdr cluster --background --timeout 4h --yolo --yolo-timeout 30s \\",
    ] {
        assert!(find(example) > examples, "{help}");
    }
    assert!(!help.contains("--cluster-yolo"));
    // The hidden switch flag stays out of every help text.
    for args in [&["--help"][..], &["switch", "--help"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success(), "{args:?}");
        let help = [output.stdout, output.stderr].concat();
        assert!(
            !String::from_utf8(help).unwrap().contains("--cluster-yolo"),
            "{args:?}"
        );
    }
}
