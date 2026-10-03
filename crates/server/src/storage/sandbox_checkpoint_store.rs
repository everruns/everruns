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
use everruns_platform::sandbox_state::{
    SandboxStateError, SandboxStateStore, validate_sandbox_state,
};
use everruns_platform::session_sandbox::{
    SessionSandboxInstance, SessionSandboxState, SessionSandboxStatus,
};
use everruns_provider::typed_id::SessionId;
use sqlx::PgPool;
use uuid::Uuid;

/// PostgreSQL-backed logical sandbox and checkpoint store.
#[derive(Clone)]
pub struct PgSandboxCheckpointStore {
    pool: PgPool,
}

/// Durable logical Environment projection. Physical provider state remains in
/// `sandbox_instances`; this record survives instance loss and replacement.
#[derive(Debug, Clone, PartialEq)]
pub struct EnvironmentRecord {
    pub id: Uuid,
    pub session_id: SessionId,
    pub provider: String,
    pub profile_name: String,
    pub profile: everruns_platform::ResolvedEnvironmentProfile,
    pub desired_state: String,
    pub observed_state: String,
    pub generation: i64,
    pub current_checkpoint_id: Option<Uuid>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

impl PgSandboxCheckpointStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Pin the fully resolved Agent/session profile to its logical Environment.
    /// Idempotent only for the same snapshot; a Session can never switch target.
    pub async fn pin_environment(
        &self,
        session_id: SessionId,
        profile_name: &str,
        profile: &everruns_platform::ResolvedEnvironmentProfile,
    ) -> Result<EnvironmentRecord, SandboxStateError> {
        let provider = profile
            .target
            .provider
            .clone()
            .unwrap_or_else(|| profile.target.kind.as_str().to_string());
        let snapshot = serde_json::to_value(profile).map_err(state_storage_error)?;
        let inserted: Option<Uuid> = sqlx::query_scalar(
            r#"
            INSERT INTO sandboxes
                (org_id, session_id, provider, profile_name, profile_snapshot)
            SELECT s.org_id, s.id, $2, $3, $4
            FROM sessions s
            WHERE s.id = $1
            ON CONFLICT (session_id) WHERE profile_snapshot IS NOT NULL DO UPDATE SET
                provider = EXCLUDED.provider,
                profile_name = EXCLUDED.profile_name,
                profile_snapshot = EXCLUDED.profile_snapshot,
                updated_at = NOW()
            WHERE sandboxes.profile_snapshot IS NULL
               OR (sandboxes.profile_name = EXCLUDED.profile_name
                   AND sandboxes.profile_snapshot = EXCLUDED.profile_snapshot
                   AND sandboxes.provider = EXCLUDED.provider)
            RETURNING sandboxes.id
            "#,
        )
        .bind(session_id)
        .bind(&provider)
        .bind(profile_name)
        .bind(&snapshot)
        .fetch_optional(&self.pool)
        .await
        .map_err(state_storage_error)?;

        if inserted.is_none() {
            return Err(SandboxStateError::Storage(
                "session environment is already pinned to a different profile".to_string(),
            ));
        }
        self.get_environment(session_id)
            .await?
            .ok_or_else(|| SandboxStateError::Storage("pinned environment disappeared".to_string()))
    }

