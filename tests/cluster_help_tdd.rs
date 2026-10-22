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
    assert!(help.contains("easy-herdr cluster <agent-cli>... [prompt]"));
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
