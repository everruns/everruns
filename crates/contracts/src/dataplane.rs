//! The control plane ↔ data plane protocol, `/dp/v1`.
//!
//! Both planes speak these types: the server as the control plane, and the
//! cells of a v2 data plane. They live here, in a crate both already depend
//! on, so the control plane never pulls in a data plane's storage.
//!
//! Decisions:
//! - Readers ignore what they don't know: serde's default for fields, a
//!   config change of an unknown kind is skipped rather than refused, and a
//!   feed record of an unknown type reads as [`Record::Unknown`]. `/dp/v1`
//!   evolves additively; a breaking change is `/dp/v2`.
//! - Byte fields (sealed secrets, wrapped keys) are base64 strings, so the
//!   JSON stays readable and the types need no extra serde helpers.
//! - Policy fields the data plane only passes on (limits, egress) stay JSON
//!   values: their shapes are owned by the features that enforce them.
//! - A connection's kind is `connection_kind` on the wire, because a config
//!   change is tagged by `kind` and the two would collide.
//! - One addition to the approved shapes, additive: an allowance grant may
//!   say the most one provider call may cost (`per_call_micros`).

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The path prefix both planes serve. The edge router never exposes it.
pub const PREFIX: &str = "/dp/v1";

// ---------------------------------------------------------------- config

/// `PUT /dp/v1/orgs/{org}/config/{epoch}/{version}`: one tenant change set.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ConfigUpdate {
    /// The control plane's epoch; a restored control plane starts a new one.
    pub epoch: u64,
    /// Monotonic per org within an epoch, assigned by the outbox.
    pub version: u64,
    /// 0: a full snapshot; otherwise must equal the applied version.
    pub base_version: u64,
    /// Each change as sent; read with [`ConfigChange::parse`], so a change
    /// of a kind the reader doesn't know is skipped, not refused.
    pub changes: Vec<Value>,
}

/// One change in a [`ConfigUpdate`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind")]
pub enum ConfigChange {
    /// A new revision of one agent.
    Agent(AgentDefinition),
    /// A new revision of one tenant secret, sealed under the org's data key.
    Secret(SealedSecret),
    /// A connection and the secret holding its token.
    Connection(Connection),
    /// The tenant's policy, whole.
    Policy(TenantPolicy),
    /// `of` is `agent`, `secret` or `connection`. Running turns keep the
    /// revision they started with.
    Removal { of: String, id: String },
}

impl ConfigChange {
    /// `None`: a kind this reader does not know.
    pub fn parse(change: &Value) -> Option<Self> {
        serde_json::from_value(change.clone()).ok()
    }

    /// The change as it goes on the wire.
    pub fn to_value(&self) -> Value {
        // A derived Serialize of plain data cannot fail.
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}

/// A fully resolved agent: everything a turn needs, and no platform id.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentDefinition {
    /// The control plane's opaque agent key.
    pub agent_id: String,
    /// Monotonic per agent.
    pub revision: u64,
    /// The package format version.
    #[serde(default = "one")]
    pub format: u32,
    /// The agent package manifest. Secrets only as [`SealedSecret`] ids.
    pub package: Value,
}

fn one() -> u32 {
    1
}

/// A tenant secret: AES-256-GCM under the org's data key. Cells store the
/// ciphertext only; executors unseal it in memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SealedSecret {
    pub secret_id: String,
    /// Monotonic per secret; a lower revision never overwrites a higher one.
    pub revision: u64,
    /// Base64.
    pub ciphertext: String,
    /// Base64, 12 bytes.
    pub nonce: String,
    pub data_key_id: String,
}

/// Who refreshes a connection's token.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
pub enum RefreshOwner {
    /// The data plane refreshes and sends the new token on the feed.
    DataPlane,
    #[default]
    ControlPlane,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Connection {
    pub connection_id: String,
    /// `mcp_oauth`, `github`, `slack`, ... On the wire `connection_kind`:
    /// a config change's own tag is `kind`.
    #[serde(rename = "connection_kind")]
    pub kind: String,
    #[serde(default)]
    pub settings: Value,
    pub token_secret_id: String,
    #[serde(default)]
    pub refresh_owner: RefreshOwner,
}

/// The connection kind of an org's model provider.
pub const LLM_PROVIDER: &str = "llm_provider";

/// The connection id of the model provider an agent's `model.provider`
/// names (`key`, normalized as provider keys are).
pub fn llm_provider_connection(key: &str) -> String {
    format!(
        "llm:{}",
        crate::runtime_provider::ProviderKey::new(key).as_str()
    )
}

/// An [`LLM_PROVIDER`] connection's settings: what a driver needs besides
/// the API key, which is the connection's token secret.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LlmProviderSettings {
    /// The provider's key: its normalized name.
    pub provider_key: String,
    /// The driver: `openai`, `openrouter`, `anthropic`, ...
    pub provider_type: String,
    #[serde(default)]
    pub base_url: Option<String>,
    /// The chat API, when the provider selects one.
    #[serde(default)]
    pub api: Option<String>,
    #[serde(default)]
    pub request_options: crate::provider::ProviderRequestOptions,
}

