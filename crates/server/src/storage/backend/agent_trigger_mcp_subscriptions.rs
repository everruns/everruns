use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_provider::typed_id::TriggerId;

use super::StorageBackend;
use crate::storage::agent_trigger_mcp_subscriptions::{
    AgentTriggerMcpSubscriptionRow, UpsertAgentTriggerMcpSubscription,
};

impl StorageBackend {
    pub async fn upsert_agent_trigger_mcp_subscription(
        &self,
        input: UpsertAgentTriggerMcpSubscription,
    ) -> Result<AgentTriggerMcpSubscriptionRow> {
        dispatch!(self, upsert_agent_trigger_mcp_subscription, input)
    }

    pub async fn get_agent_trigger_mcp_subscription(
        &self,
        trigger_id: TriggerId,
    ) -> Result<Option<AgentTriggerMcpSubscriptionRow>> {
        dispatch!(self, get_agent_trigger_mcp_subscription, trigger_id)
    }

    pub async fn delete_agent_trigger_mcp_subscription(
        &self,
        trigger_id: TriggerId,
    ) -> Result<bool> {
        dispatch!(self, delete_agent_trigger_mcp_subscription, trigger_id)
    }

    /// Confirmed or failing subscriptions whose `refresh_before` is at or
    /// before `due`, oldest first.
    pub async fn list_agent_trigger_mcp_subscriptions_due(
        &self,
        due: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<AgentTriggerMcpSubscriptionRow>> {
        dispatch!(self, list_agent_trigger_mcp_subscriptions_due, due, limit)
    }
}
