use std::time::{Duration, Instant};

use crate::api::client::{parse_response_value, ApiClient, ApiClientError};
use crate::api::schema::{
    AgentInfo, AgentPromptParams, AgentStatus, AgentTarget, Method, Request, ResponseResult,
};

#[derive(Debug, PartialEq, Eq)]
enum ClusterCommand {
    Help,
    Launch {
        agents: Vec<String>,
        prompt: Option<String>,
    },
}

fn parse_cluster_args(
    args: &[String],
    is_available: impl Fn(&str) -> bool,
) -> Result<ClusterCommand, String> {
    if args.len() == 1 && matches!(args[0].as_str(), "-h" | "--help" | "help") {
        return Ok(ClusterCommand::Help);
    }
    let separator = args.iter().position(|arg| arg == "--");
    let (agents, prompt) = if let Some(index) = separator {
        if index + 1 == args.len() {
            return Err("missing <PROMPT> after --".into());
        }
        (&args[..index], Some(args[index + 1..].join(" ")))
    } else if args
        .last()
        .is_some_and(|arg| !super::shortcut::get_is_plain_name(arg) && !arg.starts_with('-'))
    {
        (&args[..args.len() - 1], args.last().cloned())
    } else {
        (args, None)
    };
    if agents.is_empty() {
        return Err("missing <AGENT_CLI>".into());
    }
    if prompt
        .as_ref()
        .is_some_and(|prompt| prompt.trim().is_empty())
    {
        return Err("missing <PROMPT>".into());
    }
    for (index, agent) in agents.iter().enumerate() {
        if agent.starts_with('-') {
            return Err(format!("unknown option: {agent}"));
        }
        if !super::shortcut::get_is_plain_name(agent) {
            if separator.is_none() {
                return Err("the prompt must be the last argument".into());
            }
            return Err(format!("unknown agent: {agent}"));
        }
        if !is_available(agent) {
            let hint = if index + 1 == args.len() {
                "; put a one-word prompt after --"
            } else {
                ""
            };
            return Err(format!("unknown agent: {agent}{hint}"));
        }
    }
    Ok(ClusterCommand::Launch {
        agents: agents.to_vec(),
        prompt,
    })
}

pub(super) fn run(args: &[String]) -> i32 {
    let (agents, prompt) = match parse_cluster_args(args, super::shortcut::get_is_available) {
        Ok(ClusterCommand::Help) => {
            print_help();
            return 0;
        }
        Ok(ClusterCommand::Launch { agents, prompt }) => (agents, prompt),
        Err(err) => {
            eprintln!("herdr-switch: {err}");
            return 2;
        }
    };
    let (cwd, _) = match super::resolve_session() {
        Ok(session) => session,
        Err(err) => return super::report_error(&err),
    };
    let exe = match get_executable_path(std::env::current_exe()) {
        Ok(exe) => exe,
        Err(err) => return super::report_error(&err),
    };
    for agent in &agents[1..] {
        let spec = crate::platform::TerminalWindowSpec {
            argv: vec![exe.clone(), "switch".into(), agent.clone()],
            cwd: cwd.clone(),
        };
        if let Err(err) = crate::platform::open_terminal_tab(&spec) {
            eprintln!("herdr-switch: cannot open a terminal for {agent}: {err}");
        }
    }
    super::launch(vec![agents[0].clone()], None, prompt)
        .unwrap_or_else(|err| super::report_error(&err))
}

fn get_executable_path(exe: std::io::Result<std::path::PathBuf>) -> Result<String, String> {
    let exe = exe
        .and_then(|exe| exe.canonicalize())
        .map_err(|err| format!("cannot resolve the herdr executable: {err}"))?;
    exe.into_os_string()
        .into_string()
        .map_err(|_| "the herdr executable path is not valid UTF-8".into())
}

fn print_help() {
    println!("usage: easy-herdr cluster <AGENT_CLI>... [prompt]");
    println!("       easy-herdr cluster <AGENT_CLI>... -- <prompt words...>");
    println!();
    println!("Run one or more agents in the same canonical directory and switch session.");
    println!("The first agent runs in this terminal; each other agent opens in a new tab");
    println!("when supported (Konsole, kitty, WezTerm, GNOME Terminal, iTerm2), otherwise");
    println!("in a new window. A nonblank HERDR_SWITCH_TERMINAL forces the window command.");
    println!(
        "kitty tabs require remote control with a listen socket (listen_on / KITTY_LISTEN_ON)."
    );
    println!();
    println!("Agent names use ASCII letters, digits, ., _, + and -, and cannot start with -.");
    println!("A final argument outside that set is the prompt unless it starts with -.");
    println!("Plain names are agents; put a one-word prompt after --.");
    println!("Agent options are not accepted. Prompts cannot be empty or whitespace-only.");
    println!("The prompt is typed into the first agent after it is detected idle or done,");
    println!("never passed in argv. Readiness is checked every 250 ms for up to 120 seconds.");
    println!("Answer any startup approval dialogs yourself while cluster waits.");
}

pub(super) fn spawn_prompt(pane_id: String, prompt: String) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-prompt", move || {
        let client = ApiClient::local();
        prompt_when_ready(
            &pane_id,
            &prompt,
            Duration::from_secs(120),
            |method, timeout| {
                client
                    .request_value_with_timeout(
                        &Request {
                            id: "cluster:prompt".into(),
                            method,
                        },
                        timeout,
                    )
                    .and_then(parse_response_value)
                    .map(|response| response.result)
            },
        );
    }) {
        tracing::warn!(%err, "cannot start cluster prompt thread");
    }
}

fn get_is_prompt_ready(agent: &AgentInfo) -> bool {
    agent.agent.is_some() && matches!(agent.agent_status, AgentStatus::Idle | AgentStatus::Done)
}

fn prompt_when_ready(
    pane_id: &str,
    prompt: &str,
    timeout: Duration,
    mut request: impl FnMut(Method, Duration) -> Result<ResponseResult, ApiClientError>,
) {
    let deadline = Instant::now() + timeout;
    let mut last_error = None;
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match request(
            Method::AgentGet(AgentTarget {
                target: pane_id.into(),
            }),
            remaining,
        ) {
            Ok(ResponseResult::AgentInfo { agent }) if get_is_prompt_ready(&agent) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                match request(
                    Method::AgentPrompt(AgentPromptParams {
                        target: pane_id.into(),
                        text: prompt.into(),
                        wait: None,
                    }),
                    remaining,
                ) {
                    Ok(ResponseResult::AgentPrompted { .. }) => return,
                    Ok(_) => tracing::warn!(pane_id, "unexpected cluster prompt response"),
                    Err(err) => {
                        tracing::warn!(pane_id, %err, "cannot submit cluster prompt");
                        // A lost reply may follow a successful submission.
                        if !matches!(err, ApiClientError::ErrorResponse(_)) {
                            return;
                        }
                    }
                }
            }
            Ok(ResponseResult::AgentInfo { .. }) => {}
            Ok(_) => tracing::warn!(pane_id, "unexpected cluster agent response"),
            Err(err) => {
                tracing::debug!(pane_id, %err, "cannot poll cluster agent");
                last_error = Some(err);
            }
        }
        std::thread::sleep(
            Duration::from_millis(250).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
    tracing::warn!(pane_id, error = ?last_error, "timed out waiting to submit cluster prompt");
}

#[cfg(test)]
#[path = "cluster_tdd.rs"]
mod tdd;
