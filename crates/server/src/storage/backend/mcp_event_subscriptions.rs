use anyhow::Result;
use chrono::{DateTime, Utc};

use super::StorageBackend;
use crate::storage::{McpEventSubscriptionRow, UpsertMcpEventSubscription};

impl StorageBackend {
    pub async fn upsert_mcp_event_subscription(
        &self,
        input: UpsertMcpEventSubscription,
    ) -> Result<McpEventSubscriptionRow> {
        dispatch!(self, upsert_mcp_event_subscription, input)
    }

    pub async fn delete_mcp_event_subscription(&self, subscription_key: &str) -> Result<bool> {
        dispatch!(self, delete_mcp_event_subscription, subscription_key)
    }

    pub async fn list_active_mcp_event_subscriptions(
        &self,
        org_id: i64,
        event_name: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<McpEventSubscriptionRow>> {
        dispatch!(
            self,
            list_active_mcp_event_subscriptions,
            org_id,
            event_name,
            now
        )
    }
}
