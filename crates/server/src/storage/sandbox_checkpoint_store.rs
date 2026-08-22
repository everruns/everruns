// PostgreSQL implementation of SandboxCheckpointStore (EVE-870, migration 121).
//
// Postgres-only, like `PgDurableToolResultStore`: these records exist to close a
// crash window between two database commits, so an in-memory backend has nothing
// meaningful to model. Hosts without Postgres simply do not install the store
// and keep the pre-EVE-870 secret-only behaviour.
//
// Two invariants drive the SQL:
//
// 1. An upload is recorded before it is attached. A crash between the two leaves
//    a collectable orphan, never a pointer to a revision no committed turn
//    produced.
// 2. Attaching is fenced on the sandbox generation, so a checkpoint uploaded
//    against a sandbox that has since been replaced cannot advance the pointer.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_platform::sandbox_checkpoint::{
    MAX_CHECKPOINT_COLLECT_LIMIT, NewSandboxCheckpoint, SandboxCheckpoint, SandboxCheckpointError,
    SandboxCheckpointKind, SandboxCheckpointStore, SandboxRef,
};
use everruns_provider::typed_id::SessionId;
use sqlx::PgPool;
use uuid::Uuid;

/// PostgreSQL-backed logical sandbox and checkpoint store.
#[derive(Clone)]
pub struct PgSandboxCheckpointStore {
    pool: PgPool,
}

impl PgSandboxCheckpointStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[derive(sqlx::FromRow)]
struct CheckpointRow {
    id: Uuid,
    sandbox_id: Uuid,
    generation: i64,
    source_turn_id: Option<String>,
    source_tool_call_id: Option<String>,
    kind: String,
    provider_ref: Option<String>,
    workspace_revision: String,
    attached_at: Option<DateTime<Utc>>,
    created_at: DateTime<Utc>,
}

fn storage_error(error: impl std::fmt::Display) -> SandboxCheckpointError {
    SandboxCheckpointError::Storage(error.to_string())
}

fn parse_kind(raw: &str) -> Result<SandboxCheckpointKind, SandboxCheckpointError> {
    match raw {
        "provider_native" => Ok(SandboxCheckpointKind::ProviderNative),
        "portable_workspace" => Ok(SandboxCheckpointKind::PortableWorkspace),
        other => Err(SandboxCheckpointError::Storage(format!(
            "unknown sandbox checkpoint kind '{other}'"
        ))),
    }
}

impl TryFrom<CheckpointRow> for SandboxCheckpoint {
    type Error = SandboxCheckpointError;

    fn try_from(row: CheckpointRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            sandbox_id: row.sandbox_id,
            generation: row.generation,
            source_turn_id: row.source_turn_id,
            source_tool_call_id: row.source_tool_call_id,
            kind: parse_kind(&row.kind)?,
            provider_ref: row.provider_ref,
            workspace_revision: row.workspace_revision,
            attached_at: row.attached_at,
            created_at: row.created_at,
        })
    }
}

