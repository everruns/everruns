use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_provider::typed_id::TriggerId;

use super::InMemoryDatabase;
use crate::storage::agent_trigger_mcp_subscriptions::{
    AgentTriggerMcpSubscriptionRow, MCP_SUBSCRIPTION_PENDING, UpsertAgentTriggerMcpSubscription,
};

impl InMemoryDatabase {
    pub async fn upsert_agent_trigger_mcp_subscription(
        &self,
        input: UpsertAgentTriggerMcpSubscription,
    ) -> Result<AgentTriggerMcpSubscriptionRow> {
        let now = Self::now();
        let mut rows = self.agent_trigger_mcp_subscriptions.write();
        let created_at = rows
            .get(&input.trigger_id)
            .map_or(now, |existing| existing.created_at);
        let row = AgentTriggerMcpSubscriptionRow {
            trigger_id: input.trigger_id,
            org_id: input.org_id,
            secret_encrypted: input.secret_encrypted,
            remote_subscription_id: input.remote_subscription_id,
            refresh_before: input.refresh_before,
            cursor: input.cursor,
            status: input.status,
            last_error: input.last_error,
            created_at,
            updated_at: now,
        };
        rows.insert(input.trigger_id, row.clone());
        Ok(row)
    }

    pub async fn get_agent_trigger_mcp_subscription(
        &self,
        trigger_id: TriggerId,
    ) -> Result<Option<AgentTriggerMcpSubscriptionRow>> {
        Ok(self
            .agent_trigger_mcp_subscriptions
            .read()
            .get(&trigger_id)
            .cloned())
    }

    pub async fn delete_agent_trigger_mcp_subscription(
        &self,
        trigger_id: TriggerId,
    ) -> Result<bool> {
        Ok(self
            .agent_trigger_mcp_subscriptions
            .write()
            .remove(&trigger_id)
            .is_some())
    }

    pub async fn list_agent_trigger_mcp_subscriptions_due(
        &self,
        due: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<AgentTriggerMcpSubscriptionRow>> {
        let mut rows: Vec<_> = self
            .agent_trigger_mcp_subscriptions
            .read()
            .values()
            .filter(|row| row.status != MCP_SUBSCRIPTION_PENDING)
            .filter(|row| row.refresh_before.is_some_and(|at| at <= due))
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.refresh_before);
        rows.truncate(usize::try_from(limit.max(0)).unwrap_or_default());
        Ok(rows)
    }
}
