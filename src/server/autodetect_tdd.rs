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

#[test]
fn spawned_processes_use_the_appimage_only_from_inside_its_mount() {
    let root = std::env::temp_dir().join(format!("herdr-appimage-{}", std::process::id()));
    let mount = root.join("mount");
    let inner = mount.join("usr/bin/easy-herdr");
    let outside = root.join("debug/easy-herdr");
    let appimage = root.join("easy-herdr.AppImage");
    for file in [&inner, &outside, &appimage] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"").unwrap();
    }
    let pick = |appimage: &Path, appdir: Option<&Path>, exe: &Path| {
        pick_spawn_exe(
            Some(appimage.into()),
            appdir.map(Into::into),
            exe.to_path_buf(),
        )
    };
    let cases = [
        // Runs from the mount: the persistent image file.
        (pick(&appimage, Some(&mount), &inner), &appimage),
        // A debug build in a pane that inherited both variables.
        (pick(&appimage, Some(&mount), &outside), &outside),
        (pick(&appimage, None, &inner), &inner),
        (pick(&root.join("missing"), Some(&mount), &inner), &inner),
    ];
    let results = cases.map(|(picked, expected)| (picked, expected.clone()));
    std::fs::remove_dir_all(&root).unwrap();
    for (index, (picked, expected)) in results.into_iter().enumerate() {
        assert_eq!(picked, expected, "case {}", index + 1);
    }
}
