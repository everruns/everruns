//! The control plane ↔ data plane protocol, `/dp/v1`, as the cells speak
//! it (knowledge/execution/data-plane-v2/cp-dp-protocol.md).
//!
//! Decisions:
//! - The wire types live here, in the data plane's crate, and the fake
//!   control plane in the testkit uses the same ones; the real control
//!   plane takes them over in plan step 7.
//! - Readers ignore what they don't know (serde's default for fields; a
//!   config change of an unknown kind is skipped, not refused), so `/dp/v1`
//!   evolves additively, as the protocol's rules require.
//! - Step 6 implements the channels a turn needs: config (agent
//!   revisions), allowances and the feed. Org admin, secrets, connections
//!   and org tokens come with step 7.
//! - One addition to the approved shapes, additive: an allowance grant may
//!   say the most one provider call may cost (`per_call_micros`), until the
//!   cell prices calls from the model and the prompt itself (step 7).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `PUT /dp/v1/orgs/{org}/config/{epoch}/{version}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigUpdate {
    pub epoch: u64,
    pub version: u64,
    /// 0: a full snapshot; otherwise must equal the applied version.
    pub base_version: u64,
    /// Each change as sent; read with [`ConfigChange::parse`].
    pub changes: Vec<Value>,
}

/// The config changes a cell acts on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ConfigChange {
    /// A new revision of one agent.
    Agent(AgentDefinition),
    /// Running turns keep the revision they started with.
    Removal { of: String, id: String },
}

impl ConfigChange {
    /// `None`: a kind this data plane does not know (or not yet).
    pub fn parse(change: &Value) -> Option<Self> {
        serde_json::from_value(change.clone()).ok()
    }
}

/// A fully resolved agent: everything a turn needs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentDefinition {
    pub agent_id: String,
    pub revision: u64,
    #[serde(default = "one")]
    pub format: u32,
    /// The agent package manifest a session's definition is.
    pub package: Value,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result")]
pub enum ConfigAck {
    Ok { applied_version: u64 },
    AlreadyApplied { applied_version: u64 },
    NeedFullSnapshot { applied_version: u64 },
    StaleEpoch { epoch: u64 },
}

/// `GET /dp/v1/orgs/{org}/config`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigPosition {
    pub epoch: u64,
    pub applied_version: u64,
}

/// `POST /dp/v1/tenants/{org}/allowances`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllowanceRequest {
    pub request_id: String,
    pub cell: String,
    pub incarnation: u64,
    pub want_micros: i64,
    pub settlements: Vec<GrantSettlement>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantSettlement {
    pub grant_id: String,
    /// Cumulative actual cost charged to the grant.
    pub used_micros: i64,
    /// Still reserved by operations in flight.
    pub reserved_micros: i64,
    /// Final: the rest returns to the balance.
    pub closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AllowanceGrant {
    pub grant_id: String,
    /// 0: budget exhausted, or a policy stops spending.
    pub granted_micros: i64,
    pub spend_until_unix_ms: i64,
    #[serde(default)]
    pub reason: Option<String>,
    /// The most one provider call may cost (additive; see the module).
    #[serde(default)]
    pub per_call_micros: Option<i64>,
}

/// `POST /dp/v1/feed`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedBatch {
    pub data_plane: String,
    pub source: String,
    pub incarnation: u64,
    pub records: Vec<FeedRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedRecord {
    /// Raft index of the entry that produced the record.
    pub index: u64,
    /// Its position among that entry's records.
    pub ordinal: u32,
    pub org_id: String,
    pub at_unix_ms: i64,
    #[serde(flatten)]
    pub record: Record,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Record {
    Usage {
        session_id: String,
        turn_id: String,
        /// The provider call attempt: usage's logical identity.
        attempt_id: String,
        #[serde(default)]
        unresolved: bool,
        agent_id: String,
        provider: String,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cost_micros: i64,
        kind: String,
    },
    Session {
        session_id: String,
        /// Monotonic per session, also across moves.
        session_version: u64,
        agent_id: String,
        status: String,
        created_unix_ms: i64,
        last_activity_unix_ms: i64,
        turns: u64,
    },
    ConfigApplied {
        version: u64,
    },
    /// A record of a type this reader does not know.
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeedAck {
    pub source: String,
    pub incarnation: u64,
    /// `(index, ordinal)` of the last record stored.
    pub stored_through: (u64, u32),
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use serde_json::json;

    #[test]
    fn readers_ignore_what_they_do_not_know() {
        let grant: AllowanceGrant = serde_json::from_value(json!({
            "grant_id": "g", "granted_micros": 5, "spend_until_unix_ms": 9,
            "reason": null, "a_new_field": true
        }))
        .unwrap();
        assert_eq!(grant.per_call_micros, None);
        assert_eq!(
            ConfigChange::parse(&json!({ "kind": "Secret", "secret_id": "s" })),
            None,
            "a change of a kind not handled here is skipped"
        );
        let record: FeedRecord = serde_json::from_value(json!({
            "index": 1, "ordinal": 0, "org_id": "o", "at_unix_ms": 0,
            "type": "Health", "sessions": 3
        }))
        .unwrap();
        assert_eq!(record.record, Record::Unknown);
    }

    #[test]
    fn an_agent_change_round_trips() {
        let change = json!({
            "kind": "Agent", "agent_id": "a", "revision": 2, "format": 1,
            "package": { "name": "a", "instructions": "x" }
        });
        let parsed = ConfigChange::parse(&change).unwrap();
        assert_eq!(serde_json::to_value(&parsed).unwrap(), change);
    }
}
