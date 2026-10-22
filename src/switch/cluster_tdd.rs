use super::*;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

#[test]
fn cluster_parses_multiple_agents_without_shells() {
    assert_eq!(
        parse_cluster_args(&strings(&["claude", "codex", "opencode-local"]), |_| true),
        Ok(ClusterCommand::Launch {
            agents: strings(&["claude", "codex", "opencode-local"]),
            prompt: None,
        })
    );
}

#[test]
fn cluster_separates_the_optional_prompt_from_agents() {
    for (args, prompt) in [
        (vec!["claude", "codex", "fix the bug"], "fix the bug"),
        (vec!["claude", "codex", "fix!"], "fix!"),
        (
            vec!["claude", "codex", "--", "hello", "world"],
            "hello world",
        ),
        (vec!["claude", "codex", "--", "hello"], "hello"),
        (vec!["claude", "codex", "--", "--help"], "--help"),
    ] {
        assert_eq!(
            parse_cluster_args(&strings(&args), |agent| matches!(agent, "claude" | "codex")),
            Ok(ClusterCommand::Launch {
                agents: strings(&["claude", "codex"]),
                prompt: Some(prompt.into()),
            })
        );
    }
}

#[test]
fn cluster_rejects_invalid_arguments_before_launch() {
    for (args, error) in [
        (vec![], "missing <AGENT_CLI>"),
        (vec!["--", "hi"], "missing <AGENT_CLI>"),
        (vec!["claude", "--"], "missing <PROMPT> after --"),
        (vec!["claude", "-v"], "unknown option: -v"),
        (vec!["claude", "--help"], "unknown option: --help"),
        (
            vec!["claude", "a b", "codex"],
            "the prompt must be the last argument",
        ),
        (
            vec!["claude", "codx"],
            "unknown agent: codx; put a one-word prompt after --",
        ),
        (vec!["codx", "claude"], "unknown agent: codx"),
        (
            vec!["claude", "bad/name", "--", "hi"],
            "unknown agent: bad/name",
        ),
    ] {
        assert_eq!(
            parse_cluster_args(&strings(&args), |agent| matches!(agent, "claude" | "codex")),
            Err(error.into()),
            "{args:?}"
        );
    }
}

#[test]
fn cluster_help_does_not_check_agent_availability() {
    for flag in ["-h", "--help", "help"] {
        assert_eq!(
            parse_cluster_args(&strings(&[flag]), |_| panic!(
                "help must not check the shell"
            )),
            Ok(ClusterCommand::Help)
        );
    }
}

fn agent_info(status: AgentStatus) -> AgentInfo {
    serde_json::from_value(serde_json::json!({
        "terminal_id": "term_1", "agent": "claude", "agent_status": status,
        "workspace_id": "w1", "tab_id": "w1:t1", "pane_id": "w1:p1",
        "focused": false, "revision": 1,
    }))
    .unwrap()
}

#[test]
fn prompt_requires_a_detected_idle_or_done_agent() {
    for status in [AgentStatus::Idle, AgentStatus::Done] {
        assert!(get_is_prompt_ready(&agent_info(status)));
        let mut undetected = agent_info(status);
        undetected.agent = None;
        assert!(!get_is_prompt_ready(&undetected));
    }
    for status in [
        AgentStatus::Working,
        AgentStatus::Blocked,
        AgentStatus::Unknown,
    ] {
        assert!(!get_is_prompt_ready(&agent_info(status)));
    }
}

