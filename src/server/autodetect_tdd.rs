use super::*;

#[test]
fn ephemeral_server_daemon_command_passes_server_args() {
    let command = build_server_daemon_command(
        PathBuf::from("/tmp/herdr-test"),
        &[crate::server::headless::EPHEMERAL_SERVER_ARG],
    );

    let args: Vec<_> = command.get_args().collect();

    assert_eq!(args, ["server", "--ephemeral"]);
}

#[cfg(unix)]
#[test]
fn ephemeral_server_runs_the_real_binary_behind_a_switch_symlink() {
    let dir = std::env::temp_dir().join(format!("herdr-switch-exe-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let real = dir.join("herdr");
    std::fs::write(&real, b"").unwrap();
    let link = dir.join("herdr-switch");
    let _ = std::fs::remove_file(&link);
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let expected = real.canonicalize().unwrap();

    let exe = ephemeral_server_exe(&link).unwrap();
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(exe, expected);
}
