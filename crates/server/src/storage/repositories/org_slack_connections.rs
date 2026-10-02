use anyhow::Result;
use chrono::{DateTime, Utc};
use uuid::Uuid;

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
                org_id, team_id, team_name, access_token_encrypted,
                refresh_token_encrypted, access_token_expires_at, state, token_generation
            )
            VALUES ($1, $2, $3, $4, $5, $6, 'connected', 1)
            ON CONFLICT (org_id, team_id) DO UPDATE SET
                team_name = COALESCE(EXCLUDED.team_name, org_slack_connections.team_name),
                access_token_encrypted = EXCLUDED.access_token_encrypted,
                refresh_token_encrypted = EXCLUDED.refresh_token_encrypted,
                access_token_expires_at = EXCLUDED.access_token_expires_at,
                state = 'connected',
                token_generation = org_slack_connections.token_generation + 1
            RETURNING *
            "#,
        )
        .bind(input.org_id)
        .bind(input.team_id)
        .bind(input.team_name)
        .bind(input.access_token_encrypted)
        .bind(input.refresh_token_encrypted)
        .bind(input.access_token_expires_at)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn get_org_slack_connection(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        Ok(
            sqlx::query_as("SELECT * FROM org_slack_connections WHERE org_id = $1 AND id = $2")
                .bind(org_id)
                .bind(id)
                .fetch_optional(&self.pool)
                .await?,
        )
    }

    pub async fn list_org_slack_connections(
        &self,
        org_id: i64,
    ) -> Result<Vec<OrgSlackConnectionRow>> {
        Ok(sqlx::query_as(
            "SELECT * FROM org_slack_connections WHERE org_id = $1 ORDER BY created_at, id",
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn claim_org_slack_connection_rotation(
        &self,
        id: Uuid,
        expected_generation: i64,
    ) -> Result<Option<OrgSlackConnectionRow>> {
        Ok(sqlx::query_as(
            r#"
            UPDATE org_slack_connections
            SET state = 'rotating',
                token_generation = token_generation + 1
            WHERE id = $1
              AND token_generation = $2
              AND state = 'connected'
            RETURNING *
            "#,
        )
        .bind(id)
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
                team_id = COALESCE(team_id, $6),
                state = 'connected',
                token_generation = token_generation + 1
            WHERE id = $1
              AND token_generation = $2
              AND state = 'rotating'
            RETURNING *
            "#,
        )
        .bind(input.id)
        .bind(input.expected_generation)
        .bind(input.access_token_encrypted)
        .bind(input.refresh_token_encrypted)
        .bind(input.access_token_expires_at)
        .bind(input.team_id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn mark_org_slack_reconnect_required(
        &self,
        id: Uuid,
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
            WHERE id = $1
              AND token_generation = $2
              AND state = 'rotating'
            "#,
        )
        .bind(id)
        .bind(expected_generation)
        .execute(&self.pool)
        .await?
        .rows_affected()
            > 0)
    }

    /// Connections to rotate now: those near expiry, and those that predate
    /// workspace identity, since rotation is what tells us their workspace.
    pub async fn list_due_org_slack_connections(
        &self,
        rotate_before: DateTime<Utc>,
    ) -> Result<Vec<OrgSlackConnectionRow>> {
        Ok(sqlx::query_as(
            r#"
            SELECT * FROM org_slack_connections
            WHERE state = 'connected'
              AND (access_token_expires_at <= $1 OR team_id IS NULL)
            ORDER BY access_token_expires_at
            "#,
        )
        .bind(rotate_before)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn delete_org_slack_connection(&self, org_id: i64, id: Uuid) -> Result<bool> {
        Ok(
            sqlx::query("DELETE FROM org_slack_connections WHERE org_id = $1 AND id = $2")
                .bind(org_id)
                .bind(id)
                .execute(&self.pool)
                .await?
                .rows_affected()
                > 0,
        )
    }
}
