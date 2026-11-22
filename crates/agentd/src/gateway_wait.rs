//! `POST /v1/sessions/{id}/messages?wait=true` for a session on a shard: the
//! message is admitted as without `wait`, then the call follows the
//! session's events until the turn that carries the message ends, and
//! answers as the control plane does (`api/messages.rs`): `200` with
//! `completed` or `failed` and the messages after the sent one, or `202`
//! with `timeout` and what exists so far.
//!
//! The turn is found by the message, not by a count taken before it was
//! sent: a shard admits a message into a running turn (it steers it), so the
//! turn that answers is the first one to end after the message's own
//! `input.message`. A cancelled turn ends the wait as `failed`, where the
//! control plane's wait runs on to its deadline.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use everruns_contracts::typed_id::{MessageId, SessionId};
use everruns_core::events::Event;
use serde_json::{Value, json};
use tokio::time::Instant;

use crate::gateway::Gateway;

/// The control plane's budget: two minutes unless asked, ten at most.
const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const MAX_TIMEOUT_MS: u64 = 600_000;
const POLL: Duration = Duration::from_millis(250);

/// `wait` and `timeout_ms` from a send's query; `None` without `wait=true`.
pub(crate) fn requested(raw: Option<&str>) -> Option<Duration> {
    let mut wait = false;
    let mut timeout = None;
    for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        match &*key {
            "wait" => wait = value == "true",
            "timeout_ms" => timeout = value.parse::<u64>().ok(),
            _ => {}
        }
    }
    wait.then(|| {
        Duration::from_millis(
            timeout
                .unwrap_or(DEFAULT_TIMEOUT_MS)
                .clamp(1, MAX_TIMEOUT_MS),
        )
    })
}

/// How the turn that carries the message ended.
#[derive(Debug, PartialEq)]
enum Ended {
    Completed,
    Failed(String),
}

/// What the events read so far say about the message's turn.
#[derive(Default)]
struct Follow {
    /// The message's own `input.message`, once read.
    sent: Option<Value>,
    sent_sequence: Option<i32>,
    /// Messages after it.
    after: Vec<Value>,
    ended: Option<Ended>,
    /// The last sequence read.
    cursor: i32,
}

impl Follow {
    fn read(&mut self, message: MessageId, events: &[Event]) {
        let id = message.to_string();
        for event in events {
            let sequence = event.sequence.unwrap_or(0);
            self.cursor = self.cursor.max(sequence);
            if self.ended.is_some() {
                continue;
            }
            let Some(sent_at) = self.sent_sequence else {
                if event.event_type == "input.message"
                    && let Some(listed) = super::gateway_v1::message_of(event)
                    && listed["id"].as_str() == Some(id.as_str())
                {
                    self.sent = Some(listed);
                    self.sent_sequence = Some(sequence);
                }
                continue;
            };
            if sequence <= sent_at {
                continue;
            }
            if let Some(listed) = super::gateway_v1::message_of(event) {
                self.after.push(listed);
            }
            self.ended = match event.event_type.as_str() {
                everruns_core::TURN_COMPLETED => Some(Ended::Completed),
                everruns_core::TURN_FAILED => Some(Ended::Failed(error(event, "turn failed"))),
                everruns_core::TURN_CANCELLED => {
                    Some(Ended::Failed(error(event, "turn cancelled")))
                }
                _ => None,
            };
        }
    }
}

/// A failed turn's detail, as the control plane reads it.
fn error(event: &Event, otherwise: &str) -> String {
    serde_json::to_value(&event.data)
        .ok()
        .and_then(|data| data["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| otherwise.to_owned())
}

/// Follow `session` until the turn that carries `message` ends or the
/// budget runs out, and answer as the control plane's wait does.
/// `accepted` is the `201` body, kept for a message not read back yet.
pub(crate) async fn answer(
    gateway: &Arc<Gateway>,
    session: SessionId,
    message: MessageId,
    accepted: Value,
    budget: Duration,
) -> Response {
    let deadline = Instant::now() + budget;
    let mut follow = Follow::default();
    loop {
        match super::gateway_v1::read_events(gateway, session, follow.cursor).await {
            Ok(events) => follow.read(message, &events),
            Err(error) => return error.into_response(),
        }
        if follow.ended.is_some() || Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(POLL.min(deadline.saturating_duration_since(Instant::now()))).await;
    }
    let (code, status, error) = match follow.ended {
        Some(Ended::Completed) => (StatusCode::OK, "completed", None),
        // The control plane answers a failed turn with no messages.
        Some(Ended::Failed(error)) => {
            follow.after.clear();
            (StatusCode::OK, "failed", Some(error))
        }
        None => (StatusCode::ACCEPTED, "timeout", None),
    };
    let mut body = json!({
        "status": status,
        "message": follow.sent.unwrap_or(accepted),
        "messages": follow.after,
    });
    if let Some(error) = error {
        body["error"] = json!(error);
    }
    (code, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wait_is_asked_with_a_bounded_budget() {
        assert_eq!(requested(None), None);
        assert_eq!(requested(Some("wait=false")), None);
        assert_eq!(
            requested(Some("wait=true")),
            Some(Duration::from_millis(DEFAULT_TIMEOUT_MS))
        );
        assert_eq!(
            requested(Some("timeout_ms=5&wait=true")),
            Some(Duration::from_millis(5))
        );
        assert_eq!(
            requested(Some("wait=true&timeout_ms=99999999")),
            Some(Duration::from_millis(MAX_TIMEOUT_MS))
        );
    }
}
