use std::process::Command;

#[cfg(unix)]
fn assert_section_ends_with(help: &str, heading: &str, fork_lines: &[&str]) {
    let section = help
        .split("\n\n")
        .find(|section| section.starts_with(heading))
        .unwrap_or_else(|| panic!("missing {heading:?} section: {help}"));
    let lines: Vec<&str> = section.lines().collect();
    let start = lines
        .len()
        .checked_sub(fork_lines.len())
        .unwrap_or_else(|| panic!("{heading:?} section is too short: {section}"));
    let tail = &lines[start..];
    for (line, expected) in tail.iter().zip(fork_lines) {
        assert!(
            line.contains(expected),
            "{heading:?} section should end with {fork_lines:?}: {section}"
        );
    }
}

#[cfg(unix)]
#[test]
fn root_help_lists_easy_herdr_additions_at_section_end() {
    let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();

    assert_section_ends_with(
        &help,
        "Usage:",
        &[
            "easy-herdr switch <agent-cli> [agent-name] [-- agent-args...]",
            "easy-herdr cluster <agent-cli>... [prompt]",
            "easy-herdr <agent-cli> [agent-args...]",
            "easy-herdr install [--dir DIR]",
        ],
    );
    assert_section_ends_with(
        &help,
        "Common commands:",
        &[
            "easy-herdr switch <agent-cli> [agent-name]",
            "easy-herdr cluster <agent-cli>... [prompt]",
            "easy-herdr <agent-cli> [agent-args...]",
            "easy-herdr install [--dir DIR]",
        ],
    );
    assert_section_ends_with(
        &help,
        "Config:",
        &[
            "Agent shortcuts accept PATH commands",
            "easy-herdr does not update itself",
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

    for unsupported in [
        "easy-herdr switch",
        "easy-herdr cluster",
        "easy-herdr <agent-cli>",
        "easy-herdr install",
        "Agent shortcuts accept",
    ] {
        assert!(!help.contains(unsupported), "{help}");
    }
}
