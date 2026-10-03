use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_contracts::typed_id::TriggerId;

use super::Database;
use crate::storage::agent_trigger_mcp_subscriptions::{
    AgentTriggerMcpSubscriptionRow, UpsertAgentTriggerMcpSubscription,
};

impl Database {
    pub async fn upsert_agent_trigger_mcp_subscription(
        &self,
        input: UpsertAgentTriggerMcpSubscription,
    ) -> Result<AgentTriggerMcpSubscriptionRow> {
        Ok(sqlx::query_as(
            r#"
            INSERT INTO agent_trigger_mcp_subscriptions (
                trigger_id, org_id, secret_encrypted, remote_subscription_id,
                refresh_before, cursor, status, last_error
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (trigger_id) DO UPDATE SET
                secret_encrypted = EXCLUDED.secret_encrypted,
                remote_subscription_id = EXCLUDED.remote_subscription_id,
                refresh_before = EXCLUDED.refresh_before,
                cursor = EXCLUDED.cursor,
                status = EXCLUDED.status,
                last_error = EXCLUDED.last_error
            RETURNING *
            "#,
        )
        .bind(input.trigger_id)
        .bind(input.org_id)
        .bind(input.secret_encrypted)
        .bind(input.remote_subscription_id)
        .bind(input.refresh_before)
        .bind(input.cursor)
        .bind(input.status)
        .bind(input.last_error)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_agent_trigger_mcp_subscription(
        &self,
        trigger_id: TriggerId,
    ) -> Result<Option<AgentTriggerMcpSubscriptionRow>> {
        Ok(
            sqlx::query_as("SELECT * FROM agent_trigger_mcp_subscriptions WHERE trigger_id = $1")
                .bind(trigger_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn delete_agent_trigger_mcp_subscription(
        &self,
        trigger_id: TriggerId,
    ) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM agent_trigger_mcp_subscriptions WHERE trigger_id = $1")
                .bind(trigger_id)
                .execute(&self.pool)
                .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn list_agent_trigger_mcp_subscriptions_due(
        &self,
        due: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<AgentTriggerMcpSubscriptionRow>> {
        Ok(sqlx::query_as(
            r#"
            SELECT * FROM agent_trigger_mcp_subscriptions
            WHERE status <> 'pending' AND refresh_before <= $1
            ORDER BY refresh_before
            LIMIT $2
            "#,
        )
        .bind(due)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }
}
