//! A session's reads for the control plane (data plane v2, step 11.6):
//! the views the control plane builds for a v1 session from its own
//! tables, served for a v2 session from the session's events here. The
//! Trace view is the first.
//!
//! Decisions:
//! - The trace rows come from the control plane's own fold
//!   (`everruns_core::session_trace`), run over the session's committed
//!   events on each read. A cell keeps no trace index, so an idle session
//!   costs nothing, and the rows are small next to the events. The control
//!   plane builds every Trace response from them, as it does for v1.
//! - Event payloads go out in their public form, as the session stream and
//!   the feed serve them, and only for the range the control plane asks.
//!   `log` pages through the whole log for an export, 1000 events a page.
//! - `storage` is the session's values and secret names, never a secret's
//!   value (`session_storage.rs`).
//! - THREAT[TM-TENANT]: each read is of one of the path org's sessions
//!   (`gateway_dp::owned_session`).

use std::sync::Arc;

use axum::http::StatusCode;
use everruns_contracts::dataplane::{SessionEventRow, SessionTraceRows};
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::Event;
use everruns_core::session_trace::{TraceWorkingSet, trace_event};
use serde_json::Value;

use crate::gateway::Gateway;

type ApiError = (StatusCode, String);

/// The reads `GET /dp/v1/orgs/{org}/sessions/{id}/{read}` serves.
pub const READS: &[&str] = &["trace", "events", "log", "storage", "secrets"];

/// Events one `events` or `log` read returns at most.
const MAX_EVENTS: usize = 1_000;

/// One of [`READS`] on `session`, its query as sent.
pub async fn read(
    gateway: &Arc<Gateway>,
    session: SessionId,
    read: &str,
    query: Option<&str>,
) -> Result<Value, ApiError> {
    let json = |value: Result<Value, serde_json::Error>| {
        value.map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
    };
    match read {
        "trace" => {
            let events = crate::gateway_v1::read_events(gateway, session, 0).await?;
            json(serde_json::to_value(trace_rows(&events)))
        }
        "events" => {
            let range = Range::parse(query)?;
            let after = range.from.saturating_sub(1).max(0);
            let events = crate::gateway_v1::read_events(gateway, session, after).await?;
            json(serde_json::to_value(range.select(&events)))
        }
        "log" => {
            let after = log_after(query)?;
            let events = crate::gateway_v1::read_events(gateway, session, after).await?;
            json(serde_json::to_value(log_page(events)))
        }
        "storage" | "secrets" => {
            let gw = gateway.clone();
            let read = read.to_owned();
            gateway
                .with_cell(session, |cell| {
                    let gw = gw.clone();
                    let read = read.clone();
                    async move {
                        gw.call::<crate::api::Lookup<Value>>(
                            cell,
                            &format!("/cell/sessions/{session}/{read}"),
                            None,
                        )
                        .await
                    }
                })
                .await
        }
        _ => Err((StatusCode::NOT_FOUND, format!("no session read {read}"))),
    }
}

/// `?after=`: the sequence a `log` page starts after.
fn log_after(query: Option<&str>) -> Result<i32, ApiError> {
    url::form_urlencoded::parse(query.unwrap_or_default().as_bytes())
        .find(|(key, _)| key == "after")
        .map_or(Ok(0), |(_, value)| {
            value
                .parse::<i32>()
                .map(|after| after.max(0))
                .map_err(|_| (StatusCode::BAD_REQUEST, "invalid after".to_owned()))
        })
}

/// One page of a session's whole log (an export): its events, without
/// deltas, in their public form; a short page is the last.
fn log_page(events: Vec<Event>) -> Vec<Event> {
    events
        .into_iter()
        .filter(|e| !e.event_type.ends_with(".delta"))
        .take(MAX_EVENTS)
        .map(Event::into_public)
        .collect()
}

