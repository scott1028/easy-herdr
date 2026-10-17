use std::process::Command;

#[cfg(unix)]
#[test]
fn appimage_install_links_to_the_persistent_image_file() {
    let root = std::env::temp_dir().join(format!("easy-appimage-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let image = root.join("easy herdr.AppImage");
    std::fs::write(&image, "test image").unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_easy-herdr"))
        .args(["install", "--dir"])
        .arg(&root)
        .env("APPIMAGE", &image)
        .output()
        .unwrap();
    let installed = std::fs::read_link(root.join("herdr")).unwrap();
    let expected = image.canonicalize().unwrap();
    std::fs::remove_dir_all(root).unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(installed, expected);
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
