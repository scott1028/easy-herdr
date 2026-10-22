use super::*;

#[test]
fn tab_command_reports_launch_outcomes_without_owning_stdio() {
    let cwd = std::env::temp_dir();
    assert!(run_tab_command(&["/bin/true".into()], &cwd));
    assert!(!run_tab_command(&["/bin/false".into()], &cwd));
    assert!(!run_tab_command(&["/missing-terminal-cli".into()], &cwd));
    assert!(run_tab_command(&["/bin/sleep".into(), "4".into()], &cwd));
}
