//! The event tap (data plane v2, step 11.5): when a command commits session
//! events its org's policy asks for, a pointer to them goes on the feed
//! (`Record::Events`), and the control plane reads them from the session's
//! history.
//!
//! Decisions:
//! - The control plane's observers, trace exports and reports work from
//!   session events, which on v2 live only in the shard. The feed is the
//!   one channel the control plane already drains exactly once, by
//!   position, across leader changes and restarts, so the pointers go
//!   there rather than over a second, best-effort stream.
//! - A pointer, not a copy (agentd vs control plane review, 2026-11-25):
//!   copies sat in every replica's feed state until the control plane
//!   drained them, up to half a megabyte each, while the events themselves
//!   are already in the session's history. One pointer per session per
//!   command names the range and the tapped types in it.
//! - Explicit conversation messages always reach the control plane: Room
//!   delivery cannot wait for a policy poll. The org's policy names optional
//!   observer/export types (`TenantPolicy::events`). Policies live in the
//!   directory shard; each shard's leader reads them with the tenants view
//!   (`tenants.rs`) and proposes them (`Command::Taps`), so a policy change
//!   reaches every shard within a poll.
//! - It taps committed changes, in the state machine, so every replica
//!   decides the same and a turn's events reach the feed once each.

use std::collections::{BTreeMap, BTreeSet};

use everruns_contracts::dataplane::Record;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{Event, EventData};

use super::{Change, Command, Response, ShardState};

impl ShardState {
    /// Apply one committed command. Deterministic: same state and command,
    /// same result, on every replica.
    pub fn apply(&mut self, command: Command, changes: &mut Vec<Change>) -> Response {
        let start = changes.len();
        let feed_before = self.feed.records.len();
        let response = self.apply_command(command, changes);
        self.tap(start, changes);
        self.label_feed(feed_before);
        response
    }

    /// Copy their session's labels onto the feed records this command
    /// added. Decision: one pass here rather than at every place a record
    /// is made; the labels never change, so every replica writes the same.
    fn label_feed(&mut self, before: usize) {
        let added = self.feed.records.len().saturating_sub(before);
        if added == 0 {
            return;
        }
        let sessions = &self.sessions;
        for record in self.feed.records.iter_mut().rev().take(added) {
            let labels = record
                .record
                .session_id()
                .and_then(|id| id.parse::<everruns_contracts::typed_id::SessionId>().ok())
                .and_then(|id| sessions.get(&id))
                .map(|s| &s.labels);
            if let Some(labels) = labels
                && !labels.is_empty()
            {
                record.labels = labels.clone();
            }
        }
    }

    pub(super) fn set_taps(&mut self, taps: BTreeMap<String, Vec<String>>) -> Response {
        self.orgs.taps = taps;
        Response::Ok
    }

    fn tap(&mut self, start: usize, changes: &[Change]) {
        let index = self.last_applied.map_or(0, |l| l.index);
        // One pointer per session per command: the range of its tapped
        // events and their types, in the order the sessions first appear.
        let mut tapped: Vec<(SessionId, String, i64, i32, i32, BTreeSet<String>)> = Vec::new();
        for change in changes.iter().skip(start) {
            let Change::Event(session, event) = change else {
                continue;
            };
            let Some(org) = self.sessions.get(session).and_then(|s| s.tenant.as_deref()) else {
                continue;
            };
            let wanted =
                event.event_type == everruns_contracts::runtime::events::CONVERSATION_MESSAGE
                    || self.orgs.taps.get(org).is_some_and(|types| {
                        types.iter().any(|t| t == "*" || *t == event.event_type)
                    });
            if !wanted {
                continue;
            }
            let sequence = event.sequence.unwrap_or(0);
            match tapped.iter_mut().find(|t| t.0 == *session) {
                Some(t) => {
                    t.3 = t.3.min(sequence);
                    t.4 = t.4.max(sequence);
                    t.5.insert(event.event_type.clone());
                }
                None => tapped.push((
                    *session,
                    org.to_owned(),
                    event.ts.timestamp_millis(),
                    sequence,
                    sequence,
                    BTreeSet::from([event.event_type.clone()]),
                )),
            }
        }
        for (session, org, at, from, to, types) in tapped {
            let record = Record::Events {
                session_id: session.to_string(),
                from,
                to,
                types: types.into_iter().collect(),
            };
            self.feed.push(index, &org, at, record);
        }
    }
}

impl crate::node::Node {
    /// The directory's tapped types, `taps`: proposed when this shard holds
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
    fn explicit_messages_reach_the_feed_before_the_first_policy_poll() {
        let mut state = ShardState::default();
        let session = SessionId::new();
        state.sessions.insert(
            session,
            SessionState {
                tenant: Some("room-org".into()),
                ..Default::default()
            },
        );
        let mut event = Event::new(
            session,
            EventContext::empty(),
            everruns_contracts::runtime::events::ConversationMessageData {
                message_id: everruns_contracts::typed_id::MessageId::new(),
                text: "Editorial room is ready.".into(),
                tool_call_id: "send-1".into(),
                files: vec![],
                delivery: None,
            },
        );
        event.sequence = Some(3);
        state.tap(0, &[Change::Event(session, Arc::new(event))]);
        assert_eq!(state.feed.records.len(), 1);
        let record = state.feed.records.front().unwrap();
        assert_eq!(record.org_id, "room-org");
        assert!(
            matches!(&record.record, Record::Events { from: 3, to: 3, types, .. }
            if types == &["conversation.message"])
        );
    }

    #[test]
    fn only_the_types_an_org_asks_for_reach_the_feed() {
        let mut state = ShardState::default();
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
        let Record::Events {
            session_id, types, ..
        } = &records[0].record
        else {
            panic!("{:?}", records[0].record);
        };
        assert_eq!(*session_id, watched.to_string());
        assert_eq!(types, &vec!["turn.completed".to_owned()]);
    }

    /// A command's tapped events of one session make one pointer: their
    /// range and types, not the events.
    #[test]
    fn one_pointer_names_a_commands_range_and_types() {
        let mut state = ShardState::default();
        let session = SessionId::new();
        state.sessions.insert(
            session,
            SessionState {
                tenant: Some("o".into()),
                ..Default::default()
            },
        );
        state.orgs.taps.insert("o".into(), vec!["*".into()]);
        let at = |kind: &str, sequence: i32| {
            let mut e = event(session, kind, &"x".repeat(1 << 20));
            e.sequence = Some(sequence);
            Change::Event(session, Arc::new(e))
        };
        state.tap(0, &[at("turn.started", 7), at("turn.completed", 9)]);
        let records: Vec<_> = state.feed.records.iter().collect();
        assert_eq!(records.len(), 1, "{records:?}");
        let Record::Events {
            from, to, types, ..
        } = &records[0].record
        else {
            panic!("{:?}", records[0].record);
        };
        assert_eq!((*from, *to), (7, 9));
        assert_eq!(
            types,
            &vec!["turn.completed".to_owned(), "turn.started".to_owned()]
        );
        assert!(
            serde_json::to_string(&records[0].record).unwrap().len() < 512,
            "a pointer, not the events"
        );
    }
}
