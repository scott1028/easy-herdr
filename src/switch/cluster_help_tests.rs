use super::*;

#[test]
fn help_matches_the_approved_plain_text() {
    let plain = render(false);
    assert_eq!(plain, include_str!("../../tests/fixtures/cluster-help.txt"));
    for hidden in ["--cluster-yolo", "--supervise"] {
        assert!(!plain.contains(hidden));
    }
    for fact in [
        "--timeout <D>",
        "--idle-exit <D>",
        "--background",
        "--yolo-timeout <D>",
        "Default: 60s",
        "bypassPermissions",
        "askUserQuestionTimeout",
        "rounded up",
        "at most 10m",
        "--ask-for-approval never",
        "first agent only",
        "folder-trust dialog is not answered",
        "never presses Esc",
    ] {
        assert!(plain.contains(fact), "missing {fact}");
    }
}

#[test]
fn help_panels_are_closed_and_exactly_eighty_columns() {
    let mut is_panel_open = false;
    let mut panels = 0;
    for line in render(false).lines() {
        assert!(line.width() <= 80, "{line}");
        if line.starts_with('╭') {
            assert!(!is_panel_open);
            is_panel_open = true;
            panels += 1;
        }
        if is_panel_open {
            assert_eq!(line.width(), 80, "{line}");
            assert!(line.starts_with(['╭', '│', '╰']), "{line}");
        }
        if line.starts_with('╰') {
            assert!(is_panel_open);
            is_panel_open = false;
        }
    }
    assert!(!is_panel_open);
    assert_eq!(panels, 5);
}

#[test]
fn help_colors_preserve_all_plain_text_and_padding() {
    let color = render(true);
    assert!(color.contains("\x1b["));
    let ansi = regex::Regex::new(r"\x1b\[[0-9;]*m").unwrap();
    assert_eq!(ansi.replace_all(&color, ""), render(false));
    for styled in [
        "Usage".bold().to_string(),
        "--yolo-herdr".cyan().bold().to_string(),
    ] {
        assert!(color.contains(&styled), "missing {styled:?}");
    }
}

#[test]
fn help_color_requires_a_terminal_and_honors_environment_settings() {
    use std::ffi::OsStr;
    for (terminal, no_color, term, expected) in [
        (true, None, None, true),
        (true, Some(""), Some("xterm"), true),
        (true, Some("1"), Some("xterm"), false),
        (true, None, Some("dumb"), false),
        (false, None, Some("xterm"), false),
    ] {
        assert_eq!(
            get_is_color_enabled(terminal, no_color.map(OsStr::new), term.map(OsStr::new)),
            expected
        );
    }
}
