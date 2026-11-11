//! What a cell keeps for its link to the control plane, as replicated state
//! (cp-dp-protocol.md): each org's config (directory cell only) and the
//! feed records not yet acknowledged.
//!
//! Decisions:
//! - Config applies in the directory cell's Raft, so the ordering rules
//!   (in order per org, a new epoch only as a full snapshot, an older epoch
//!   refused) hold across leader changes: a retried `PUT` lands on the
//!   applied version and is answered `AlreadyApplied`.
//! - Agent revisions are kept whole in the directory cell's state for now:
//!   they are small, and a session copies its revision's package when it
//!   is created, so a later revision or a removal never changes a session
//!   that started on the old one. They move to the object store when
//!   packages grow (step 7).
//! - Feed records are made at apply from committed events, numbered by the
//!   Raft index of their entry and their position in it, and stay in state
//!   until the control plane acknowledges them, so a new leader resends
//!   exactly what the old one had not delivered, under the same ids.

use std::collections::{BTreeMap, VecDeque};

use everruns_core::events::{Event, EventData};
use serde::{Deserialize, Serialize};

use crate::dp::{AgentDefinition, ConfigAck, ConfigChange, ConfigUpdate, FeedRecord, Record};
use crate::state::SessionState;

/// Every org's config, in the directory cell.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Orgs {
    pub orgs: BTreeMap<String, OrgConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OrgConfig {
    pub epoch: u64,
    pub applied_version: u64,
    /// The current revision of each agent.
    pub agents: BTreeMap<String, AgentDefinition>,
}

impl Orgs {
    /// Apply `update` to `org` by the protocol's ordering rules.
    pub fn apply(&mut self, org: &str, update: &ConfigUpdate) -> ConfigAck {
        let config = self.orgs.entry(org.to_owned()).or_default();
        let full = update.base_version == 0;
        if update.epoch < config.epoch {
            return ConfigAck::StaleEpoch {
                epoch: config.epoch,
            };
        }
        let applied_version = config.applied_version;
        if update.epoch > config.epoch {
            // A restored control plane: only a full snapshot starts it.
            if !full {
                return ConfigAck::NeedFullSnapshot { applied_version };
            }
        } else if update.version <= applied_version {
            return ConfigAck::AlreadyApplied { applied_version };
        } else if !full && update.base_version != applied_version {
            return ConfigAck::NeedFullSnapshot { applied_version };
        }
        if full {
            config.agents.clear();
        }
        for change in update.changes.iter().filter_map(ConfigChange::parse) {
            match change {
                ConfigChange::Agent(agent) => {
                    let newer = config
                        .agents
                        .get(&agent.agent_id)
                        .is_none_or(|current| agent.revision > current.revision);
                    if newer {
                        config.agents.insert(agent.agent_id.clone(), agent);
                    }
                }
                ConfigChange::Removal { of, id } if of == "agent" => {
                    config.agents.remove(&id);
                }
                ConfigChange::Removal { .. } => {}
            }
        }
        config.epoch = update.epoch;
        config.applied_version = update.version;
        ConfigAck::Ok {
            applied_version: update.version,
        }
    }
}

/// Feed records waiting for the control plane's acknowledgement.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Feed {
    pub records: VecDeque<FeedRecord>,
    /// `(index, ordinal)` of the last record acknowledged.
    #[serde(default)]
    pub acked: (u64, u32),
}

impl Feed {
    pub fn push(&mut self, index: u64, org: &str, at_unix_ms: i64, record: Record) {
        let ordinal = match self.records.back() {
            Some(last) if last.index == index => last.ordinal + 1,
            _ => 0,
        };
        self.records.push_back(FeedRecord {
            index,
            ordinal,
            org_id: org.to_owned(),
            at_unix_ms,
            record,
        });
    }

    /// Drop the records the control plane stored, through `through`.
    pub fn ack(&mut self, through: (u64, u32)) {
        while self
            .records
            .front()
            .is_some_and(|r| (r.index, r.ordinal) <= through)
        {
            self.records.pop_front();
        }
        self.acked = self.acked.max(through);
    }

    /// The records `session`'s committed `event` makes for its tenant.
    pub fn observe(&mut self, index: u64, session: &str, s: &SessionState, event: &Event) {
        let Some(org) = s.tenant.as_deref() else {
            return;
        };
        let at = event.ts.timestamp_millis();
        match &event.data {
            EventData::LlmGeneration(data) => {
                let usage = data.metadata.usage.as_ref();
                let cost = usage
                    .and_then(|u| u.effective_cost_usd())
                    .map(|usd| (usd * 1_000_000.0).ceil() as i64)
                    .unwrap_or(0);
                let record = Record::Usage {
                    session_id: session.to_owned(),
                    turn_id: event
                        .context
                        .turn_id
                        .map(|t| t.to_string())
                        .unwrap_or_default(),
                    // The event's id is stable: a replay of the same entry
                    // makes the same id, so usage is billed once.
                    attempt_id: event.id.to_string(),
                    unresolved: false,
                    agent_id: s.agent.clone(),
                    provider: data.metadata.provider.clone().unwrap_or_default(),
                    model: data.metadata.model.clone(),
                    input_tokens: usage.map_or(0, |u| u64::from(u.input_tokens)),
                    output_tokens: usage.map_or(0, |u| u64::from(u.output_tokens)),
                    cached_tokens: usage
                        .and_then(|u| u.cache_read_tokens)
                        .map_or(0, u64::from),
                    cost_micros: cost,
                    kind: "model".into(),
                };
                self.push(index, org, at, record);
            }
            _ => {}
        }
    }