#[test]
fn prompt_polls_until_ready_retries_rejection_and_honors_the_deadline() {
    use crate::api::schema::{ErrorBody, ErrorResponse};

    let statuses = [
        AgentStatus::Blocked,
        AgentStatus::Unknown,
        AgentStatus::Working,
        AgentStatus::Idle,
        AgentStatus::Done,
    ];
    let mut polls = 0;
    let mut prompts = 0;
    prompt_when_ready(
        "w1:p1",
        "fix the bug",
        Duration::from_secs(3),
        |method, timeout| {
            assert!(!timeout.is_zero() && timeout <= Duration::from_secs(3));
            match method {
                Method::AgentGet(target) => {
                    assert_eq!(target.target, "w1:p1");
                    let agent = agent_info(statuses[polls]);
                    polls += 1;
                    Ok(ResponseResult::AgentInfo { agent })
                }
                Method::AgentPrompt(params) => {
                    assert_eq!(params.target, "w1:p1");
                    assert_eq!(params.text, "fix the bug");
                    assert_eq!(params.wait, None);
                    prompts += 1;
                    if prompts == 1 {
                        Err(ApiClientError::ErrorResponse(ErrorResponse {
                            id: "test".into(),
                            error: ErrorBody {
                                code: "agent_not_ready".into(),
                                message: "retry".into(),
                            },
                        }))
                    } else {
                        Ok(ResponseResult::AgentPrompted {
                            agent: agent_info(AgentStatus::Working),
                        })
                    }
                }
                _ => panic!("unexpected API method"),
            }
        },
    );
    assert_eq!(polls, 5);
    assert_eq!(prompts, 2);

    let mut expired_calls = 0;
    prompt_when_ready("w1:p1", "hi", Duration::ZERO, |_, _| {
        expired_calls += 1;
        Ok(ResponseResult::AgentInfo {
            agent: agent_info(AgentStatus::Idle),
        })
    });
    assert_eq!(expired_calls, 0);
}

#[test]
fn prompt_does_not_resubmit_after_an_ambiguous_transport_error() {
    let mut prompts = 0;
    prompt_when_ready(
        "w1:p1",
        "hi",
        Duration::from_secs(1),
        |method, _| match method {
            Method::AgentGet(_) => Ok(ResponseResult::AgentInfo {
                agent: agent_info(AgentStatus::Idle),
            }),
            Method::AgentPrompt(_) => {
                prompts += 1;
                if prompts == 1 {
                    Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof).into())
                } else {
                    Ok(ResponseResult::AgentPrompted {
                        agent: agent_info(AgentStatus::Working),
                    })
                }
            }
            _ => panic!("unexpected API method"),
        },
    );
    assert_eq!(prompts, 1);
}

#[test]
fn cluster_executable_path_resolves_herdr_switch_symlinks() {
    let directory = std::env::temp_dir().join(format!(
        "herdr-cluster-exe-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&directory).unwrap();
    let directory = directory.canonicalize().unwrap();
    let binary = directory.join("easy-herdr");
    let link = directory.join("herdr-switch");
    std::fs::write(&binary, b"test binary").unwrap();
    std::os::unix::fs::symlink("easy-herdr", &link).unwrap();
    let executable = get_executable_path(Ok(link));
    std::fs::remove_dir_all(&directory).unwrap();
    assert_eq!(executable, Ok(binary.to_str().unwrap().to_owned()));
}

#[derive(Clone)]
struct LogWriter(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn prompt_poll_failures_warn_once_at_timeout_with_last_error() {
    let output = std::sync::Arc::new(std::sync::Mutex::new(Vec::<u8>::new()));
    let writer = LogWriter(output.clone());
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::DEBUG)
        .with_writer(move || writer.clone())
        .finish();
    let mut polls = 0;
    tracing::subscriber::with_default(subscriber, || {
        prompt_when_ready("w1:p1", "hi", Duration::from_millis(600), |_, _| {
            polls += 1;
            Err(std::io::Error::other(format!("poll error {polls}")).into())
        });
    });
    let logs = String::from_utf8(output.lock().unwrap().clone()).unwrap();
    let warnings = logs
        .lines()
        .filter(|line| line.contains("WARN"))
        .collect::<Vec<_>>();
    assert!(polls >= 2, "{logs}");
    assert_eq!(warnings.len(), 1, "{logs}");
    assert!(warnings[0].contains("timed out waiting to submit cluster prompt"));
    assert!(warnings[0].contains(&format!("poll error {polls}")));
    assert_eq!(
        logs.lines().filter(|line| line.contains("DEBUG")).count(),
        polls
    );
}

#[test]
fn cluster_rejects_blank_prompts_with_exit_two() {
    for args in [
        vec!["claude", "codex", ""],
        vec!["claude", "codex", " \t\n "],
        vec!["claude", "codex", "--", ""],
        vec!["claude", "codex", "--", "", ""],
        vec!["claude", "codex", "--", " ", "\t"],
    ] {
        let args = strings(&args);
        assert_eq!(
            parse_cluster_args(&args, |_| true),
            Err("missing <PROMPT>".into())
        );
        assert_eq!(run(&args), 2);
    }
}