#[async_trait]
impl SandboxCheckpointStore for PgSandboxCheckpointStore {
    async fn ensure_sandbox(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<SandboxRef, SandboxCheckpointError> {
        // `org_id` is denormalised from the owning session rather than passed
        // in: the tool context carries the typed `OrgId`, not the numeric key,
        // and sourcing it in SQL keeps the two rows from disagreeing.
        //
        // DO UPDATE rather than DO NOTHING so RETURNING yields a row on the
        // conflict path too; touching updated_at is the cheapest such no-op.
        let row: (Uuid, i64) = sqlx::query_as(
            r#"
            INSERT INTO sandboxes (org_id, session_id, provider)
            SELECT s.org_id, s.id, $2
            FROM sessions s
            WHERE s.id = $1
            ON CONFLICT (session_id, provider)
            DO UPDATE SET updated_at = NOW()
            RETURNING id, generation
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;

        Ok(SandboxRef {
            id: row.0,
            generation: row.1,
        })
    }

    async fn record_checkpoint(
        &self,
        checkpoint: NewSandboxCheckpoint,
    ) -> Result<SandboxCheckpoint, SandboxCheckpointError> {
        // DO UPDATE on the revision key keeps this idempotent under retry while
        // preserving `attached_at`: a replayed upload must not un-attach a
        // checkpoint that has already been committed.
        let row: CheckpointRow = sqlx::query_as(
            r#"
            INSERT INTO sandbox_checkpoints
                (sandbox_id, generation, source_turn_id, source_tool_call_id,
                 kind, provider_ref, workspace_revision)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (sandbox_id, workspace_revision)
            DO UPDATE SET
                source_turn_id =
                    COALESCE(sandbox_checkpoints.source_turn_id, EXCLUDED.source_turn_id),
                source_tool_call_id =
                    COALESCE(sandbox_checkpoints.source_tool_call_id, EXCLUDED.source_tool_call_id)
            RETURNING id, sandbox_id, generation, source_turn_id, source_tool_call_id,
                      kind, provider_ref, workspace_revision, attached_at, created_at
            "#,
        )
        .bind(checkpoint.sandbox_id)
        .bind(checkpoint.generation)
        .bind(checkpoint.source_turn_id.as_deref())
        .bind(checkpoint.source_tool_call_id.as_deref())
        .bind(checkpoint.kind.as_str())
        .bind(checkpoint.provider_ref.as_deref())
        .bind(&checkpoint.workspace_revision)
        .fetch_one(&self.pool)
        .await
        .map_err(storage_error)?;

        row.try_into()
    }

    async fn attach_checkpoint(
        &self,
        sandbox_id: Uuid,
        checkpoint_id: Uuid,
        generation: i64,
    ) -> Result<(), SandboxCheckpointError> {
        // Stamping `attached_at` and moving `current_checkpoint_id` happen in
        // one transaction: a checkpoint the sandbox points at is always marked
        // attached, which is what makes the garbage collector safe.
        let mut tx = self.pool.begin().await.map_err(storage_error)?;

        let current: Option<(i64,)> =
            sqlx::query_as("SELECT generation FROM sandboxes WHERE id = $1 FOR UPDATE")
                .bind(sandbox_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(storage_error)?;

        let Some((current_generation,)) = current else {
            let _ = tx.rollback().await;
            return Err(SandboxCheckpointError::Storage(format!(
                "sandbox {sandbox_id} not found"
            )));
        };

        if current_generation != generation {
            let _ = tx.rollback().await;
            return Err(SandboxCheckpointError::StaleGeneration {
                sandbox_id,
                current: current_generation,
                carried: generation,
            });
        }

        sqlx::query(
            r#"
            UPDATE sandbox_checkpoints
            SET attached_at = COALESCE(attached_at, NOW())
            WHERE id = $1 AND sandbox_id = $2
            "#,
        )
        .bind(checkpoint_id)
        .bind(sandbox_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;

        sqlx::query(
            r#"
            UPDATE sandboxes
            SET current_checkpoint_id = $1, updated_at = NOW()
            WHERE id = $2
            "#,
        )
        .bind(checkpoint_id)
        .bind(sandbox_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;

        tx.commit().await.map_err(storage_error)?;
        Ok(())
    }

    async fn current_checkpoint(
        &self,
        sandbox_id: Uuid,
    ) -> Result<Option<SandboxCheckpoint>, SandboxCheckpointError> {
        let row: Option<CheckpointRow> = sqlx::query_as(
            r#"
            SELECT c.id, c.sandbox_id, c.generation, c.source_turn_id,
                   c.source_tool_call_id, c.kind, c.provider_ref,
                   c.workspace_revision, c.attached_at, c.created_at
            FROM sandboxes s
            JOIN sandbox_checkpoints c ON c.id = s.current_checkpoint_id
            WHERE s.id = $1
            "#,
        )
        .bind(sandbox_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(storage_error)?;

        row.map(SandboxCheckpoint::try_from).transpose()
    }

    async fn rollback_current_checkpoint(
        &self,
        sandbox_id: Uuid,
        checkpoint_id: Uuid,
        generation: i64,
    ) -> Result<Option<SandboxCheckpoint>, SandboxCheckpointError> {
        // Same transaction shape as `attach_checkpoint`, run backwards: detach
        // the rejected revision and move the pointer, so the sandbox never
        // points at a row whose `attached_at` is NULL.
        let mut tx = self.pool.begin().await.map_err(storage_error)?;

        let current: Option<(i64, Option<Uuid>)> = sqlx::query_as(
            "SELECT generation, current_checkpoint_id FROM sandboxes WHERE id = $1 FOR UPDATE",
        )
        .bind(sandbox_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;

        let Some((current_generation, current_checkpoint_id)) = current else {
            let _ = tx.rollback().await;
            return Err(SandboxCheckpointError::Storage(format!(
                "sandbox {sandbox_id} not found"
            )));
        };

        if current_generation != generation {
            let _ = tx.rollback().await;
            return Err(SandboxCheckpointError::StaleGeneration {
                sandbox_id,
                current: current_generation,
                carried: generation,
            });
        }

        // The pointer moved while this reconciliation was deciding. Whatever
        // moved it is newer than this decision, so leave it alone and report
        // where the sandbox actually sits.
        if current_checkpoint_id != Some(checkpoint_id) {
            let _ = tx.rollback().await;
            return self.current_checkpoint(sandbox_id).await;
        }

        // Ordered by `attached_at` rather than `created_at`: an out-of-order
        // upload that was attached later is still the more recent commit.
        let previous: Option<CheckpointRow> = sqlx::query_as(
            r#"
            SELECT id, sandbox_id, generation, source_turn_id, source_tool_call_id,
                   kind, provider_ref, workspace_revision, attached_at, created_at
            FROM sandbox_checkpoints
            WHERE sandbox_id = $1
              AND id <> $2
              AND attached_at IS NOT NULL
            ORDER BY attached_at DESC, created_at DESC
            LIMIT 1
            "#,
        )
        .bind(sandbox_id)
        .bind(checkpoint_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(storage_error)?;

        sqlx::query(
            r#"
            UPDATE sandboxes
            SET current_checkpoint_id = $1, updated_at = NOW()
            WHERE id = $2
            "#,
        )
        .bind(previous.as_ref().map(|row| row.id))
        .bind(sandbox_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;

        // Detach after the pointer has moved, so `ON DELETE RESTRICT` never sees
        // an unattached row that is still the current checkpoint.
        sqlx::query(
            r#"
            UPDATE sandbox_checkpoints
            SET attached_at = NULL
            WHERE id = $1 AND sandbox_id = $2
            "#,
        )
        .bind(checkpoint_id)
        .bind(sandbox_id)
        .execute(&mut *tx)
        .await
        .map_err(storage_error)?;

        tx.commit().await.map_err(storage_error)?;

        previous.map(SandboxCheckpoint::try_from).transpose()
    }

    async fn collect_unattached_checkpoints(
        &self,
        sandbox_id: Uuid,
        before: DateTime<Utc>,
        limit: i64,
    ) -> Result<Vec<String>, SandboxCheckpointError> {
        // THREAT[TM-DOS]: clamp before the delete so caller input can never
        // request an unbounded destructive batch.
        let limit = limit.clamp(0, MAX_CHECKPOINT_COLLECT_LIMIT);

        // The `attached_at IS NULL` filter is the safety property: an
        // authoritative revision is always attached, so it can never be selected
        // here. The `ON DELETE RESTRICT` foreign key on
        // `sandboxes.current_checkpoint_id` backstops that at the database.
        let rows: Vec<(String,)> = sqlx::query_as(
            r#"
            DELETE FROM sandbox_checkpoints
            WHERE id IN (
                SELECT id
                FROM sandbox_checkpoints
                WHERE sandbox_id = $1
                  AND attached_at IS NULL
                  AND created_at < $2
                ORDER BY created_at
                LIMIT $3
            )
            RETURNING workspace_revision
            "#,
        )
        .bind(sandbox_id)
        .bind(before)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map_err(storage_error)?;

        Ok(rows.into_iter().map(|(revision,)| revision).collect())
    }
}