/// The org, seen by the data plane only as a tenant.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct TenantPolicy {
    /// Safety nets only: per-cell caps, file quotas.
    #[serde(default)]
    pub limits: Value,
    /// Allowlists, caps, kill switch.
    #[serde(default)]
    pub egress: Value,
    #[serde(default)]
    pub placement: PlacementPolicy,
    /// Enabled feature stages.
    #[serde(default)]
    pub features: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind")]
pub enum PlacementPolicy {
    #[default]
    Shared,
    Dedicated {
        pool: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "result")]
pub enum ConfigAck {
    Ok {
        applied_version: u64,
    },
    AlreadyApplied {
        applied_version: u64,
    },
    /// A gap or a mismatched base: send a full snapshot.
    NeedFullSnapshot {
        applied_version: u64,
    },
    /// From an older control plane epoch: refused.
    StaleEpoch {
        epoch: u64,
    },
}

/// `GET /dp/v1/orgs/{org}/config`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigPosition {
    pub epoch: u64,
    pub applied_version: u64,
}

/// `GET /dp/v1/orgs/{org}/config/unknown?epoch={e}`: after a control plane
/// restore, what the data plane holds that the restored epoch doesn't know.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigDiff {
    /// `(agent_id, revision)`.
    pub agents: Vec<(String, u64)>,
    /// `(secret_id, revision)`.
    pub secrets: Vec<(String, u64)>,
    pub connections: Vec<String>,
}

/// What the control plane says it knows, for [`ConfigDiff`]: the newest
/// revision of each entry in its own epoch.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConfigKnown {
    pub agents: Vec<(String, u64)>,
    pub secrets: Vec<(String, u64)>,
    pub connections: Vec<String>,
}

// ------------------------------------------------------------- org admin

/// `PUT /dp/v1/orgs/{org}`: enable the org on this data plane.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OrgEnable {
    pub command_id: String,
    /// Base64: the org's data key, wrapped with the data plane's key.
    pub wrapped_data_key: String,
    pub data_key_id: String,
    #[serde(default)]
    pub policy: TenantPolicy,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OrgState {
    Active,
    Suspended,
}

/// `POST /dp/v1/orgs/{org}/state`. Takes effect at once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgStateCommand {
    pub command_id: String,
    pub state: OrgState,
    #[serde(default)]
    pub reason: Option<String>,
}

/// `POST /dp/v1/orgs/{org}/exports`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgExport {
    pub command_id: String,
    pub export_id: String,
}

/// `DELETE /dp/v1/orgs/{org}`: a tombstone now; the bytes go with the
/// explicit deletion job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgDelete {
    pub command_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommandAck {
    pub command_id: String,
    pub accepted: bool,
    #[serde(default)]
    pub error: Option<String>,
}

/// `GET /dp/v1/orgs/{org}/commands/{command_id}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandStatus {
    pub command_id: String,
    /// `running`, `done` or `failed`.
    pub state: String,
    #[serde(default)]
    pub progress: Option<f32>,
    #[serde(default)]
    pub error: Option<String>,
}

// ------------------------------------------------------------ allowances

/// `POST /dp/v1/tenants/{org}/allowances`. Idempotent by `request_id`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AllowanceRequest {
    pub request_id: String,
    pub cell: String,
    pub incarnation: u64,
    pub want_micros: i64,
    /// Usage against earlier grants since the last request.
    #[serde(default)]
    pub settlements: Vec<GrantSettlement>,
}

/// Cumulative, so a repeated or late settlement changes nothing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrantSettlement {
    pub grant_id: String,
    /// Cumulative actual cost charged to the grant.
    pub used_micros: i64,
    /// Still reserved by operations in flight.
    pub reserved_micros: i64,
    /// Final: nothing more will be charged; the rest returns to the balance.
    pub closed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AllowanceGrant {
    /// Reserved in the ledger when issued.
    pub grant_id: String,
    /// 0: budget exhausted, or a policy stops spending.
    pub granted_micros: i64,
    /// After this the cell stops spending from the grant. The money is
    /// released only by a closed settlement, never by time.
    pub spend_until_unix_ms: i64,
    /// `budget_exhausted`, `ask_to_continue`, `suspended`.
    #[serde(default)]
    pub reason: Option<String>,
    /// The most one provider call may reserve; the cell's default when
    /// absent.
    #[serde(default)]
    pub per_call_micros: Option<i64>,
}

// ------------------------------------------------------------------ feed

/// `POST /dp/v1/feed`. At least once.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedBatch {
    pub data_plane: String,
    /// The cell that produced the records.
    pub source: String,
    /// Bumped when the cell is rebuilt from a restore.
    pub incarnation: u64,
    /// Ordered by `(index, ordinal)`.
    pub records: Vec<FeedRecord>,
}

