use anyhow::Result;
use chrono::{DateTime, Utc};

use super::Database;
use crate::storage::{OrgSlackConnectionRow, RotateOrgSlackConnection, UpsertOrgSlackConnection};

impl Database {
    pub async fn upsert_org_slack_connection(
        &self,
        input: UpsertOrgSlackConnection,
    ) -> Result<OrgSlackConnectionRow> {
        Ok(sqlx::query_as(
            r#"
            INSERT INTO org_slack_connections (
                org_id, access_token_encrypted, refresh_token_encrypted,
                access_token_expires_at, state, token_generation
            )
            VALUES ($1, $2, $3, $4, 'connected', 1)
            ON CONFLICT (org_id) DO UPDATE SET
                access_token_encrypted = EXCLUDED.access_token_encrypted,
                refresh_token_encrypted = EXCLUDED.refresh_token_encrypted,
                access_token_expires_at = EXCLUDED.access_token_expires_at,
                state = 'connected',
                token_generation = org_slack_connections.token_generation + 1
            RETURNING *
            "#,
        )
        .bind(input.org_id)
        .bind(input.access_token_encrypted)
        .bind(input.refresh_token_encrypted)
        .bind(input.access_token_expires_at)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_org_slack_connection(
        &self,
        org_id: i64,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        Ok(
            sqlx::query_as("SELECT * FROM org_slack_connections WHERE org_id = $1")
                .bind(org_id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn claim_org_slack_connection_rotation(
        &self,
        org_id: i64,
        expected_generation: i64,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        Ok(sqlx::query_as(
            r#"
            UPDATE org_slack_connections
            SET token_generation = token_generation + 1
            WHERE org_id = $1
              AND token_generation = $2
              AND state = 'connected'
            RETURNING *
            "#,
        )
        .bind(org_id)
        .bind(expected_generation)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn rotate_org_slack_connection(
        &self,
        input: RotateOrgSlackConnection,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        Ok(sqlx::query_as(
            r#"
            UPDATE org_slack_connections
            SET access_token_encrypted = $3,
                refresh_token_encrypted = $4,
                access_token_expires_at = $5,
                token_generation = token_generation + 1
            WHERE org_id = $1
              AND token_generation = $2
              AND state = 'connected'
            RETURNING *
            "#,
        )
        .bind(input.org_id)
        .bind(input.expected_generation)
        .bind(input.access_token_encrypted)
        .bind(input.refresh_token_encrypted)
        .bind(input.access_token_expires_at)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn mark_org_slack_reconnect_required(
        &self,
        org_id: i64,
        expected_generation: i64,
    ) -> Result<bool> {
        Ok(sqlx::query(
            r#"
            UPDATE org_slack_connections
            SET state = 'reconnect_required',
                access_token_encrypted = NULL,
                refresh_token_encrypted = NULL,
                access_token_expires_at = NULL,
                token_generation = token_generation + 1
            WHERE org_id = $1
              AND token_generation = $2
              AND state = 'connected'
            "#,
        )
        .bind(org_id)
        .bind(expected_generation)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    pub async fn list_due_org_slack_connections(
        &self,
        rotate_before: DateTime<Utc>,
    ) -> Result<Vec<OrgSlackConnectionRow>> {
        Ok(sqlx::query_as(
            r#"
            SELECT * FROM org_slack_connections
            WHERE state = 'connected'
              AND access_token_expires_at <= $1
            ORDER BY access_token_expires_at
            "#,
        )
        .bind(rotate_before)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_org_slack_connection(&self, org_id: i64) -> Result<bool> {
        Ok(
            sqlx::query("DELETE FROM org_slack_connections WHERE org_id = $1")
                .bind(org_id)
                .execute(&self.pool)
                .await?
                .rows_affected()
                > 0,
        )
    }
}
