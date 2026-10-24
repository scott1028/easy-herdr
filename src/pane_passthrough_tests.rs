use super::*;
use crate::pty::passthrough::{PassthroughSink, PtyPassthrough};

#[derive(Default)]
struct CollectingSink {
    output: Mutex<Vec<u8>>,
    exit: Mutex<Option<i32>>,
}

impl PassthroughSink for Arc<CollectingSink> {
    fn push_output(&self, bytes: &[u8]) -> Option<usize> {
        self.output.lock().unwrap().extend_from_slice(bytes);
        Some(0)
    }

    fn queued_bytes(&self) -> Option<usize> {
        Some(0)
    }

    fn push_exit(&self, code: i32) {
        *self.exit.lock().unwrap() = Some(code);
    }
}

fn spawn_passthrough_sh(
    script: &str,
    launch_env: PaneLaunchEnv,
) -> (PaneRuntime, Arc<CollectingSink>, mpsc::Receiver<AppEvent>) {
    let (events, event_rx) = mpsc::channel(64);
    let sink = Arc::new(CollectingSink::default());
    let passthrough = PtyPassthrough::new(Box::new(sink.clone()));
    let runtime = PaneRuntime::spawn_argv_command(
        PaneId::from_raw(7),
        24,
        80,
        std::env::temp_dir(),
        &["/bin/sh".to_owned(), "-c".to_owned(), script.to_owned()],
        &launch_env.with_passthrough(passthrough),
        AgentDetection::Disabled,
        0,
        crate::terminal_theme::TerminalTheme::default(),
        None,
        events,
        Arc::new(Notify::new()),
        Arc::new(RenderSignal::new()),
    )
    .unwrap();
    (runtime, sink, event_rx)
}

async fn wait_for_exit(sink: &CollectingSink) -> Option<i32> {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if let Some(code) = *sink.exit.lock().unwrap() {
                return code;
            }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .ok()
}

#[cfg(unix)]
#[tokio::test]
async fn passthrough_pane_streams_raw_output_then_exit_code() {
    let (runtime, sink, _events) =
        spawn_passthrough_sh("printf 'hi\\033[?25h'; exit 3", PaneLaunchEnv::default());

    let code = wait_for_exit(&sink).await;

    assert_eq!(code, Some(3));
    assert!(
        sink.output.lock().unwrap().ends_with(b"hi\x1b[?25h"),
        "raw bytes must arrive before exit: {:?}",
        String::from_utf8_lossy(&sink.output.lock().unwrap())
    );
    runtime.shutdown();
}

#[cfg(unix)]
#[tokio::test]
async fn passthrough_pane_uses_client_environment_and_terminal() {
    let launch_env = PaneLaunchEnv::default().with_base_env(vec![
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("TERM".to_owned(), "xterm-kitty".to_owned()),
        ("SWITCH_MARKER".to_owned(), "from-client".to_owned()),
    ]);
    let (runtime, sink, _events) = spawn_passthrough_sh(
        "printf '%s|%s|%s|%s' \"$TERM\" \"$SWITCH_MARKER\" \"$HERDR_ENV\" \"${HOME-unset}\"",
        launch_env,
    );

    assert_eq!(wait_for_exit(&sink).await, Some(0));

    let output = String::from_utf8_lossy(&sink.output.lock().unwrap()).into_owned();
    assert!(
        output.ends_with("xterm-kitty|from-client|1|unset"),
        "unexpected env: {output:?}"
    );
    runtime.shutdown();
}

#[cfg(unix)]
#[tokio::test]
async fn passthrough_pane_leaves_bell_to_host_terminal() {
    let (runtime, sink, mut events) =
        spawn_passthrough_sh("printf '\\a'", PaneLaunchEnv::default());

    assert_eq!(wait_for_exit(&sink).await, Some(0));

    while let Ok(event) = events.try_recv() {
        assert!(
            !matches!(event, AppEvent::TerminalBell { .. }),
            "host terminal rings the bell itself"
        );
    }
    runtime.shutdown();
}

#[test]
fn passthrough_launch_keeps_the_client_ssh_agent() {
    let _lock = crate::integration::integration_env_lock();
    let dir = std::env::temp_dir().join(format!("herdr-switch-ssh-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let api_socket = dir.join("herdr.sock");
    // The session-shared agent link a server makes from its first client.
    let shared_agent = dir.join("herdr.sock.agent");
    let _ = std::fs::remove_file(&shared_agent);
    std::os::unix::fs::symlink(dir.join("first-client.sock"), &shared_agent).unwrap();
    let previous = std::env::var_os(crate::api::SOCKET_PATH_ENV_VAR);
    std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, &api_socket);

    let launch = |launch_env: PaneLaunchEnv| {
        let mut cmd = CommandBuilder::new("true");
        apply_pane_launch_env(&mut cmd, &launch_env);
        cmd.get_env("SSH_AUTH_SOCK").map(std::ffi::OsStr::to_owned)
    };
    let own_agent = launch(PaneLaunchEnv::default().with_base_env(vec![(
        "SSH_AUTH_SOCK".to_owned(),
        "/client-b/agent.sock".to_owned(),
    )]));
    let no_agent = launch(PaneLaunchEnv::default().with_base_env(Vec::new()));
    let regular_pane = launch(PaneLaunchEnv::default());

    match previous {
        Some(value) => std::env::set_var(crate::api::SOCKET_PATH_ENV_VAR, value),
        None => std::env::remove_var(crate::api::SOCKET_PATH_ENV_VAR),
    }
    std::fs::remove_dir_all(&dir).unwrap();

    assert_eq!(own_agent.as_deref(), Some("/client-b/agent.sock".as_ref()));
    assert_eq!(no_agent, None);
    assert_eq!(regular_pane.as_deref(), Some(shared_agent.as_os_str()));
}
