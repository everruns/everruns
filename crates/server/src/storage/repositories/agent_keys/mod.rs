// PostgreSQL repository: agent keys for `api` channels.

pub(super) mod rows;
use rows::*;

use super::Database;
use anyhow::Result;
use chrono::{DateTime, Utc};
use everruns_server_macros::sql;
use uuid::Uuid;

impl Database {
    pub async fn create_agent_key(&self, input: CreateAgentKeyRow) -> Result<AgentKeyRow> {
        let row = sqlx::query_as::<_, AgentKeyRow>(sql!(
            r#"
            INSERT INTO agent_keys
                (org_id, channel_id, name, token_hash, token_prefix, permissions, expires_at, created_by_user_id)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            RETURNING {AgentKeyRow}
            "#
        ))
        .bind(input.org_id)
        .bind(input.channel_id)
        .bind(&input.name)
        .bind(&input.token_hash)
        .bind(&input.token_prefix)
        .bind(&input.permissions)
        .bind(input.expires_at)
        .bind(input.created_by_user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    /// Keys granted to one channel, newest first, revoked ones included.
    pub async fn list_agent_keys(&self, org_id: i64, channel_id: Uuid) -> Result<Vec<AgentKeyRow>> {
        let rows = sqlx::query_as::<_, AgentKeyRow>(sql!(
            r#"
            SELECT {AgentKeyRow} FROM agent_keys
            WHERE org_id = $1 AND channel_id = $2
            ORDER BY created_at DESC, id DESC
            "#
        ))
        .bind(org_id)
        .bind(channel_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    pub async fn get_agent_key(
        &self,
        org_id: i64,
        channel_id: Uuid,
        id: Uuid,
    ) -> Result<Option<AgentKeyRow>> {
        let row = sqlx::query_as::<_, AgentKeyRow>(sql!(
            r#"
            SELECT {AgentKeyRow} FROM agent_keys
            WHERE org_id = $1 AND channel_id = $2 AND id = $3
            "#
        ))
        .bind(org_id)
        .bind(channel_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// The live key a secret belongs to: its current secret, or the previous
    /// one inside the rotation overlap. Revoked and expired keys are not found.
    pub async fn find_agent_key_by_hash(&self, token_hash: &str) -> Result<Option<AgentKeyRow>> {
        let row = sqlx::query_as::<_, AgentKeyRow>(sql!(
            r#"
            SELECT {AgentKeyRow} FROM agent_keys
            WHERE (token_hash = $1
                   OR (previous_token_hash = $1 AND previous_valid_until > NOW()))
              AND revoked_at IS NULL
              AND (expires_at IS NULL OR expires_at > NOW())
            "#
        ))
        .bind(token_hash)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Replace a key's secret; the old one keeps working until `previous_valid_until`.
    pub async fn rotate_agent_key(
        &self,
        org_id: i64,
        channel_id: Uuid,
        id: Uuid,
        token_hash: &str,
        token_prefix: &str,
        previous_valid_until: DateTime<Utc>,
    ) -> Result<Option<AgentKeyRow>> {
        let row = sqlx::query_as::<_, AgentKeyRow>(sql!(
            r#"
            UPDATE agent_keys
            SET previous_token_hash = token_hash,
                previous_valid_until = $6,
                token_hash = $4,
                token_prefix = $5
            WHERE org_id = $1 AND channel_id = $2 AND id = $3 AND revoked_at IS NULL
            RETURNING {AgentKeyRow}
            "#
        ))
        .bind(org_id)
        .bind(channel_id)
        .bind(id)
        .bind(token_hash)
        .bind(token_prefix)
        .bind(previous_valid_until)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Revoke a key. Idempotent: an already revoked key keeps its first time.
    pub async fn revoke_agent_key(
        &self,
        org_id: i64,
        channel_id: Uuid,
        id: Uuid,
    ) -> Result<Option<AgentKeyRow>> {
        let row = sqlx::query_as::<_, AgentKeyRow>(sql!(
            r#"
            UPDATE agent_keys
            SET revoked_at = COALESCE(revoked_at, NOW()),
                previous_token_hash = NULL,
                previous_valid_until = NULL
            WHERE org_id = $1 AND channel_id = $2 AND id = $3
            RETURNING {AgentKeyRow}
            "#
        ))
        .bind(org_id)
        .bind(channel_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    /// Record a use, at most once a minute per key.
    pub async fn touch_agent_key(&self, id: Uuid) -> Result<()> {
        sqlx::query(
            "UPDATE agent_keys SET last_used_at = NOW()
             WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < NOW() - INTERVAL '1 minute')",
        )
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// What one Agent API caller spent on one channel since `since`, in US
    /// dollars: every generation of the channel's sessions carrying all of
    /// `tags` (the caller's identity tags), actual cost when known.
    pub async fn api_caller_spend_since(
        &self,
        org_id: i64,
        channel_id: Uuid,
        tags: &[String],
        since: DateTime<Utc>,
    ) -> Result<f64> {
        let (spent,): (Option<f64>,) = sqlx::query_as(
            r#"
            SELECT SUM(COALESCE(lg.actual_cost_usd, lg.estimated_cost_usd, 0))::DOUBLE PRECISION
            FROM llm_generations lg
            JOIN sessions s ON s.id = lg.session_id
            WHERE lg.org_id = $1 AND lg.created_at >= $4
              AND s.org_id = $1 AND s.channel_id = $2 AND s.tags @> $3
            "#,
        )
        .bind(org_id)
        .bind(channel_id)
        .bind(tags)
        .bind(since)
        .fetch_one(&self.pool)
        .await?;
        Ok(spent.unwrap_or(0.0))
    }
}
