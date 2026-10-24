use std::process::Command;

#[cfg(unix)]
#[test]
fn appimage_install_links_to_the_image_file_only_when_run_from_its_mount() {
    let root = std::env::temp_dir().join(format!("easy-appimage-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let image = root.join("easy herdr.AppImage");
    std::fs::write(&image, "test image").unwrap();
    let exe = std::path::Path::new(env!("CARGO_BIN_EXE_easy-herdr"));
    // The test binary's directory stands in for the AppImage mount.
    let mount = exe.parent().unwrap();
    let (image_file, exe_file) = (image.canonicalize().unwrap(), exe.canonicalize().unwrap());
    for (index, (appdir, expected)) in [
        (Some(mount), &image_file),
        (Some(root.as_path()), &exe_file),
        (None, &exe_file),
    ]
    .into_iter()
    .enumerate()
    {
        let dir = root.join(format!("bin-{index}"));
        std::fs::create_dir_all(&dir).unwrap();
        let mut command = Command::new(exe);
        command
            .args(["install", "--dir"])
            .arg(&dir)
            .env("APPIMAGE", &image);
        match appdir {
            Some(appdir) => command.env("APPDIR", appdir),
            None => command.env_remove("APPDIR"),
        };
        let output = command.output().unwrap();
        assert!(output.status.success(), "{appdir:?}: {output:?}");
        assert_eq!(
            &std::fs::read_link(dir.join("herdr")).unwrap(),
            expected,
            "{appdir:?}"
        );
    }
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn channel_show_prints_the_configured_channel() {
    let root = std::env::temp_dir().join(format!("easy-channel-show-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let config = root.join("config.toml");
    let outputs = ["stable", "preview"].map(|channel| {
        std::fs::write(&config, format!("[update]\nchannel = \"{channel}\"\n")).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
            .args(["channel", "show"])
            .env("HERDR_CONFIG_PATH", &config)
            .output()
            .unwrap();
        (channel, output)
    });
    std::fs::remove_dir_all(root).unwrap();
    for (channel, output) in outputs {
        assert_eq!(output.status.code(), Some(0), "{channel}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), channel);
        assert!(output.stderr.is_empty(), "{channel}: {output:?}");
    }
}

#[test]
fn update_and_other_channel_forms_print_the_fork_download_location() {
    let notice = "easy-herdr does not update itself. Download new builds from https://github.com/scott1028/herdr/releases";
    for args in [
        vec!["update"],
        vec!["update", "--handoff"],
        vec!["update", "--help"],
        vec!["channel"],
        vec!["channel", "set"],
        vec!["channel", "set", "preview"],
        vec!["channel", "show", "extra"],
        vec!["channel", "help"],
        vec!["channel", "--help"],
        vec!["channel", "-h"],
        vec!["channel", "unknown"],
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
            .args(&args)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1), "{args:?}: {output:?}");
        assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), notice);
    }
}
