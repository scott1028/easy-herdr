use std::process::Command;

/// Asserts a section starts with `expected`, one entry per consecutive line.
#[cfg(unix)]
fn assert_section_starts_with(help: &str, expected: &[&str]) {
    let lines: Vec<&str> = help.lines().collect();
    let start = lines
        .iter()
        .position(|line| line.contains(expected[0]))
        .unwrap_or_else(|| panic!("missing {:?}: {help}", expected[0]));
    assert!(
        start == 0 || lines[start - 1].is_empty(),
        "{:?} should start a section: {help}",
        expected[0]
    );
    let block = lines
        .get(start..start + expected.len())
        .unwrap_or_else(|| panic!("section is too short for {expected:?}: {help}"));
    for (line, expected_line) in block.iter().zip(expected) {
        assert!(
            line.contains(expected_line),
            "section should start with {expected:?}: {help}"
        );
    }
}

#[cfg(unix)]
#[test]
fn root_help_lists_easy_herdr_additions_at_section_start() {
    let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();

    assert_section_starts_with(
        &help,
        &[
            "Usage: easy-herdr switch <agent-cli> [agent-name] [-- agent-args...]",
            "       easy-herdr cluster [OPTIONS] <agent-cli>... [prompt]",
            "       easy-herdr <agent-cli> [agent-args...]",
            "       easy-herdr install [--dir DIR]",
            "       easy-herdr uninstall [--dir DIR]",
            "       easy-herdr [options]",
        ],
    );
    assert_section_starts_with(
        &help,
        &[
            "Common commands:",
            "  easy-herdr switch <agent-cli> [agent-name]",
            "  easy-herdr cluster [OPTIONS] <agent-cli>... [prompt]",
            "  easy-herdr <agent-cli> [agent-args...]",
            "  easy-herdr install [--dir DIR]",
            "  easy-herdr uninstall [--dir DIR]",
            "  herdr                            Launch or attach to the persistent session",
        ],
    );
    assert_section_starts_with(
        &help,
        &[
            "Agent shortcuts accept PATH commands",
            "easy-herdr does not update itself",
            "Config: ",
        ],
    );
    assert!(!help.contains("herdr update"), "{help}");
}

#[cfg(windows)]
#[test]
fn root_help_omits_unix_only_fork_commands_on_windows() {
    let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();

    assert!(help.contains("\nUsage: easy-herdr [options]\n"), "{help}");
    for unsupported in [
        "easy-herdr switch",
        "easy-herdr cluster",
        "easy-herdr <agent-cli>",
        "easy-herdr install",
        "easy-herdr uninstall",
        "Agent shortcuts accept",
    ] {
        assert!(!help.contains(unsupported), "{help}");
    }
}
