// Session resource registry repository (PostgreSQL)

use super::super::models::{SessionResourceRow, UpsertSessionResourceRow};
use super::Database;
use anyhow::Result;
use everruns_core::SessionId;

impl Database {
    pub async fn upsert_session_resource(
        &self,
        input: UpsertSessionResourceRow,
    ) -> Result<SessionResourceRow> {
        let row = sqlx::query_as::<_, SessionResourceRow>(
            r#"
            INSERT INTO session_resources (session_id, resource_id, kind, display_name, status, metadata)
            VALUES ($1, $2, $3, $4, $5, $6)
            ON CONFLICT (session_id, resource_id) DO UPDATE SET
                kind = EXCLUDED.kind,
                display_name = EXCLUDED.display_name,
                status = EXCLUDED.status,
                metadata = EXCLUDED.metadata
            RETURNING id, session_id, resource_id, kind, display_name, status, metadata, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.resource_id)
        .bind(&input.kind)
        .bind(&input.display_name)
        .bind(&input.status)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn update_session_resource_status(
        &self,
        session_id: SessionId,
        resource_id: &str,
        status: &str,
    ) -> Result<Option<SessionResourceRow>> {
        let row = sqlx::query_as::<_, SessionResourceRow>(
            r#"
            UPDATE session_resources
            SET status = $3
            WHERE session_id = $1 AND resource_id = $2
            RETURNING id, session_id, resource_id, kind, display_name, status, metadata, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(resource_id)
        .bind(status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<Option<SessionResourceRow>> {
        let row = sqlx::query_as::<_, SessionResourceRow>(
            r#"
            SELECT id, session_id, resource_id, kind, display_name, status, metadata, created_at, updated_at
            FROM session_resources
            WHERE session_id = $1 AND resource_id = $2
            "#,
        )
        .bind(session_id)
        .bind(resource_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_session_resources(
        &self,
        session_id: SessionId,
        kind: Option<&str>,
        status: Option<&str>,
    ) -> Result<Vec<SessionResourceRow>> {
        let rows = sqlx::query_as::<_, SessionResourceRow>(
            r#"
            SELECT id, session_id, resource_id, kind, display_name, status, metadata, created_at, updated_at
            FROM session_resources
            WHERE session_id = $1
              AND ($2::text IS NULL OR kind = $2)
              AND ($3::text IS NULL OR status = $3)
            ORDER BY created_at ASC
            "#,
        )
        .bind(session_id)
        .bind(kind)
        .bind(status)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn delete_session_resource(
        &self,
        session_id: SessionId,
        resource_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"DELETE FROM session_resources WHERE session_id = $1 AND resource_id = $2"#,
        )
        .bind(session_id)
        .bind(resource_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
