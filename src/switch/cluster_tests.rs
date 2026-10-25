use super::*;

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| (*arg).to_owned()).collect()
}

const CLAUDE_YOLO_FLAGS: [&str; 4] = [
    "--permission-mode",
    "bypassPermissions",
    "--settings",
    r#"{"askUserQuestionTimeout":"60s","skipDangerousModePermissionPrompt":true}"#,
];
const CODEX_YOLO_FLAGS: [&str; 2] = ["--ask-for-approval", "never"];

#[test]
fn cluster_parses_multiple_agents_without_shells() {
    assert_eq!(
        parse_cluster_args(&strings(&["claude", "codex", "opencode-local"]), |_| true),
        Ok(ClusterCommand::Launch {
            agents: strings(&["claude", "codex", "opencode-local"]),
            prompt: None,
            options: ClusterOptions::default(),
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
                options: ClusterOptions::default(),
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

#[test]
fn cluster_accepts_auto_exit_options_before_the_first_agent() {
    const MINUTE: u64 = 60;
    const HOUR: u64 = 60 * MINUTE;
    for (flags, exit_when_done, background, timeout, idle_exit) in [
        (vec![], false, false, 2 * HOUR, 5 * MINUTE),
        (vec!["--exit-when-done"], true, false, 2 * HOUR, 5 * MINUTE),
        (
            vec!["--exit-when-done", "--timeout", "3h", "--idle-exit", "10m"],
            true,
            false,
            3 * HOUR,
            10 * MINUTE,
        ),
        (vec!["--background"], true, true, 2 * HOUR, 5 * MINUTE),
        (
            vec!["--background", "--exit-when-done"],
            true,
            true,
            2 * HOUR,
            5 * MINUTE,
        ),
        (
            vec!["--background", "--timeout", "4h"],
            true,
            true,
            4 * HOUR,
            5 * MINUTE,
        ),
        (
            vec![
                "--background",
                "--exit-when-done",
                "--timeout",
                "4h",
                "--idle-exit",
                "10m",
            ],
            true,
            true,
            4 * HOUR,
            10 * MINUTE,
        ),
        (
            vec!["--idle-exit", "30s", "--exit-when-done"],
            true,
            false,
            2 * HOUR,
            30,
        ),
    ] {
        let args = strings(&[flags.as_slice(), &["claude", "codex", "--", "fix", "it"]].concat());
        assert_eq!(
            parse_cluster_args(&args, |agent| matches!(agent, "claude" | "codex")),
            Ok(ClusterCommand::Launch {
                agents: strings(&["claude", "codex"]),
                prompt: Some("fix it".into()),
                options: ClusterOptions {
                    exit_when_done,
                    background,
                    timeout: Some(Duration::from_secs(timeout)),
                    idle_exit: Duration::from_secs(idle_exit),
                    ..ClusterOptions::default()
                },
            }),
            "{args:?}"
        );
    }
    for (args, prompt) in [
        (vec!["--exit-when-done", "claude"], None),
        (
            vec!["--background", "claude", "fix the bug"],
            Some("fix the bug"),
        ),
    ] {
        let Ok(ClusterCommand::Launch {
            agents,
            prompt: parsed_prompt,
            options,
        }) = parse_cluster_args(&strings(&args), |agent| agent == "claude")
        else {
            panic!("{args:?} must launch");
        };
        assert_eq!(agents, strings(&["claude"]));
        assert_eq!(parsed_prompt.as_deref(), prompt);
        assert!(options.exit_when_done);
    }
}

#[test]
fn cluster_rejects_invalid_auto_exit_options_before_launch() {
    for (args, error) in [
        (
            vec!["--timeout", "1h", "claude", "--", "hi"],
            "--timeout requires --exit-when-done or --background",
        ),
        (
            vec!["--timeout", "-1", "claude", "--", "hi"],
            "--timeout requires --exit-when-done or --background",
        ),
        (
            vec!["--exit-when-done", "--idle-exit", "-1", "claude"],
            "invalid duration: -1 (use 30s, 90m or 2h)",
        ),
        (
            vec!["--idle-exit", "10m", "claude", "--", "hi"],
            "--idle-exit requires --exit-when-done or --background",
        ),
        (
            vec!["--background", "claude", "codex"],
            "--background requires a prompt",
        ),
        (
            vec!["--background", "--exit-when-done", "claude"],
            "--background requires a prompt",
        ),
        (
            vec!["--timeout", "1h", "--timeout", "2h", "claude", "--", "hi"],
            "--timeout given twice",
        ),
        (
            vec![
                "--exit-when-done",
                "--idle-exit",
                "1m",
                "--idle-exit",
                "2m",
                "claude",
            ],
            "--idle-exit given twice",
        ),
        (
            vec!["--exit-when-done", "--exit-when-done", "claude"],
            "--exit-when-done given twice",
        ),
        (
            vec!["--background", "--background", "claude", "--", "hi"],
            "--background given twice",
        ),
        (
            vec!["--exit-when-done", "--timeout"],
            "missing <DURATION> after --timeout",
        ),
        (
            vec!["--exit-when-done", "--idle-exit"],
            "missing <DURATION> after --idle-exit",
        ),
        (
            vec!["--exit-when-done", "--timeout", "0s", "claude"],
            "invalid duration: 0s (use 30s, 90m, 2h or -1)",
        ),
        (
            vec![
                "--exit-when-done",
                "--timeout",
                "-1",
                "--timeout",
                "1h",
                "claude",
            ],
            "--timeout given twice",
        ),
        (
            vec![
                "--exit-when-done",
                "--timeout",
                "1h",
                "--timeout",
                "-1",
                "claude",
            ],
            "--timeout given twice",
        ),
        (
            vec![
                "--exit-when-done",
                "--timeout",
                "-1",
                "--timeout",
                "-1",
                "claude",
            ],
            "--timeout given twice",
        ),
        (
            vec!["--exit-when-done", "--idle-exit", "10", "claude"],
            "invalid duration: 10 (use 30s, 90m or 2h)",
        ),
        (vec!["-x", "claude"], "unknown option: -x"),
        (
            vec!["--exit-when-done", "--verbose", "claude"],
            "unknown option: --verbose",
        ),
        (
            vec!["--exit-when-done", "claude", "-v"],
            "unknown option: -v",
        ),
        (
            vec!["--exit-when-done", "claude", "--timeout", "1h"],
            "unknown option: --timeout",
        ),
        (vec!["--exit-when-done"], "missing <AGENT_CLI>"),
        (vec!["--exit-when-done", "--", "hi"], "missing <AGENT_CLI>"),
    ] {
        assert_eq!(
            parse_cluster_args(&strings(&args), |agent| matches!(agent, "claude" | "codex")),
            Err(error.into()),
            "{args:?}"
        );
    }
}

#[test]
fn cluster_timeout_accepts_minus_one_as_no_deadline() {
    assert_eq!(parse_timeout("-1"), Ok(None));
    assert_eq!(parse_timeout("2h"), Ok(Some(Duration::from_secs(7200))));
    let args = strings(&["--exit-when-done", "--timeout", "-1", "claude", "--", "hi"]);
    assert_eq!(
        parse_cluster_args(&args, |agent| agent == "claude"),
        Ok(ClusterCommand::Launch {
            agents: strings(&["claude"]),
            prompt: Some("hi".into()),
            options: ClusterOptions {
                exit_when_done: true,
                background: false,
                timeout: None,
                idle_exit: DEFAULT_IDLE_EXIT,
                ..ClusterOptions::default()
            },
        }),
        "{args:?}"
    );
}

#[test]
fn cluster_durations_parse_a_positive_count_with_one_unit() {
    for (text, seconds) in [
        ("1s", 1),
        ("45s", 45),
        ("30s", 30),
        ("5m", 300),
        ("90m", 5400),
        ("2h", 7200),
        ("5124095576030431h", 5124095576030431 * 3600),
    ] {
        assert_eq!(parse_duration(text), Ok(Duration::from_secs(seconds)));
    }
    for text in [
        "",
        "s",
        "0s",
        "0m",
        "0h",
        "30",
        "30d",
        "30ms",
        "1.5h",
        "1h30m",
        "+5m",
        "-5m",
        " 5m",
        "5m ",
        "5 m",
        "\u{ff15}m",
        "5\u{79d2}",
        "18446744073709551616s",
        "5124095576030432h",
    ] {
        assert_eq!(
            parse_duration(text),
            Err(format!("invalid duration: {text} (use 30s, 90m or 2h)"))
        );
    }
}

#[test]
fn cluster_durations_format_with_the_largest_exact_unit() {
    for (seconds, text) in [
        (7200, "2h"),
        (3600, "1h"),
        (300, "5m"),
        (5400, "90m"),
        (60, "1m"),
        (45, "45s"),
        (61, "61s"),
        (0, "0s"),
    ] {
        assert_eq!(format_duration(Duration::from_secs(seconds)), text);
    }
}

#[test]
fn cluster_options_help_box_lines_share_one_width_within_80_columns() {
    let width = OPTIONS_HELP.lines().next().unwrap().chars().count();
    for help in [OPTIONS_HELP, YOLO_HELP] {
        let lines = help.lines().collect::<Vec<_>>();
        assert!(lines.len() > 2 && width <= 80, "{lines:?}");
        for (index, line) in lines.iter().enumerate() {
            assert_eq!(line.chars().count(), width, "{line:?}");
            let edge = if index == 0 || index + 1 == lines.len() {
                '+'
            } else {
                '|'
            };
            assert!(line.starts_with(edge) && line.ends_with(edge), "{line:?}");
        }
    }
    for flag in [
        "--exit-when-done",
        "--timeout <D>",
        "--timeout -1",
        "--idle-exit <D>",
        "--background",
    ] {
        assert!(OPTIONS_HELP.contains(flag), "missing {flag}");
    }
    for phrase in [
        "--yolo ",
        "--yolo-timeout <D>",
        "Default: 60s",
        "bypassPermissions",
        "askUserQuestionTimeout",
        "--ask-for-approval never",
        "Only the first agent gets --yolo",
        "folder-trust dialog is not answered",
    ] {
        assert!(YOLO_HELP.contains(phrase), "missing {phrase}");
    }
    assert!(!OPTIONS_HELP.contains("--yolo"));
    assert!(!YOLO_HELP.contains("--cluster-yolo"));
}

#[test]
fn cluster_yolo_is_off_by_default_and_parses_its_timeout() {
    let agents = strings(&["claude-personal", "codex-personal"]);
    let is_available = |agent: &str| matches!(agent, "claude-personal" | "codex-personal");
    for (flags, yolo, yolo_timeout) in [
        (vec![], false, 60),
        (vec!["--exit-when-done"], false, 60),
        (vec!["--yolo"], true, 60),
        (vec!["--yolo", "--yolo-timeout", "30s"], true, 30),
        (vec!["--yolo-timeout", "5m", "--yolo"], true, 300),
        (vec!["--yolo", "--yolo-timeout", "2h"], true, 7200),
        (vec!["--yolo", "--yolo-timeout", "0"], true, 0),
        (vec!["--background", "--yolo", "--timeout", "4h"], true, 60),
    ] {
        let args = strings(
            &[
                flags.as_slice(),
                &["claude-personal", "codex-personal", "--", "fix"],
            ]
            .concat(),
        );
        let Ok(ClusterCommand::Launch {
            agents: parsed_agents,
            prompt,
            options,
        }) = parse_cluster_args(&args, is_available)
        else {
            panic!("{args:?} must launch");
        };
        assert_eq!(parsed_agents, agents, "{args:?}");
        assert_eq!(prompt.as_deref(), Some("fix"), "{args:?}");
        assert_eq!(options.yolo, yolo, "{args:?}");
        assert_eq!(
            options.yolo_timeout,
            Duration::from_secs(yolo_timeout),
            "{args:?}"
        );
    }
}

#[test]
fn cluster_rejects_invalid_yolo_options_with_exit_two() {
    for (args, error) in [
        (
            vec!["--yolo-timeout", "5s", "claude", "--", "hi"],
            "--yolo-timeout requires --yolo",
        ),
        (
            vec!["--exit-when-done", "--yolo-timeout", "0", "claude"],
            "--yolo-timeout requires --yolo",
        ),
        (vec!["--yolo", "--yolo", "claude"], "--yolo given twice"),
        (
            vec!["--yolo", "--yolo", "5s", "claude"],
            "--yolo given twice",
        ),
        (
            vec![
                "--yolo",
                "--yolo-timeout",
                "1m",
                "--yolo-timeout",
                "2m",
                "claude",
            ],
            "--yolo-timeout given twice",
        ),
        (
            vec!["--yolo", "--yolo-timeout"],
            "missing <DURATION> after --yolo-timeout",
        ),
        (vec!["claude", "--yolo"], "unknown option: --yolo"),
        (
            vec!["--yolo", "claude", "--yolo-timeout", "5s"],
            "unknown option: --yolo-timeout",
        ),
    ] {
        let args = strings(&args);
        assert_eq!(
            parse_cluster_args(&args, |agent| agent == "claude"),
            Err(error.into()),
            "{args:?}"
        );
        assert_eq!(run(&args), 2, "{args:?}");
    }
    for text in ["0s", "0m", "00", "-0", "-1", "5", "1.5m", "", " 0"] {
        let args = strings(&["--yolo", "--yolo-timeout", text, "claude"]);
        assert_eq!(
            parse_cluster_args(&args, |agent| agent == "claude"),
            Err(format!("invalid duration: {text} (use 0, 30s, 90m or 2h)")),
            "{args:?}"
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
fn cluster_tabs_run_switch_with_the_leader_group() {
    for (ready_file, expected) in [
        (
            None,
            strings(&[
                "/bin/easy-herdr",
                "switch",
                "--cluster-group",
                "12345",
                "codex",
            ]),
        ),
        (
            Some("/d/cluster-12345/1.ready"),
            strings(&[
                "/bin/easy-herdr",
                "switch",
                "--cluster-group",
                "12345",
                "--cluster-ready",
                "/d/cluster-12345/1.ready",
                "codex",
            ]),
        ),
    ] {
        let argv = get_tab_argv("/bin/easy-herdr", 12345, ready_file, false, "codex");
        assert_eq!(argv, expected);
        assert_eq!(
            crate::switch::parse_switch_args(&argv[2..]),
            Ok(crate::switch::SwitchCommand::Launch {
                argv: strings(&["codex"]),
                agent_name: None,
                member: Some(crate::switch::ClusterMember {
                    group: 12345,
                    ready_file: ready_file.map(std::path::PathBuf::from),
                }),
            })
        );
    }
}

#[test]
fn cluster_yolo_adds_family_flags_to_the_foreground_leader_only_when_on() {
    let claude: &[&str] = &CLAUDE_YOLO_FLAGS;
    let codex: &[&str] = &CODEX_YOLO_FLAGS;
    let none: &[&str] = &[];
    let on = ClusterOptions {
        yolo: true,
        ..ClusterOptions::default()
    };
    let off = ClusterOptions::default();
    for (agent, flags) in [
        ("claude", claude),
        ("claude-personal", claude),
        ("claude_x", claude),
        ("claude-", claude),
        ("codex", codex),
        ("codex-personal", codex),
        ("codex_x", codex),
        ("opencode", none),
        ("kimi", none),
        ("claudette", none),
        ("xclaude", none),
        ("codexx", none),
        ("Claude", none),
        ("claude.x", none),
    ] {
        assert_eq!(get_yolo_args(agent), strings(flags), "{agent}");
        assert_eq!(
            get_leader_argv(agent, &on),
            strings(&[&[agent][..], flags].concat()),
            "{agent}"
        );
        assert_eq!(get_leader_argv(agent, &off), strings(&[agent]), "{agent}");
    }
    let settings: serde_json::Value = serde_json::from_str(CLAUDE_YOLO_FLAGS[3]).unwrap();
    assert_eq!(
        settings,
        serde_json::json!({"askUserQuestionTimeout": "60s", "skipDangerousModePermissionPrompt": true})
    );
}

#[test]
fn cluster_background_passes_cluster_yolo_to_the_first_agent_only() {
    let claude_leader = [&["claude-personal"][..], &CLAUDE_YOLO_FLAGS].concat();
    let codex_leader = [&["codex-personal"][..], &CODEX_YOLO_FLAGS].concat();
    for (agents, yolo, launched) in [
        (
            ["claude-personal", "codex-personal", "claude"],
            false,
            [
                vec!["claude-personal"],
                vec!["codex-personal"],
                vec!["claude"],
            ],
        ),
        (
            ["claude-personal", "codex-personal", "claude"],
            true,
            [claude_leader, vec!["codex-personal"], vec!["claude"]],
        ),
        (
            ["codex-personal", "claude-personal", "codex"],
            false,
            [
                vec!["codex-personal"],
                vec!["claude-personal"],
                vec!["codex"],
            ],
        ),
        (
            ["codex-personal", "claude-personal", "codex"],
            true,
            [codex_leader, vec!["claude-personal"], vec!["codex"]],
        ),
        (
            ["opencode", "claude", "codex"],
            true,
            [vec!["opencode"], vec!["claude"], vec!["codex"]],
        ),
    ] {
        let options = ClusterOptions {
            exit_when_done: true,
            background: true,
            yolo,
            ..ClusterOptions::default()
        };
        for (index, agent) in agents.into_iter().enumerate() {
            let argv = cluster_background::get_member_argv(
                "/bin/easy-herdr",
                12345,
                std::path::Path::new("/d/cluster-12345"),
                index,
                agent,
                &options,
            );
            let ready_file = format!("/d/cluster-12345/{index}.ready");
            let mut expected = vec![
                "/bin/easy-herdr",
                "switch",
                "--cluster-group",
                "12345",
                "--cluster-ready",
                &ready_file,
            ];
            if yolo && index == 0 {
                expected.push("--cluster-yolo");
            }
            expected.push(agent);
            assert_eq!(argv, strings(&expected), "{agents:?} yolo={yolo}");
            assert_eq!(
                crate::switch::parse_switch_args(&argv[2..]),
                Ok(crate::switch::SwitchCommand::Launch {
                    argv: strings(&launched[index]),
                    agent_name: None,
                    member: Some(crate::switch::ClusterMember {
                        group: 12345,
                        ready_file: Some(std::path::PathBuf::from(&ready_file)),
                    }),
                }),
                "{agents:?} yolo={yolo} index={index}"
            );
        }
    }
}

#[test]
fn cluster_leader_starts_the_yolo_watch_only_with_yolo() {
    let stop = Arc::new(AtomicBool::new(false));
    let leader = |yolo| {
        super::super::ClusterLaunch::Leader(LeaderConfig {
            group: 12345,
            prompt: None,
            yolo,
            watch: None,
        })
    };
    // Without a prompt, a supervisor or --yolo, the leader starts nothing.
    assert!(get_started_callback(&leader(None), &stop).is_none());
    for grace in [Duration::ZERO, DEFAULT_YOLO_TIMEOUT] {
        assert!(get_started_callback(&leader(Some(grace)), &stop).is_some());
    }
}

#[test]
fn cluster_end_messages_report_the_reason_and_exit_code() {
    let options = ClusterOptions {
        exit_when_done: true,
        timeout: Some(Duration::from_secs(7200)),
        idle_exit: Duration::from_secs(300),
        ..ClusterOptions::default()
    };
    for (reason, message, code) in [
        (
            cluster_watch::EndReason::Done,
            "easy-herdr cluster 12345: all agents idle for 5m; closed 4 agent panes",
            0,
        ),
        (
            cluster_watch::EndReason::Timeout,
            "easy-herdr cluster 12345: timed out after 2h; closed 4 agent panes",
            124,
        ),
        (
            cluster_watch::EndReason::AgentsExited,
            "easy-herdr cluster 12345: all agents exited",
            0,
        ),
        (
            cluster_watch::EndReason::ServerGone,
            "easy-herdr cluster 12345: lost the herdr server",
            1,
        ),
        (
            cluster_watch::EndReason::Stopped,
            "easy-herdr cluster 12345: stopped; closed 4 agent panes",
            143,
        ),
    ] {
        let outcome = cluster_watch::Outcome {
            reason,
            closed: 4,
            members: Vec::new(),
            timeline: Default::default(),
            snapshots: Vec::new(),
        };
        assert_eq!(
            get_end_message(12345, &options, &outcome),
            (message.to_owned(), code)
        );
    }
}

#[test]
fn cluster_supervisor_gets_the_options_and_agents_but_not_the_prompt_in_argv() {
    let options = ClusterOptions {
        exit_when_done: true,
        background: true,
        timeout: Some(Duration::from_secs(4 * 60 * 60)),
        idle_exit: DEFAULT_IDLE_EXIT,
        ..ClusterOptions::default()
    };
    let agents = strings(&["claude", "codex"]);
    let argv = cluster_background::get_supervisor_argv(&agents, &options);
    assert_eq!(
        argv,
        strings(&[
            "cluster",
            "--supervise",
            "--background",
            "--timeout",
            "4h",
            "--idle-exit",
            "5m",
            "claude",
            "codex",
        ])
    );
    assert_eq!(
        parse_cluster_args(&argv[1..], |agent| matches!(agent, "claude" | "codex")),
        Ok(ClusterCommand::Supervise {
            agents: agents.clone(),
            options
        })
    );
    let never = ClusterOptions {
        exit_when_done: true,
        background: true,
        timeout: None,
        idle_exit: DEFAULT_IDLE_EXIT,
        ..ClusterOptions::default()
    };
    let argv = cluster_background::get_supervisor_argv(&agents, &never);
    assert_eq!(
        argv,
        strings(&[
            "cluster",
            "--supervise",
            "--background",
            "--timeout",
            "-1",
            "--idle-exit",
            "5m",
            "claude",
            "codex",
        ])
    );
    assert_eq!(
        parse_cluster_args(&argv[1..], |agent| matches!(agent, "claude" | "codex")),
        Ok(ClusterCommand::Supervise {
            agents,
            options: never
        })
    );
    for (args, error) in [
        (
            vec!["--supervise", "claude"],
            "--supervise requires --background",
        ),
        (
            vec!["--supervise", "--background", "claude", "--", "fix"],
            "--supervise reads the prompt from stdin",
        ),
        (
            vec!["--supervise", "--background", "claude", "fix it"],
            "--supervise reads the prompt from stdin",
        ),
    ] {
        assert_eq!(
            parse_cluster_args(&strings(&args), |_| true),
            Err(error.into()),
            "{args:?}"
        );
    }
    assert!(!OPTIONS_HELP.contains("--supervise"));
}

#[test]
fn cluster_supervisor_argv_passes_yolo_only_when_on() {
    let agents = strings(&["claude-personal", "codex-personal"]);
    for (yolo, yolo_timeout, flags) in [
        (false, 60, vec![]),
        (true, 60, vec!["--yolo", "--yolo-timeout", "1m"]),
        (true, 30, vec!["--yolo", "--yolo-timeout", "30s"]),
        (true, 0, vec!["--yolo", "--yolo-timeout", "0"]),
    ] {
        let options = ClusterOptions {
            exit_when_done: true,
            background: true,
            yolo,
            yolo_timeout: Duration::from_secs(yolo_timeout),
            ..ClusterOptions::default()
        };
        let argv = cluster_background::get_supervisor_argv(&agents, &options);
        let expected = [
            &[
                "cluster",
                "--supervise",
                "--background",
                "--timeout",
                "2h",
                "--idle-exit",
                "5m",
            ][..],
            &flags,
            &["claude-personal", "codex-personal"],
        ]
        .concat();
        assert_eq!(argv, strings(&expected));
        assert_eq!(
            parse_cluster_args(&argv[1..], |agent| {
                matches!(agent, "claude-personal" | "codex-personal")
            }),
            Ok(ClusterCommand::Supervise {
                agents: agents.clone(),
                options
            }),
            "{argv:?}"
        );
    }
}

#[test]
fn cluster_background_starts_every_member_or_stops_the_started_ones() {
    let mut stopped = Vec::new();
    let result = cluster_background::spawn_all(
        4,
        |index| {
            if index == 2 {
                Err(format!("no PTY for agent {index}"))
            } else {
                Ok(index)
            }
        },
        |members| stopped.extend(members),
    );
    assert_eq!(result, Err("no PTY for agent 2".into()));
    assert_eq!(stopped, [0, 1]);
    let result = cluster_background::spawn_all(3, Ok, |_: Vec<usize>| panic!("nothing failed"));
    assert_eq!(result, Ok(vec![0, 1, 2]));
}

#[test]
fn cluster_background_launch_prints_how_to_watch_and_stop() {
    assert_eq!(
        cluster_background::get_launch_message(
            12345,
            "sw-repo-1a2b3c4d5e6f",
            Some(Duration::from_secs(7200))
        ),
        "\
easy-herdr cluster 12345 started in the background
watch: easy-herdr --session sw-repo-1a2b3c4d5e6f agent list
stop:  kill 12345
log:   ./easy-herdr-cluster-*-12345.log is written only if it times out
"
    );
    assert_eq!(
        cluster_background::get_launch_message(12345, "sw-repo-1a2b3c4d5e6f", None),
        "\
easy-herdr cluster 12345 started in the background
watch: easy-herdr --session sw-repo-1a2b3c4d5e6f agent list
stop:  kill 12345
log:   none: this cluster never times out
"
    );
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
