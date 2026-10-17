use std::fs;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

#[test]
fn config_load_copies_only_legacy_config_once() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "easy-herdr-config-migration-{}-{unique}",
        std::process::id()
    ));
    let legacy = root.join(if cfg!(debug_assertions) {
        "herdr-dev"
    } else {
        "herdr"
    });
    let current = root.join(if cfg!(debug_assertions) {
        "easy-herdr-dev"
    } else {
        "easy-herdr"
    });
    fs::create_dir_all(legacy.join("sessions")).unwrap();
    fs::create_dir_all(legacy.join("plugins")).unwrap();
    let original = "onboarding = false\n";
    fs::write(legacy.join("config.toml"), original).unwrap();
    fs::write(legacy.join("sessions/session.json"), "{}").unwrap();
    fs::write(legacy.join("plugins/plugin.json"), "{}").unwrap();
    let run = |override_path: Option<&std::path::Path>| {
        let mut command = Command::new(env!("CARGO_BIN_EXE_easy-herdr"));
        command
            .args(["config", "check"])
            .env("XDG_CONFIG_HOME", &root)
            .env_remove("HERDR_CONFIG_PATH")
            .env_remove("HERDR_SOCKET_PATH")
            .env_remove("HERDR_CLIENT_SOCKET_PATH")
            .env_remove("HERDR_SESSION");
        if let Some(path) = override_path {
            command.env("HERDR_CONFIG_PATH", path);
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "{output:?}");
    };

    let override_path = root.join("override.toml");
    fs::write(&override_path, original).unwrap();
    run(Some(&override_path));
    assert!(!current.exists());

    run(None);
    assert!(
        current.join("config.toml").is_file(),
        "legacy config was not copied"
    );
    assert_eq!(
        fs::read_to_string(current.join("config.toml")).unwrap(),
        original
    );
    assert_eq!(fs::read_dir(&current).unwrap().count(), 1);
    assert_eq!(
        fs::read_to_string(legacy.join("config.toml")).unwrap(),
        original
    );
    assert!(legacy.join("sessions/session.json").exists());
    assert!(legacy.join("plugins/plugin.json").exists());

    let edited = "onboarding = true\n";
    fs::write(current.join("config.toml"), edited).unwrap();
    run(None);
    assert_eq!(
        fs::read_to_string(current.join("config.toml")).unwrap(),
        edited
    );
    assert_eq!(
        fs::read_to_string(legacy.join("config.toml")).unwrap(),
        original
    );
    fs::remove_dir_all(root).unwrap();
}
