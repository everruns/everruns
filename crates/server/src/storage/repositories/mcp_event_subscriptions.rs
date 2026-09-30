use anyhow::Result;
use chrono::{DateTime, Utc};

use super::Database;
use crate::storage::{McpEventSubscriptionRow, UpsertMcpEventSubscription};

impl Database {
    pub async fn upsert_mcp_event_subscription(
        &self,
        input: UpsertMcpEventSubscription,
    ) -> Result<McpEventSubscriptionRow> {
        Ok(sqlx::query_as(
            r#"
            INSERT INTO mcp_event_subscriptions (
                subscription_key, org_id, user_id, event_name, arguments,
                callback_url, secret_encrypted, expires_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (subscription_key) DO UPDATE SET
                secret_encrypted = EXCLUDED.secret_encrypted,
                expires_at = EXCLUDED.expires_at
            RETURNING *
            "#,
        )
        .bind(input.subscription_key)
        .bind(input.org_id)
        .bind(input.user_id)
        .bind(input.event_name)
        .bind(input.arguments)
        .bind(input.callback_url)
        .bind(input.secret_encrypted)
        .bind(input.expires_at)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn delete_mcp_event_subscription(&self, subscription_key: &str) -> Result<bool> {
        let result = sqlx::query("DELETE FROM mcp_event_subscriptions WHERE subscription_key = $1")
            .bind(subscription_key)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_active_mcp_event_subscriptions(
        &self,
        org_id: i64,
        event_name: &str,
        now: DateTime<Utc>,
    ) -> Result<Vec<McpEventSubscriptionRow>> {
        Ok(sqlx::query_as(
            r#"
            SELECT * FROM mcp_event_subscriptions
            WHERE org_id = $1 AND event_name = $2 AND expires_at > $3
            ORDER BY created_at
            "#,
        )
        .bind(org_id)
        .bind(event_name)
        .bind(now)
        .fetch_all(&self.pool)
        .await?)
    }
}
