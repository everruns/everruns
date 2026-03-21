// Repository layer: Session Git Objects & Refs

use super::Database;
use crate::storage::models::*;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // Session Git Objects
    // ============================================

    pub async fn write_git_object(&self, input: CreateSessionGitObject) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO session_git_objects (session_id, oid, obj_type, size, data)
            VALUES ($1, $2, $3, $4, $5)
            ON CONFLICT (session_id, oid) DO NOTHING
            "#,
        )
        .bind(input.session_id)
        .bind(&input.oid)
        .bind(input.obj_type)
        .bind(input.size)
        .bind(&input.data)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Write multiple git objects in a single transaction.
    pub async fn write_git_objects_batch(
        &self,
        objects: Vec<CreateSessionGitObject>,
    ) -> Result<()> {
        if objects.is_empty() {
            return Ok(());
        }
        let mut tx = self.pool.begin().await?;
        for obj in &objects {
            sqlx::query(
                r#"
                INSERT INTO session_git_objects (session_id, oid, obj_type, size, data)
                VALUES ($1, $2, $3, $4, $5)
                ON CONFLICT (session_id, oid) DO NOTHING
                "#,
            )
            .bind(obj.session_id)
            .bind(&obj.oid)
            .bind(obj.obj_type)
            .bind(obj.size)
            .bind(&obj.data)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Read a git object by OID.
    pub async fn read_git_object(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<SessionGitObjectRow>> {
        let row = sqlx::query_as::<_, SessionGitObjectRow>(
            r#"
            SELECT session_id, oid, obj_type, size, data
            FROM session_git_objects
            WHERE session_id = $1 AND oid = $2
            "#,
        )
        .bind(session_id)
        .bind(oid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Load all git objects for a session in a single query (avoids N+1).
    pub async fn load_all_git_objects(&self, session_id: Uuid) -> Result<Vec<SessionGitObjectRow>> {
        let rows = sqlx::query_as::<_, SessionGitObjectRow>(
            r#"
            SELECT session_id, oid, obj_type, size, data
            FROM session_git_objects
            WHERE session_id = $1
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Check if a git object exists.
    pub async fn git_object_exists(&self, session_id: Uuid, oid: &[u8]) -> Result<bool> {
        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT TRUE FROM session_git_objects WHERE session_id = $1 AND oid = $2 LIMIT 1",
        )
        .bind(session_id)
        .bind(oid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    /// Read a git object's header (type + size) without fetching data.
    pub async fn read_git_object_header(
        &self,
        session_id: Uuid,
        oid: &[u8],
    ) -> Result<Option<(i16, i64)>> {
        let row: Option<(i16, i64)> = sqlx::query_as(
            "SELECT obj_type, size FROM session_git_objects WHERE session_id = $1 AND oid = $2",
        )
        .bind(session_id)
        .bind(oid)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// List all git object OIDs for a session.
    pub async fn list_git_object_oids(&self, session_id: Uuid) -> Result<Vec<Vec<u8>>> {
        let rows: Vec<(Vec<u8>,)> =
            sqlx::query_as("SELECT oid FROM session_git_objects WHERE session_id = $1")
                .bind(session_id)
                .fetch_all(&self.pool)
                .await?;

        Ok(rows.into_iter().map(|(oid,)| oid).collect())
    }

    /// Copy all git objects from one session to another (for fork).
    pub async fn fork_git_objects(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        let result = sqlx::query(
            r#"
            INSERT INTO session_git_objects (session_id, oid, obj_type, size, data)
            SELECT $2, oid, obj_type, size, data
            FROM session_git_objects
            WHERE session_id = $1
            ON CONFLICT (session_id, oid) DO NOTHING
            "#,
        )
        .bind(source_session_id)
        .bind(target_session_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }

    // ============================================
    // Session Git Refs
    // ============================================

    /// Write (upsert) a git ref.
    pub async fn write_git_ref(&self, input: CreateSessionGitRef) -> Result<()> {
        sqlx::query(
            r#"
            INSERT INTO session_git_refs (session_id, name, target, is_symbolic)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (session_id, name) DO UPDATE
            SET target = EXCLUDED.target, is_symbolic = EXCLUDED.is_symbolic
            "#,
        )
        .bind(input.session_id)
        .bind(&input.name)
        .bind(&input.target)
        .bind(input.is_symbolic)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    /// Read a git ref by name.
    pub async fn read_git_ref(
        &self,
        session_id: Uuid,
        name: &str,
    ) -> Result<Option<SessionGitRefRow>> {
        let row = sqlx::query_as::<_, SessionGitRefRow>(
            r#"
            SELECT session_id, name, target, is_symbolic
            FROM session_git_refs
            WHERE session_id = $1 AND name = $2
            "#,
        )
        .bind(session_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Delete a git ref.
    pub async fn delete_git_ref(&self, session_id: Uuid, name: &str) -> Result<bool> {
        let result =
            sqlx::query("DELETE FROM session_git_refs WHERE session_id = $1 AND name = $2")
                .bind(session_id)
                .bind(name)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    /// List all git refs for a session.
    pub async fn list_git_refs(&self, session_id: Uuid) -> Result<Vec<SessionGitRefRow>> {
        let rows = sqlx::query_as::<_, SessionGitRefRow>(
            r#"
            SELECT session_id, name, target, is_symbolic
            FROM session_git_refs
            WHERE session_id = $1
            ORDER BY name
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Copy all git refs from one session to another (for fork).
    pub async fn fork_git_refs(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<u64> {
        let result = sqlx::query(
            r#"
            INSERT INTO session_git_refs (session_id, name, target, is_symbolic)
            SELECT $2, name, target, is_symbolic
            FROM session_git_refs
            WHERE session_id = $1
            ON CONFLICT (session_id, name) DO NOTHING
            "#,
        )
        .bind(source_session_id)
        .bind(target_session_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected())
    }
}
