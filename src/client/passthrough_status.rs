use std::io::{self, BufRead, BufReader, Write};
use std::sync::{mpsc::SyncSender, Weak};
use std::time::Duration;

use super::RelayInput;
use crate::api::{client::ApiClient, schema::*};
use interprocess::local_socket::traits::Stream as _;

#[derive(Debug, PartialEq, Eq)]
enum StatusLine {
    Started,
    Status(Option<AgentStatus>),
    Ignore,
    Lost,
}

fn parse_line(line: &str, pane_id: &str) -> StatusLine {
    let Ok(line) = serde_json::from_str::<serde_json::Value>(line) else {
        return StatusLine::Lost;
    };
    if line.get("error").is_some() {
        return StatusLine::Lost;
    }
    if line["result"]["type"] == "subscription_started" {
        return StatusLine::Started;
    }
    if line["event"] != "pane.agent_status_changed" || line["data"]["pane_id"] != pane_id {
        return StatusLine::Ignore;
    }
    serde_json::from_value(line["data"]["agent_status"].clone())
        .map(|status| StatusLine::Status(line["data"]["agent"].as_str().map(|_| status)))
        .unwrap_or(StatusLine::Lost)
}

/// Shows finished work as done even when the server already marked the pane seen.
#[derive(Default)]
pub(super) struct CompletionTracker {
    worked: bool,
}

impl CompletionTracker {
    pub(super) fn display(&mut self, status: Option<AgentStatus>) -> Option<AgentStatus> {
        match status {
            Some(AgentStatus::Working | AgentStatus::Blocked | AgentStatus::Done) => {
                self.worked = true;
                status
            }
            Some(AgentStatus::Idle) if self.worked => Some(AgentStatus::Done),
            None => {
                self.worked = false;
                None
            }
            _ => status,
        }
    }
}

/// Leading window-title symbol; Unknown shows none.
pub(super) fn title_icon(status: AgentStatus) -> Option<&'static str> {
    match status {
        AgentStatus::Idle => Some("💤"),
        AgentStatus::Working => Some("⏳"),
        AgentStatus::Done => Some("✅"),
        AgentStatus::Blocked => Some("✋"),
        AgentStatus::Unknown => None,
    }
}

pub(super) fn spawn(pane_id: String, sender: SyncSender<RelayInput>, alive: Weak<()>) {
    std::thread::spawn(move || {
        while alive.strong_count() > 0 {
            if let Err(error) = subscribe(&pane_id, &sender, &alive) {
                tracing::debug!(%error, "switch status subscription disconnected");
            }
            if alive.strong_count() == 0 {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    });
}

fn subscribe(pane_id: &str, sender: &SyncSender<RelayInput>, alive: &Weak<()>) -> io::Result<()> {
    let mut stream = crate::ipc::connect_local_stream(&crate::session::active_api_socket_path())?;
    stream.set_recv_timeout(Some(Duration::from_secs(1)))?;
    stream.set_send_timeout(Some(Duration::from_secs(1)))?;
    let request = Request {
        id: "switch:status".into(),
        method: Method::EventsSubscribe(EventsSubscribeParams {
            subscriptions: vec![Subscription::PaneAgentStatusChanged {
                pane_id: pane_id.into(),
                agent_status: None,
            }],
        }),
    };
    writeln!(stream, "{}", serde_json::to_string(&request)?)?;
    stream.flush()?;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut started = false;
    while alive.strong_count() > 0 {
        match reader.read_line(&mut line) {
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof)),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock
                ) =>
            {
                continue
            }
            Err(error) => return Err(error),
            Ok(_) => {}
        }
        let event = parse_line(&line, pane_id);
        line.clear();
        let status = match event {
            StatusLine::Started if !started => {
                started = true;
                let response = ApiClient::local()
                    .request_value_with_timeout(
                        &Request {
                            id: "switch:status:initial".into(),
                            method: Method::PaneGet(PaneTarget {
                                pane_id: pane_id.into(),
                            }),
                        },
                        Duration::from_secs(1),
                    )
                    .map_err(io::Error::other)?;
                let response: SuccessResponse = serde_json::from_value(response)?;
                let ResponseResult::PaneInfo { pane } = response.result else {
                    return Err(io::Error::other("unexpected pane status response"));
                };
                pane.agent.map(|_| pane.agent_status)
            }
            StatusLine::Status(status) if started => status,
            StatusLine::Lost => return Err(io::Error::other("lost status subscription")),
            _ => continue,
        };
        sender
            .send(RelayInput::Status(status))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "passthrough_status_tests.rs"]
mod tests;