    /// `session` recorded a turn's outcome: its summary for the tenant.
    /// Decision: the summary follows the outcome record, not the terminal
    /// event, because the turn count moves with the outcome.
    pub fn turn_finished(&mut self, index: u64, session: &str, s: &SessionState) {
        let Some(org) = s.tenant.as_deref() else {
            return;
        };
        let last = s.events.iter().rev().find(|e| {
            matches!(
                e.data,
                EventData::TurnCompleted(_) | EventData::TurnFailed(_) | EventData::TurnCancelled(_)
            )
        });
        let status = match last.map(|e| &e.data) {
            Some(EventData::TurnFailed(_)) => "failed",
            _ => "idle",
        };
        let at = last.map_or(s.created_ms, |e| e.ts.timestamp_millis());
        let record = Record::Session {
            session_id: session.to_owned(),
            session_version: s.last_sequence().unwrap_or(0).max(0) as u64,
            agent_id: s.agent.clone(),
            status: status.into(),
            created_unix_ms: s.created_ms,
            last_activity_unix_ms: at,
            turns: s.turns_completed(),
        };
        self.push(index, org, at, record);
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use serde_json::json;

    fn agent(id: &str, revision: u64) -> serde_json::Value {
        json!({ "kind": "Agent", "agent_id": id, "revision": revision,
                "package": { "name": id, "instructions": format!("r{revision}") } })
    }

    fn update(epoch: u64, version: u64, base: u64, changes: Vec<serde_json::Value>) -> ConfigUpdate {
        ConfigUpdate {
            epoch,
            version,
            base_version: base,
            changes,
        }
    }

    #[test]
    fn config_applies_in_order_and_once() {
        let mut orgs = Orgs::default();
        let ok = |v| ConfigAck::Ok { applied_version: v };
        assert_eq!(orgs.apply("o", &update(1, 1, 0, vec![agent("a", 1)])), ok(1));
        assert_eq!(orgs.apply("o", &update(1, 2, 1, vec![agent("a", 2)])), ok(2));
        assert_eq!(
            orgs.apply("o", &update(1, 2, 1, vec![agent("a", 2)])),
            ConfigAck::AlreadyApplied { applied_version: 2 },
            "a retried PUT"
        );
        assert_eq!(
            orgs.apply("o", &update(1, 4, 3, vec![])),
            ConfigAck::NeedFullSnapshot { applied_version: 2 },
            "a gap"
        );
        assert_eq!(orgs.orgs["o"].agents["a"].revision, 2);
        // Revisions only move forward, whatever order they arrive in.
        assert_eq!(orgs.apply("o", &update(1, 3, 2, vec![agent("a", 1)])), ok(3));
        assert_eq!(orgs.orgs["o"].agents["a"].revision, 2);
    }

    #[test]
    fn a_restored_control_plane_starts_with_a_full_snapshot() {
        let mut orgs = Orgs::default();
        orgs.apply("o", &update(1, 5, 0, vec![agent("a", 3), agent("b", 1)]));
        assert_eq!(
            orgs.apply("o", &update(2, 1, 1, vec![])),
            ConfigAck::NeedFullSnapshot { applied_version: 5 },
            "a new epoch's first update must be a full snapshot"
        );
        assert_eq!(
            orgs.apply("o", &update(2, 1, 0, vec![agent("a", 3)])),
            ConfigAck::Ok { applied_version: 1 }
        );
        assert!(!orgs.orgs["o"].agents.contains_key("b"), "the snapshot is whole");
        assert_eq!(
            orgs.apply("o", &update(1, 6, 5, vec![agent("b", 2)])),
            ConfigAck::StaleEpoch { epoch: 2 },
            "the old control plane is refused"
        );
        assert!(!orgs.orgs["o"].agents.contains_key("b"));
    }

    #[test]
    fn feed_records_number_by_entry_and_leave_once_acknowledged() {
        let mut feed = Feed::default();
        for (index, n) in [(7, 2), (9, 1)] {
            for _ in 0..n {
                feed.push(index, "o", 0, Record::ConfigApplied { version: 1 });
            }
        }
        let ids: Vec<(u64, u32)> = feed.records.iter().map(|r| (r.index, r.ordinal)).collect();
        assert_eq!(ids, vec![(7, 0), (7, 1), (9, 0)]);
        feed.ack((7, 1));
        assert_eq!(feed.records.len(), 1);
        feed.ack((7, 0));
        assert_eq!(feed.records.len(), 1, "an older ack changes nothing");
        assert_eq!(feed.acked, (7, 1));
    }
}