/// The trace rows of a session whose committed events, in sequence order,
/// are `events`.
pub fn trace_rows(events: &[Event]) -> SessionTraceRows {
    let mut set = TraceWorkingSet::default();
    let mut children = Vec::new();
    for event in events {
        if let Some(slice) = trace_event(event) {
            set.apply(&slice);
        }
        if event.event_type == "tool.completed"
            && let Some(child) = child_session(event)
        {
            children.push(child);
        }
    }
    let (turn_count, mut turns, mut steps) = set.into_changes();
    turns.sort_by_key(|t| t.turn_no);
    steps.sort_by_key(|s| (s.turn_no, s.step_no));
    SessionTraceRows {
        turn_count,
        turns,
        steps,
        children,
    }
}

/// `(tool call id, child session id)` of a finished call that started a
/// subagent: its result names the child (`delegation_result.rs`). The
/// control plane links a v1 call through its spawn handles instead.
fn child_session(event: &Event) -> Option<(String, String)> {
    let data = serde_json::to_value(&event.data).ok()?;
    let call = data["tool_call"]["id"]
        .as_str()
        .or_else(|| data["tool_call_id"].as_str())?
        .to_owned();
    let child = find_child(&data["result"], 0)
        .or_else(|| find_child(&data["tool_result"], 0))?
        .parse::<SessionId>()
        .ok()?;
    Some((call, child.to_string()))
}

/// The first `child_session_id` in `value`, looking into JSON text too.
fn find_child(value: &Value, depth: usize) -> Option<String> {
    if depth > 6 {
        return None;
    }
    match value {
        Value::Object(map) => map
            .get("child_session_id")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| map.values().find_map(|v| find_child(v, depth + 1))),
        Value::Array(items) => items.iter().find_map(|v| find_child(v, depth + 1)),
        Value::String(text) if text.contains("child_session_id") => {
            serde_json::from_str::<Value>(text)
                .ok()
                .and_then(|v| find_child(&v, depth + 1))
        }
        _ => None,
    }
}

/// `?from=&to=&types=&messages=&last=`: an inclusive sequence range, of
/// some event types (comma separated; all when absent), carrying one of
/// some messages (by message id; any when absent), the newest `last` of
/// them.
#[derive(Debug, PartialEq)]
struct Range {
    from: i32,
    to: i32,
    types: Vec<String>,
    messages: Vec<String>,
    last: Option<usize>,
}

impl Range {
    fn parse(query: Option<&str>) -> Result<Self, ApiError> {
        let mut range = Self {
            from: 1,
            to: i32::MAX,
            types: Vec::new(),
            messages: Vec::new(),
            last: None,
        };
        for (key, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
            let bound = || {
                value
                    .parse::<i32>()
                    .map_err(|_| (StatusCode::BAD_REQUEST, format!("invalid {key}")))
            };
            match key.as_ref() {
                "from" => range.from = bound()?,
                "to" => range.to = bound()?,
                "types" => range.types = value.split(',').map(str::to_owned).collect(),
                "messages" => range.messages = value.split(',').map(str::to_owned).collect(),
                "last" => range.last = Some(usize::try_from(bound()?).unwrap_or(0)),
                _ => {}
            }
        }
        Ok(range)
    }

    fn carries_message(&self, event: &Event) -> bool {
        let id = match &event.data {
            everruns_core::events::EventData::InputMessage(data) => data.message.id.to_string(),
            everruns_core::events::EventData::OutputMessageCompleted(data) => {
                data.message.id.to_string()
            }
            _ => return false,
        };
        self.messages.contains(&id)
    }