/// Deduplicated by `(source, incarnation, index, ordinal)`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FeedRecord {
    /// Raft index of the entry that produced the record.
    pub index: u64,
    /// Its position among that entry's records.
    pub ordinal: u32,
    pub org_id: String,
    /// When the thing the record reports happened; the control plane's
    /// feed lag is measured from it.
    pub at_unix_ms: i64,
    #[serde(flatten)]
    pub record: Record,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum Record {
    /// One provider call attempt; a retry that bills again is a new one.
    Usage {
        session_id: String,
        turn_id: String,
        /// Usage's logical identity: a replay can't bill twice.
        attempt_id: String,
        /// The process died before usage came back; the reservation stays.
        #[serde(default)]
        unresolved: bool,
        agent_id: String,
        #[serde(default)]
        agent_hash: String,
        provider: String,
        model: String,
        input_tokens: u64,
        output_tokens: u64,
        cached_tokens: u64,
        cost_micros: i64,
        /// `model`, `tool`, `sandbox`, `egress`.
        kind: String,
    },
    /// A session's summary, for lists and search.
    Session {
        session_id: String,
        /// Monotonic per session, also across moves: an older version
        /// never overwrites a newer one.
        session_version: u64,
        agent_id: String,
        #[serde(default)]
        title: String,
        /// `idle`, `running`, `waiting`, `failed`, `deleted`.
        status: String,
        created_unix_ms: i64,
        last_activity_unix_ms: i64,
        turns: u64,
        #[serde(default)]
        cost_micros: i64,
    },
    /// A data-plane-refreshed connection token.
    SecretRefreshed {
        secret_id: String,
        revision: u64,
        ciphertext: String,
        nonce: String,
        data_key_id: String,
    },
    ConfigApplied {
        version: u64,
    },
    OrgEvent {
        /// `budget_exhausted`, `limit_hit`, `suspended`.
        kind: String,
        #[serde(default)]
        detail: String,
    },
    Health {
        sessions: u32,
        running_turns: u32,
        lag_ms: u32,
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

/// `POST /dp/v1/feed/replay`: resend from the shipped log.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FeedReplay {
    pub command_id: String,
    pub source: String,
    pub incarnation: u64,
    pub from_index: u64,
}

// ------------------------------------------------------------ org tokens

/// The claims of an org token: an EdDSA JWT the control plane signs and
/// the edge and gateways verify with its JWKS, with no call back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OrgTokenClaims {
    /// User id or API key id.
    pub sub: String,
    pub org: String,
    /// The data plane the edge routes the org to.
    pub dp: String,
    /// Roles and scopes, space separated (`sessions:write admin`).
    pub scope: String,
    pub iat: i64,
    /// About ten minutes after `iat`.
    pub exp: i64,
    /// Token id, for audit.
    pub jti: String,
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
            ConfigChange::parse(&json!({ "kind": "Webhook", "id": "w" })),
            None,
            "a change of a kind not known here is skipped"
        );
        let record: FeedRecord = serde_json::from_value(json!({
            "index": 1, "ordinal": 0, "org_id": "o", "at_unix_ms": 0,
            "type": "Anomaly", "sessions": 3
        }))
        .unwrap();
        assert_eq!(record.record, Record::Unknown);
    }

    #[test]
    fn changes_round_trip() {
        for change in [
            json!({
                "kind": "Agent", "agent_id": "a", "revision": 2, "format": 1,
                "package": { "name": "a", "instructions": "x" }
            }),
            json!({
                "kind": "Secret", "secret_id": "s", "revision": 3,
                "ciphertext": "AA==", "nonce": "AA==", "data_key_id": "k"
            }),
            json!({
                "kind": "Connection", "connection_id": "c", "connection_kind": "github",
                "settings": { "org": "x" }, "token_secret_id": "s", "refresh_owner": "DataPlane"
            }),
            json!({
                "kind": "Policy", "limits": {}, "egress": {},
                "placement": { "kind": "Dedicated", "pool": "p" }, "features": ["s1"]
            }),
            json!({ "kind": "Removal", "of": "agent", "id": "a" }),
        ] {
            let parsed = ConfigChange::parse(&change).unwrap();
            assert_eq!(parsed.to_value(), change);
        }
    }

    #[test]
    fn a_usage_record_keeps_older_writers_readable() {
        // Step 6's cells wrote usage without `agent_hash`.
        let record: FeedRecord = serde_json::from_value(json!({
            "index": 4, "ordinal": 1, "org_id": "o", "at_unix_ms": 7,
            "type": "Usage", "session_id": "s", "turn_id": "t",
            "attempt_id": "a", "agent_id": "g", "provider": "p", "model": "m",
            "input_tokens": 1, "output_tokens": 2, "cached_tokens": 0,
            "cost_micros": 10, "kind": "model"
        }))
        .unwrap();
        let Record::Usage { agent_hash, .. } = record.record else {
            panic!("usage");
        };
        assert_eq!(agent_hash, "");
    }
}