    pub async fn get_environment(
        &self,
        session_id: SessionId,
    ) -> Result<Option<EnvironmentRecord>, SandboxStateError> {
        #[derive(sqlx::FromRow)]
        struct Row {
            id: Uuid,
            session_id: SessionId,
            provider: String,
            profile_name: Option<String>,
            profile_snapshot: Option<serde_json::Value>,
            desired_state: String,
            observed_state: String,
            generation: i64,
            current_checkpoint_id: Option<Uuid>,
            last_activity_at: Option<DateTime<Utc>>,
        }

        let row: Option<Row> = sqlx::query_as(
            r#"
            SELECT id, session_id, provider, profile_name, profile_snapshot,
                   desired_state, observed_state, generation,
                   current_checkpoint_id, last_activity_at
            FROM sandboxes
            WHERE session_id = $1
              AND profile_snapshot IS NOT NULL
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(state_storage_error)?;

        row.map(|row| {
            let snapshot = row.profile_snapshot.ok_or_else(|| {
                SandboxStateError::Storage(
                    "logical environment has no pinned profile snapshot".to_string(),
                )
            })?;
            let profile = serde_json::from_value(snapshot).map_err(state_storage_error)?;
            Ok(EnvironmentRecord {
                id: row.id,
                session_id: row.session_id,
                provider: row.provider,
                profile_name: row.profile_name.unwrap_or_else(|| "legacy".to_string()),
                profile,
                desired_state: row.desired_state,
                observed_state: row.observed_state,
                generation: row.generation,
                current_checkpoint_id: row.current_checkpoint_id,
                last_activity_at: row.last_activity_at,
            })
        })
        .transpose()
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

#[derive(sqlx::FromRow)]
struct SandboxStateRow {
    sandbox_id: Uuid,
    generation: i64,
    provider: String,
    status: String,
    external_id: String,
    display_name: Option<String>,
    workspace_path: Option<String>,
    provider_state: serde_json::Value,
    metadata: serde_json::Value,
    init_completed_at: Option<DateTime<Utc>>,
    last_init_error: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

fn storage_error(error: impl std::fmt::Display) -> SandboxCheckpointError {
    SandboxCheckpointError::Storage(error.to_string())
}

fn state_storage_error(error: impl std::fmt::Display) -> SandboxStateError {
    SandboxStateError::Storage(error.to_string())
}

fn parse_status(raw: &str) -> Result<SessionSandboxStatus, SandboxStateError> {
    match raw {
        "running" => Ok(SessionSandboxStatus::Running),
        "paused" => Ok(SessionSandboxStatus::Paused),
        "lost" => Ok(SessionSandboxStatus::Lost),
        other => Err(SandboxStateError::Storage(format!(
            "unknown sandbox instance status '{other}'"
        ))),
    }
}

fn status_str(status: SessionSandboxStatus) -> &'static str {
    match status {
        SessionSandboxStatus::Running => "running",
        SessionSandboxStatus::Paused => "paused",
        SessionSandboxStatus::Lost => "lost",
    }
}

fn observed_state_str(status: SessionSandboxStatus) -> &'static str {
    match status {
        SessionSandboxStatus::Running => "ready",
        SessionSandboxStatus::Paused => "paused",
        SessionSandboxStatus::Lost => "lost",
    }
}

fn parse_timestamp(raw: &str, field: &str) -> Result<DateTime<Utc>, SandboxStateError> {
    DateTime::parse_from_rfc3339(raw)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|error| SandboxStateError::Storage(format!("invalid {field}: {error}")))
}

impl TryFrom<SandboxStateRow> for SessionSandboxState {
    type Error = SandboxStateError;