    /// The events in range, without deltas, in their public form.
    fn select(&self, events: &[Event]) -> Vec<SessionEventRow> {
        let chosen: Vec<&Event> = events
            .iter()
            .filter(|e| {
                e.sequence
                    .is_some_and(|s| (self.from..=self.to).contains(&s))
            })
            .filter(|e| !e.event_type.ends_with(".delta"))
            .filter(|e| self.types.is_empty() || self.types.contains(&e.event_type))
            .filter(|e| self.messages.is_empty() || self.carries_message(e))
            .collect();
        let skip = self
            .last
            .map_or(0, |last| chosen.len().saturating_sub(last));
        chosen
            .into_iter()
            .skip(skip)
            .take(MAX_EVENTS)
            .filter_map(|e| {
                let public = serde_json::to_value(e.clone().into_public()).ok()?;
                Some(SessionEventRow {
                    id: e.id.to_string(),
                    sequence: e.sequence?,
                    event_type: e.event_type.clone(),
                    ts: e.ts,
                    data: public["data"].clone(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use everruns_contracts::typed_id::{MessageId, TurnId};
    use everruns_core::events::EventContext;
    use serde_json::json;

    use super::*;

    fn turn(session: SessionId, turn: TurnId, kind: &str, sequence: i32) -> Event {
        let data = json!({ "turn_id": turn, "iterations": 1, "duration_ms": 5,
                           "input_message_id": MessageId::new() });
        let data = match kind {
            "turn.completed" => everruns_core::events::EventData::TurnCompleted(
                serde_json::from_value(data).unwrap(),
            ),
            _ => {
                everruns_core::events::EventData::TurnStarted(serde_json::from_value(data).unwrap())
            }
        };
        let mut event = Event::new(session, EventContext::turn(turn, MessageId::new()), data);
        event.sequence = Some(sequence);
        event
    }

    #[test]
    fn a_turn_becomes_a_trace_row() {
        let session = SessionId::new();
        let id = TurnId::new();
        let events = [
            turn(session, id, "turn.started", 1),
            turn(session, id, "turn.completed", 2),
        ];
        let rows = trace_rows(&events);
        assert_eq!(rows.turn_count, 1);
        assert_eq!(rows.turns.len(), 1);
        assert_eq!(rows.turns[0].turn_id, id.uuid());
        assert_eq!(rows.turns[0].status, "completed");
        assert_eq!(rows.turns[0].end_sequence, Some(2));
    }

    #[test]
    fn a_range_selects_by_sequence() {
        let session = SessionId::new();
        let id = TurnId::new();
        let events = [
            turn(session, id, "turn.started", 1),
            turn(session, id, "turn.completed", 2),
        ];
        let range = Range::parse(Some("from=2&to=9")).unwrap();
        let rows = range.select(&events);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].sequence, 2);
        assert_eq!(rows[0].event_type, "turn.completed");
        assert!(Range::parse(Some("from=x")).is_err());
        let newest = Range::parse(Some("types=turn.started,turn.completed&last=1")).unwrap();
        assert_eq!(newest.select(&events)[0].sequence, 2);
        let started = Range::parse(Some("types=turn.started")).unwrap();
        assert_eq!(started.select(&events)[0].sequence, 1);
        let messages = Range::parse(Some("messages=message_x")).unwrap();
        assert!(messages.select(&events).is_empty());
    }

    #[test]
    fn a_log_page_is_bounded_and_public() {
        let session = SessionId::new();
        let id = TurnId::new();
        let events: Vec<Event> = (1..=1_200)
            .map(|seq| turn(session, id, "turn.started", seq))
            .collect();
        let page = log_page(events);
        assert_eq!(page.len(), MAX_EVENTS);
        assert_eq!(page[0].sequence, Some(1));
        assert_eq!(log_after(Some("after=7")).unwrap(), 7);
        assert_eq!(log_after(None).unwrap(), 0);
        assert!(log_after(Some("after=x")).is_err());
    }

    #[test]
    fn a_spawn_result_names_its_child() {
        let child = SessionId::new();
        let text = json!({ "child_session_id": child.to_string(), "status": "done" }).to_string();
        assert_eq!(
            find_child(&json!({ "content": [{ "text": text }] }), 0),
            Some(child.to_string())
        );
        assert_eq!(find_child(&json!({ "nothing": 1 }), 0), None);
    }
}
