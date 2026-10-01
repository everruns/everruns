//! Inbound MCP Events subscription state for `mcp_event` agent triggers
//! (EVE-1121). See `knowledge/integrations/mcp-events.md` and
//! `domains::agent_triggers::mcp_event`.

use chrono::{DateTime, Utc};
use everruns_provider::typed_id::TriggerId;
use sqlx::FromRow;

/// Subscription is being set up: the secret exists, the server has not
/// confirmed it yet. Signed verification challenges are answered.
pub const MCP_SUBSCRIPTION_PENDING: &str = "pending";
/// The server confirmed the subscription; events are accepted.
pub const MCP_SUBSCRIPTION_ACTIVE: &str = "active";
/// A refresh failed; events are still accepted until the remote subscription
/// lapses, and the refresher keeps retrying.
pub const MCP_SUBSCRIPTION_FAILED: &str = "failed";

/// The subscription one `mcp_event` trigger holds on its MCP server.
#[derive(Clone, Debug, FromRow)]
pub struct AgentTriggerMcpSubscriptionRow {
    pub trigger_id: TriggerId,
    pub org_id: i64,
    /// The `whsec_` signing secret Everruns generated, encrypted.
    pub secret_encrypted: Vec<u8>,
    /// Subscription id the server returned (`X-MCP-Subscription-Id`).
    pub remote_subscription_id: Option<String>,
    /// Re-subscribe before this instant.
    pub refresh_before: Option<DateTime<Utc>>,
    /// Last cursor the server sent, passed back on refresh.
    pub cursor: Option<String>,
    pub status: String,
    pub last_error: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full replacement of a trigger's subscription state.
#[derive(Clone, Debug)]
pub struct UpsertAgentTriggerMcpSubscription {
    pub trigger_id: TriggerId,
    pub org_id: i64,
    pub secret_encrypted: Vec<u8>,
    pub remote_subscription_id: Option<String>,
    pub refresh_before: Option<DateTime<Utc>>,
    pub cursor: Option<String>,
    pub status: String,
    pub last_error: Option<String>,
}

impl From<AgentTriggerMcpSubscriptionRow> for UpsertAgentTriggerMcpSubscription {
    fn from(row: AgentTriggerMcpSubscriptionRow) -> Self {
        Self {
            trigger_id: row.trigger_id,
            org_id: row.org_id,
            secret_encrypted: row.secret_encrypted,
            remote_subscription_id: row.remote_subscription_id,
            refresh_before: row.refresh_before,
            cursor: row.cursor,
            status: row.status,
            last_error: row.last_error,
        }
    }
}
