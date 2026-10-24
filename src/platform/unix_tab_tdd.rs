use super::*;

#[test]
fn tab_command_reports_launch_outcomes_without_owning_stdio() {
    let cwd = std::env::temp_dir();
    // macOS has no /bin/true or /bin/false, so resolve them from PATH.
    assert!(run_tab_command(&["true".into()], &cwd));
    assert!(!run_tab_command(&["false".into()], &cwd));
    assert!(!run_tab_command(&["/missing-terminal-cli".into()], &cwd));
    assert!(run_tab_command(&["/bin/sleep".into(), "4".into()], &cwd));
}
