//! `easy-herdr cluster --yolo`: answers known dialogs that keep the first agent blocked.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::api::client::{parse_response_value, ApiClient, ApiClientError};
use crate::api::schema::{
    AgentReadParams, AgentSendKeysParams, AgentStatus, AgentTarget, Method, PaneTarget, ReadFormat,
    ReadSource, Request, ResponseResult,
};

#[path = "yolo_policy.rs"]
mod yolo_policy;

const POLL_INTERVAL: Duration = Duration::from_secs(2);
/// How long after sending keys the dialog is checked again.
const RECHECK_DELAY: Duration = Duration::from_secs(3);
/// Key presses per blocked episode: the first one and two retries.
const MAX_ATTEMPTS: usize = 3;
/// Consecutive failed polls that mean the server is gone.
const MAX_FAILURES: usize = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum WatchEnd {
    LeaderGone,
    ServerGone,
    Stopped,
}

/// One stretch of polls that all saw the first agent blocked.
struct Episode {
    since: Instant,
    attempts: usize,
    /// The dialog that got keys last: agent kind, rule id and keys.
    dialog: Option<(String, String, &'static [&'static str])>,
    /// Set after an unknown dialog or the last retry: no more keys until the episode ends.
    is_finished: bool,
}

/// Starts the watch of the first agent's pane in its own thread.
pub(super) fn spawn_watch(pane_id: String, grace: Duration, stop: Arc<AtomicBool>) {
    if let Err(err) = crate::thread_spawn::spawn_named("herdr-cluster-yolo", move || {
        tracing::info!(pane = %pane_id, ?grace, "cluster yolo watch started");
        let client = ApiClient::local();
        let end = watch(
            &pane_id,
            grace,
            |method| {
                client
                    .request_value_with_timeout(
                        &Request {
                            id: "cluster:yolo".into(),
                            method,
                        },
                        Duration::from_secs(5),
                    )
                    .and_then(parse_response_value)
                    .map(|response| response.result)
            },
            Instant::now,
            std::thread::sleep,
            || stop.load(Ordering::SeqCst),
        );
        tracing::info!(pane = %pane_id, ?end, "cluster yolo watch ended");
    }) {
        tracing::warn!(%err, "cannot start cluster yolo watch");
    }
}

/// Polls the first agent until its pane or the server is gone, or `is_stopped`.
/// A known dialog still blocking it after `grace` gets its policy keys.
pub(super) fn watch(
    pane_id: &str,
    grace: Duration,
    mut request: impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
    now: impl Fn() -> Instant,
    mut sleep: impl FnMut(Duration),
    is_stopped: impl Fn() -> bool,
) -> WatchEnd {
    let mut episode: Option<Episode> = None;
    let mut failures = 0;
    loop {
        if is_stopped() {
            return WatchEnd::Stopped;
        }
        let mut delay = POLL_INTERVAL;
        match request(Method::AgentGet(AgentTarget {
            target: pane_id.into(),
        })) {
            Ok(ResponseResult::AgentInfo { agent }) => {
                failures = 0;
                if agent.agent_status == AgentStatus::Blocked {
                    let episode = episode.get_or_insert_with(|| Episode {
                        since: now(),
                        attempts: 0,
                        dialog: None,
                        is_finished: false,
                    });
                    if !episode.is_finished
                        && now().saturating_duration_since(episode.since) >= grace
                        && answer(pane_id, episode, &mut request, &is_stopped)
                    {
                        delay = RECHECK_DELAY;
                    }
                } else if let Some(Episode {
                    attempts,
                    dialog: Some((kind, rule, keys)),
                    is_finished: false,
                    ..
                }) = episode.take()
                {
                    tracing::info!(
                        pane = pane_id,
                        kind,
                        rule,
                        ?keys,
                        attempt = attempts,
                        result = "answered",
                        "cluster yolo dialog answered"
                    );
                }
            }
            // A pane whose agent is not detected is not listed as an agent. Its next
            // block is a new dialog, so it starts a new episode.
            Err(ApiClientError::ErrorResponse(response))
                if response.error.code == "agent_not_found" =>
            {
                failures = 0;
                episode = None;
                let pane_id = pane_id.to_owned();
                if matches!(
                    request(Method::PaneGet(PaneTarget { pane_id })),
                    Err(ApiClientError::ErrorResponse(response)) if response.error.code == "pane_not_found"
                ) {
                    return WatchEnd::LeaderGone;
                }
            }
            result => {
                if let Err(error) = result {
                    tracing::debug!(pane = pane_id, %error, "cannot poll the cluster yolo agent");
                }
                failures += 1;
                if failures == MAX_FAILURES {
                    return WatchEnd::ServerGone;
                }
            }
        }
        sleep(delay);
    }
}

