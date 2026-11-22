//! The event tap (data plane v2, step 11.5): a session event its org's
//! policy asks for goes on the feed as `Record::Event`.
//!
//! Decisions:
//! - The control plane's observers, trace exports and reports work from
//!   session events, which on v2 live only in the cell. The feed is the
//!   one channel the control plane already drains exactly once, by
//!   position, across leader changes and restarts, so the events go there
//!   rather than over a second, best-effort stream.
//! - The org's policy names the types (`TenantPolicy::events`), so an org
//!   that uses none of these pays nothing, and one with only observers
//!   copies three types, not the whole stream. Policies live in the
//!   directory cell; each cell's leader reads them with the tenants view
//!   (`tenants.rs`) and proposes them (`Command::Taps`), so a policy change
//!   reaches every cell within a poll.
//! - It taps committed changes, in the state machine, so every replica
//!   decides the same and a turn's events reach the feed once each.
//! - An event's payload over [`MAX_EVENT_BYTES`] is left out and named,
//!   so one large tool output cannot swell replicated state; the envelope
//!   (type, sequence, context) still goes, so a count stays exact.

use std::collections::BTreeMap;

use everruns_contracts::dataplane::Record;
use everruns_core::events::{Event, EventData};
use serde_json::{Value, json};

use super::{CellState, Change, Command, Response};

/// The largest event payload copied onto the feed whole.
pub const MAX_EVENT_BYTES: usize = 512 * 1024;

impl CellState {
    /// Apply one committed command. Deterministic: same state and command,
    /// same result, on every replica.
    pub fn apply(&mut self, command: Command, changes: &mut Vec<Change>) -> Response {
        let start = changes.len();
        let response = self.apply_command(command, changes);
        if !self.orgs.taps.is_empty() {
            self.tap(start, changes);
        }
        response
    }

    pub(super) fn set_taps(&mut self, taps: BTreeMap<String, Vec<String>>) -> Response {
        self.orgs.taps = taps;
        Response::Ok
    }

    fn tap(&mut self, start: usize, changes: &[Change]) {
        let index = self.last_applied.map_or(0, |l| l.index);
        for change in changes.iter().skip(start) {
            let Change::Event(session, event) = change else {
                continue;
            };
            let Some(org) = self.sessions.get(session).and_then(|s| s.tenant.as_deref()) else {
                continue;
            };
            let wanted = self
                .orgs
                .taps
                .get(org)
                .is_some_and(|types| types.iter().any(|t| t == "*" || *t == event.event_type));
            if !wanted {
                continue;
            }
            let record = Record::Event {
                session_id: session.to_string(),
                event: stream_form(event),
            };
            let org = org.to_owned();
            self.feed
                .push(index, &org, event.ts.timestamp_millis(), record);
        }
    }
}

impl crate::node::Node {
    /// The directory's tapped types, `taps`: proposed when this cell holds
    /// others (`tenants.rs` calls it on each poll while leading).
    pub(crate) async fn sync_taps(&self, taps: BTreeMap<String, Vec<String>>) {
        if self.shared.state.read().orgs.taps == taps {
            return;
        }
        if let Err(error) = self.propose(Command::Taps { taps }).await {
            tracing::debug!(%error, "tapped event types not proposed");
        }
    }
}

/// What a provider call cost, from its `llm.generation` event, in micro
/// dollars (0 when the provider reported no cost).
pub(super) fn generation_cost_micros(event: &Event) -> Option<i64> {
    let EventData::LlmGeneration(data) = &event.data else {
        return None;
    };
    let usd = data
        .metadata
        .usage
        .as_ref()
        .and_then(|u| u.effective_cost_usd())
        .unwrap_or(0.0);
    Some((usd * 1_000_000.0).ceil() as i64)
}

/// `event` as the session stream carries it (its public projection), its
/// payload left out when too large.
fn stream_form(event: &Event) -> Value {
    let mut value = serde_json::to_value(event.clone().into_public()).unwrap_or(Value::Null);
    let size = serde_json::to_string(&value["data"]).map_or(0, |s| s.len());
    if size > MAX_EVENT_BYTES {
        value["data"] = json!({ "omitted": "too large for the feed", "bytes": size });
    }
    value
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use std::sync::Arc;

    use everruns_contracts::typed_id::SessionId;
    use everruns_core::events::EventContext;
    use serde_json::json;

    use super::*;
    use crate::state::SessionState;

    /// A `kind` event (`turn.started` or `turn.completed`) whose input is
    /// `input`.
    fn event(session: SessionId, kind: &str, input: &str) -> Event {
        let turn = everruns_contracts::typed_id::TurnId::new();
        let data = json!({ "turn_id": turn, "iterations": 1, "input_content": input,
                           "input_message_id": everruns_contracts::typed_id::MessageId::new() });
        let data = match kind {
            "turn.completed" => EventData::TurnCompleted(serde_json::from_value(data).unwrap()),
            _ => EventData::TurnStarted(serde_json::from_value(data).unwrap()),
        };
        Event::new(session, EventContext::empty(), data)
    }

    #[test]
    fn only_the_types_an_org_asks_for_reach_the_feed() {
        let mut state = CellState::default();
        let (watched, quiet) = (SessionId::new(), SessionId::new());
        for (session, org) in [(watched, "watched"), (quiet, "quiet")] {
            state.sessions.insert(
                session,
                SessionState {
                    tenant: Some(org.into()),
                    ..Default::default()
                },
            );
        }
        state
            .orgs
            .taps
            .insert("watched".into(), vec!["turn.completed".into()]);
        let changes = vec![
            Change::Event(watched, Arc::new(event(watched, "turn.started", ""))),
            Change::Event(watched, Arc::new(event(watched, "turn.completed", ""))),
            Change::Session(watched),
            Change::Event(quiet, Arc::new(event(quiet, "turn.completed", ""))),
        ];
        state.tap(0, &changes);
        let records: Vec<_> = state.feed.records.iter().collect();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].org_id, "watched");
        let Record::Event { session_id, event } = &records[0].record else {
            panic!("{:?}", records[0].record);
        };
        assert_eq!(*session_id, watched.to_string());
        assert_eq!(event["type"], "turn.completed");
    }

    #[test]
    fn a_large_payload_is_named_not_copied() {
        let big = "x".repeat(MAX_EVENT_BYTES + 1);
        let value = stream_form(&event(SessionId::new(), "turn.completed", &big));
        assert_eq!(value["type"], "turn.completed");
        assert_eq!(value["data"]["omitted"], "too large for the feed");
    }
}