    fn try_from(row: SandboxStateRow) -> Result<Self, Self::Error> {
        Ok(Self {
            sandbox: Some(SandboxRef {
                id: row.sandbox_id,
                generation: row.generation,
            }),
            provider: row.provider,
            status: parse_status(&row.status)?,
            instance: SessionSandboxInstance {
                external_id: row.external_id,
                display_name: row.display_name,
                workspace_path: row.workspace_path,
                provider_state: row.provider_state,
                metadata: row.metadata,
            },
            init_completed_at: row.init_completed_at.map(|value| value.to_rfc3339()),
            last_init_error: row.last_init_error,
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
        })
    }
}

#[async_trait]
impl SandboxStateStore for PgSandboxCheckpointStore {
    async fn load_current_state(
        &self,
        session_id: SessionId,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
        let row: Option<SandboxStateRow> = sqlx::query_as(
            r#"
            SELECT s.id AS sandbox_id, s.generation, s.provider,
                   i.status, i.external_id, i.display_name,
                   i.workspace_path, i.provider_state, i.metadata,
                   i.init_completed_at, i.last_init_error,
                   s.created_at, i.updated_at
            FROM sandboxes s
            JOIN sandbox_instances i ON i.id = s.current_instance_id
            WHERE s.session_id = $1
            ORDER BY s.created_at
            LIMIT 1
            "#,
        )
        .bind(session_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(state_storage_error)?;

        row.map(SessionSandboxState::try_from).transpose()
    }

    async fn load_state(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<SessionSandboxState>, SandboxStateError> {
        let row: Option<SandboxStateRow> = sqlx::query_as(
            r#"
            SELECT s.id AS sandbox_id, s.generation, s.provider,
                   i.status, i.external_id, i.display_name,
                   i.workspace_path, i.provider_state, i.metadata,
                   i.init_completed_at, i.last_init_error,
                   s.created_at, i.updated_at
            FROM sandboxes s
            JOIN sandbox_instances i ON i.id = s.current_instance_id
            WHERE s.session_id = $1 AND s.provider = $2
            "#,
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&self.pool)
        .await
        .map_err(state_storage_error)?;

        row.map(SessionSandboxState::try_from).transpose()
    }

    async fn save_state(
        &self,
        session_id: SessionId,
        state: &SessionSandboxState,
        expected: Option<&SandboxRef>,
    ) -> Result<SandboxRef, SandboxStateError> {
        validate_sandbox_state(state)?;
        let created_at = parse_timestamp(&state.created_at, "sandbox created_at")?;
        let updated_at = parse_timestamp(&state.updated_at, "sandbox updated_at")?;
        let init_completed_at = state
            .init_completed_at
            .as_deref()
            .map(|value| parse_timestamp(value, "sandbox init_completed_at"))
            .transpose()?;

        let mut tx = self.pool.begin().await.map_err(state_storage_error)?;
        let logical: Option<(Uuid, i64, Option<Uuid>)> = sqlx::query_as(
            r#"
            SELECT id, generation, current_instance_id
            FROM sandboxes
            WHERE session_id = $1 AND provider = $2
            FOR UPDATE
            "#,
        )
        .bind(session_id)
        .bind(&state.provider)
        .fetch_optional(&mut *tx)
        .await
        .map_err(state_storage_error)?;

        let (sandbox_id, mut generation, current_instance_id) = match logical {
            Some(row) => row,
            None => sqlx::query_as(
                r#"
                INSERT INTO sandboxes
                    (org_id, session_id, provider, created_at, updated_at)
                SELECT s.org_id, s.id, $2, $3, $4
                FROM sessions s
                WHERE s.id = $1
                RETURNING id, generation, current_instance_id
                "#,
            )
            .bind(session_id)
            .bind(&state.provider)
            .bind(created_at)
            .bind(updated_at)
            .fetch_one(&mut *tx)
            .await
            .map_err(state_storage_error)?,
        };

        if let Some(expected) = expected
            && expected.id != sandbox_id
        {
            return Err(SandboxStateError::WrongSandbox {
                current: sandbox_id,
                carried: expected.id,
            });
        }
        if let Some(expected) = expected
            && expected.generation != generation
        {
            return Err(SandboxStateError::StaleGeneration {
                sandbox_id,
                current: generation,
                carried: expected.generation,
            });
        }
        if expected.is_none() && current_instance_id.is_some() {
            return Err(SandboxStateError::StaleGeneration {
                sandbox_id,
                current: generation,
                carried: 0,
            });
        }

        let current_external_id: Option<String> = match current_instance_id {
            Some(instance_id) => {
                sqlx::query_scalar("SELECT external_id FROM sandbox_instances WHERE id = $1")
                    .bind(instance_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(state_storage_error)?
            }
            None => None,
        };

        let instance_id = if current_external_id.as_deref()
            == Some(state.instance.external_id.as_str())
        {
            let instance_id = current_instance_id.ok_or_else(|| {
                SandboxStateError::Storage(
                    "sandbox current incarnation disappeared while locked".to_string(),
                )
            })?;
            sqlx::query(
                r#"
                UPDATE sandbox_instances
                SET display_name = $2, workspace_path = $3,
                    provider_state = $4, metadata = $5, status = $6,
                    init_completed_at = $7, last_init_error = $8,
                    updated_at = $9
                WHERE id = $1
                "#,
            )
            .bind(instance_id)
            .bind(state.instance.display_name.as_deref())
            .bind(state.instance.workspace_path.as_deref())
            .bind(&state.instance.provider_state)
            .bind(&state.instance.metadata)
            .bind(status_str(state.status))
            .bind(init_completed_at)
            .bind(state.last_init_error.as_deref())
            .bind(updated_at)
            .execute(&mut *tx)
            .await
            .map_err(state_storage_error)?;
            instance_id
        } else {
            if let Some(instance_id) = current_instance_id {
                sqlx::query(
                    "UPDATE sandbox_instances SET retired_at = NOW(), updated_at = NOW() WHERE id = $1",
                )
                .bind(instance_id)
                .execute(&mut *tx)
                .await
                .map_err(state_storage_error)?;
                generation += 1;
            }

            sqlx::query_scalar(
                r#"
                INSERT INTO sandbox_instances
                    (sandbox_id, generation, external_id, display_name,
                     workspace_path, provider_state, metadata, status,
                     init_completed_at, last_init_error, created_at, updated_at)
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                RETURNING id
                "#,
            )
            .bind(sandbox_id)
            .bind(generation)
            .bind(&state.instance.external_id)
            .bind(state.instance.display_name.as_deref())
            .bind(state.instance.workspace_path.as_deref())
            .bind(&state.instance.provider_state)
            .bind(&state.instance.metadata)
            .bind(status_str(state.status))
            .bind(init_completed_at)
            .bind(state.last_init_error.as_deref())
            .bind(updated_at)
            .bind(updated_at)
            .fetch_one(&mut *tx)
            .await
            .map_err(state_storage_error)?
        };

        sqlx::query(
            r#"
            UPDATE sandboxes
            SET generation = $2, current_instance_id = $3,
                desired_state = CASE
                    WHEN $4 = 'ready' THEN 'ready'
                    WHEN $4 = 'paused' THEN 'paused'
                    ELSE desired_state
                END,
                observed_state = $4, last_activity_at = $5, updated_at = $5
            WHERE id = $1
            "#,
        )
        .bind(sandbox_id)
        .bind(generation)
        .bind(instance_id)
        .bind(observed_state_str(state.status))
        .bind(updated_at)
        .execute(&mut *tx)
        .await
        .map_err(state_storage_error)?;

        tx.commit().await.map_err(state_storage_error)?;
        Ok(SandboxRef {
            id: sandbox_id,
            generation,
        })
    }

    async fn delete_state(
        &self,
        session_id: SessionId,
        provider: &str,
        expected: Option<&SandboxRef>,
    ) -> Result<bool, SandboxStateError> {
        let mut tx = self.pool.begin().await.map_err(state_storage_error)?;
        let current: Option<(Uuid, i64, Option<Uuid>, bool)> = sqlx::query_as(
            "SELECT id, generation, current_instance_id, profile_snapshot IS NOT NULL FROM sandboxes WHERE session_id = $1 AND provider = $2 FOR UPDATE",
        )
        .bind(session_id)
        .bind(provider)
        .fetch_optional(&mut *tx)
        .await
        .map_err(state_storage_error)?;
        let Some((sandbox_id, generation, current_instance_id, is_profiled)) = current else {
            tx.commit().await.map_err(state_storage_error)?;
            return Ok(false);
        };
        if let Some(expected) = expected
            && expected.id != sandbox_id
        {
            return Err(SandboxStateError::WrongSandbox {
                current: sandbox_id,
                carried: expected.id,
            });
        }
        if let Some(expected) = expected
            && expected.generation != generation
        {
            return Err(SandboxStateError::StaleGeneration {
                sandbox_id,
                current: generation,
                carried: expected.generation,
            });
        }
        if expected.is_none() {
            return Err(SandboxStateError::StaleGeneration {
                sandbox_id,
                current: generation,
                carried: 0,
            });
        }
        if let Some(instance_id) = current_instance_id {
            sqlx::query(
                "UPDATE sandbox_instances SET retired_at = NOW(), updated_at = NOW() WHERE id = $1",
            )
            .bind(instance_id)
            .execute(&mut *tx)
            .await
            .map_err(state_storage_error)?;
        }
        let deleted = if is_profiled {
            sqlx::query(
                "UPDATE sandboxes SET current_instance_id = NULL, desired_state = 'deleted', observed_state = 'deleted', updated_at = NOW() WHERE id = $1",
            )
            .bind(sandbox_id)
            .execute(&mut *tx)
            .await
            .map_err(state_storage_error)?;
            true
        } else {
            sqlx::query("DELETE FROM sandboxes WHERE id = $1")
                .bind(sandbox_id)
                .execute(&mut *tx)
                .await
                .map_err(state_storage_error)?
                .rows_affected()
                > 0
        };
        tx.commit().await.map_err(state_storage_error)?;
        Ok(deleted)
    }
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