/// Sends the policy keys for the dialog on screen; returns whether keys went out.
/// A stop seen before any request ends the attempt, so no keys follow a stop.
fn answer(
    pane_id: &str,
    episode: &mut Episode,
    request: &mut impl FnMut(Method) -> Result<ResponseResult, ApiClientError>,
    is_stopped: &impl Fn() -> bool,
) -> bool {
    if episode.attempts == MAX_ATTEMPTS {
        if let Some((kind, rule, keys)) = &episode.dialog {
            tracing::warn!(
                pane = pane_id,
                kind,
                rule,
                ?keys,
                attempt = episode.attempts,
                result = "gave_up",
                "cluster yolo dialog still open; giving up"
            );
        }
        episode.is_finished = true;
        return false;
    }
    if is_stopped() {
        return false;
    }
    let explain = match request(Method::AgentExplain(AgentTarget {
        target: pane_id.into(),
    })) {
        Ok(ResponseResult::AgentExplain { explain }) => explain,
        result => {
            tracing::debug!(
                pane = pane_id,
                ?result,
                "cannot explain the cluster yolo agent"
            );
            return false;
        }
    };
    let kind = explain["agent"].as_str().unwrap_or("-");
    let rule = explain["matched_rule"]["id"].as_str().unwrap_or("-");
    let Some(answer) = yolo_policy::get_answer(kind, rule) else {
        tracing::info!(
            pane = pane_id,
            kind,
            rule,
            result = "unknown",
            "cluster yolo dialog unknown; not answering"
        );
        episode.is_finished = true;
        return false;
    };
    let keys = answer.keys;
    if let Some(gate) = answer.gate {
        if is_stopped() {
            return false;
        }
        let screen = match request(Method::AgentRead(AgentReadParams {
            target: pane_id.into(),
            source: ReadSource::Detection,
            lines: None,
            format: ReadFormat::Text,
            strip_ansi: true,
        })) {
            Ok(ResponseResult::PaneRead { read }) => read.text,
            result => {
                tracing::debug!(
                    pane = pane_id,
                    ?result,
                    "cannot read the cluster yolo agent"
                );
                return false;
            }
        };
        if !screen.contains(gate) {
            tracing::info!(
                pane = pane_id,
                kind,
                rule,
                gate,
                result = "gate_mismatch",
                "cluster yolo dialog lacks its gate text; not answering"
            );
            episode.is_finished = true;
            return false;
        }
    }
    if is_stopped() {
        return false;
    }
    episode.attempts += 1;
    let attempt = episode.attempts;
    match request(Method::AgentSendKeys(AgentSendKeysParams {
        target: pane_id.into(),
        keys: keys.iter().map(|key| (*key).to_owned()).collect(),
    })) {
        Ok(_) => {
            tracing::info!(
                pane = pane_id,
                kind,
                rule,
                ?keys,
                attempt,
                result = "sent",
                "cluster yolo sent keys to a dialog"
            );
        }
        Err(error) => {
            tracing::warn!(pane = pane_id, kind, rule, ?keys, attempt, result = "send_failed", %error, "cluster yolo cannot send keys");
        }
    }
    episode.dialog = Some((kind.to_owned(), rule.to_owned(), keys));
    true
}

#[cfg(test)]
#[path = "cluster_yolo_tests.rs"]
mod tests;
