use super::*;

#[test]
fn manifest_checks_run_while_binary_checks_remain_disabled() {
    const { assert!(AppPolicy::PRODUCTION.background_updates) };
    #[cfg(unix)]
    const {
        assert!(AppPolicy::HANDOFF_REPLACEMENT.background_updates)
    };
    let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
    let mut app = App::new(
        &Config::default(),
        AppPolicy::TEST,
        None,
        api_rx,
        crate::api::EventHub::default(),
    );
    assert!(!app.update_version_check_enabled);
    assert!(app.update_manifest_check_enabled);
    app.policy = AppPolicy::PRODUCTION;
    app.next_auto_update_check = Some(Instant::now());
    app.run_auto_update_check();
    assert!(app.next_auto_update_check.is_none());
    if cfg!(debug_assertions) {
        return;
    }
    let _guard = crate::config::test_config_env_lock().lock().unwrap();
    let root = std::env::temp_dir().join(format!("easy-update-policy-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let catalog = root.join("index.toml");
    std::fs::write(&catalog, "schema_version = 1\nagents = []\n").unwrap();
    let overrides = [
        ("XDG_STATE_HOME", root.join("state").into_os_string()),
        (
            "HERDR_AGENT_DETECTION_MANIFEST_CATALOG_URL",
            format!(
                "file:///{}",
                catalog
                    .to_string_lossy()
                    .replace('\\', "/")
                    .trim_start_matches('/')
            )
            .into(),
        ),
    ];
    let previous: Vec<_> = overrides
        .iter()
        .map(|(key, _)| (*key, std::env::var_os(key)))
        .collect();
    for (key, value) in overrides {
        std::env::set_var(key, value);
    }
    app.run_agent_manifest_update_check();
    let deadline = Instant::now() + Duration::from_secs(5);
    let event = loop {
        if let Ok(event) = app.event_rx.try_recv() {
            break Some(event);
        }
        if Instant::now() >= deadline {
            break None;
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    for (key, value) in previous {
        match value {
            Some(value) => std::env::set_var(key, value),
            None => std::env::remove_var(key),
        }
    }
    std::fs::remove_dir_all(root).unwrap();
    assert!(app.next_agent_manifest_update_check.is_some());
    let Some(AppEvent::AgentDetectionManifestsUpdated { status, .. }) = event else {
        panic!("manifest worker did not report a result");
    };
    assert_eq!(status.last_result.as_deref(), Some("checked"));
}
