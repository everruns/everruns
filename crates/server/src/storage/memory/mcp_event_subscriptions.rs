use anyhow::Result;
use chrono::{DateTime, Utc};

use super::InMemoryDatabase;
use crate::storage::{McpEventSubscriptionRow, UpsertMcpEventSubscription};

impl InMemoryDatabase {
    pub async fn upsert_mcp_event_subscription(
        &self,
        input: UpsertMcpEventSubscription,
    ) -> Result<McpEventSubscriptionRow> {
        let now = Self::now();
        let mut subscriptions = self.mcp_event_subscriptions.write();
        let row = match subscriptions.get(&input.subscription_key) {
            Some(existing) => McpEventSubscriptionRow {
                secret_encrypted: input.secret_encrypted,
                expires_at: input.expires_at,
                updated_at: now,
                ..existing.clone()
            },
            None => McpEventSubscriptionRow {
                id: uuid::Uuid::now_v7(),
                subscription_key: input.subscription_key.clone(),
                org_id: input.org_id,
                user_id: input.user_id,
                event_name: input.event_name,
                arguments: input.arguments,
                callback_url: input.callback_url,
                secret_encrypted: input.secret_encrypted,
                expires_at: input.expires_at,
                created_at: now,
                updated_at: now,
            },
        };
        subscriptions.insert(input.subscription_key, row.clone());
        Ok(row)
    }

    pub async fn delete_mcp_event_subscription(&self, subscription_key: &str) -> Result<bool> {
        Ok(self
            .mcp_event_subscriptions
            .write()
            .remove(subscription_key)
            .is_some())
    }

    pub async fn list_active_mcp_event_subscriptions(
        &self,
        org_id: i64,
        event_name: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<McpEventSubscriptionRow>> {
        let mut rows: Vec<_> = self
            .mcp_event_subscriptions
            .read()
            .values()
            .filter(|row| row.org_id == org_id && row.event_name == event_name)
            .filter(|row| row.expires_at > now)
            .cloned()
            .collect();
        rows.sort_by_key(|row| row.created_at);
        Ok(rows)
    }
}
