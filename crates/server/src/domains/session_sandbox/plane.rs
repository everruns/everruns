//! Pausing, resuming and deleting a v2 session's managed sandbox (data
//! plane v2, step 11.6).
//!
//! Decisions:
//! - The control plane does it, with the v1 lifecycle code: it pinned the
//!   sandbox at create (`data_plane/sandboxes.rs`) and holds the provider
//!   account. What it starts from is the session's cell: the agent's sandbox
//!   keeps its state there as a reserved session secret, sealed with the
//!   org's data key, and the control plane reads it, opens it, and writes
//!   the new state back sealed. The executor's next sandbox call reads that
//!   state, so a paused sandbox is resumed and a deleted one is replaced and
//!   restored from the session's workspace.
//! - The config is the pinned sandbox's, the one the data plane runs, not
//!   the agent's current capabilities.
//! - No hosted state store is installed here: it would adopt the cell's
//!   state into the control plane's tables and take it away from the
//!   executor.
//! - Idle pause: the control plane's sweep (`background::sandbox_idle_sweep`)
//!   pauses the sandbox of a v2 session whose last turn ended longer ago
//!   than the sandbox's idle limit, from the turn summaries the data plane
//!   feeds back. The sandbox's row remembers the activity it was paused
//!   after (`last_activity_at`, unused otherwise for a v2 sandbox), so it
//!   is paused once per idle spell. A turn that runs past the limit may
//!   find its sandbox paused; its next sandbox call resumes it, as on v1.
//!   An idle action of `delete` pauses here: retention deletes come later.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_capabilities::session_sandbox::{
    SessionSandboxConfig, session_sandbox_config_from_capabilities,
};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::session_sandbox::SESSION_SANDBOX_SECRET_NAME;
use everruns_contracts::typed_id::SessionId;
use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};

use crate::domains::common::CommandError;
use crate::domains::sandbox_templates::record::SandboxTargetKind;
use crate::domains::sandbox_templates::resolution::capability_for_sandbox;
use crate::domains::session_storage::plane::PlaneStorage;
use crate::storage::StorageBackend;

/// v2 sessions whose managed sandbox may still run past its idle limit
/// (plus `grace`): their org and the activity they went idle after.
pub async fn idle_on_data_planes(
    pool: &sqlx::PgPool,
    grace: std::time::Duration,
    limit: i64,
) -> sqlx::Result<Vec<(uuid::Uuid, i64, chrono::DateTime<chrono::Utc>)>> {
    sqlx::query_as(
        r#"
        SELECT s.session_id, s.org_id, d.last_activity_at
        FROM sandboxes s
        JOIN dp_session_summaries d
          ON d.session_id = 'session_' || replace(s.session_id::text, '-', '')
        WHERE s.deleted_at IS NULL
          AND s.role = 'primary'
          AND COALESCE(s.profile_snapshot->'target'->>'kind', '') = 'managed'
          AND COALESCE(s.profile_snapshot->'lifecycle'->>'idle_action', '') <> 'keep_running'
          AND (s.last_activity_at IS NULL OR s.last_activity_at < d.last_activity_at)
          AND d.last_activity_at < now() - make_interval(secs => COALESCE(
                (s.profile_snapshot->'lifecycle'->>'idle_after_seconds')::double precision,
                180) + $1)
        ORDER BY d.last_activity_at
        LIMIT $2
        "#,
    )
    .bind(grace.as_secs_f64())
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Remember that `session`'s sandbox was handled for the idle spell after
/// `activity`.
pub async fn idle_handled(
    pool: &sqlx::PgPool,
    session: uuid::Uuid,
    activity: chrono::DateTime<chrono::Utc>,
) -> sqlx::Result<()> {
    sqlx::query(
        "UPDATE sandboxes SET last_activity_at = $2
         WHERE session_id = $1 AND role = 'primary' AND deleted_at IS NULL",
    )
    .bind(session)
    .bind(activity)
    .execute(pool)
    .await
    .map(|_| ())
}

/// Pause `session`'s sandbox on its data plane when its idle pause is on;
/// whether a running one was paused.
pub async fn pause_idle(
    service: &super::SessionSandboxService,
    org_id: i64,
    session: SessionId,
) -> anyhow::Result<bool> {
    let Some(storage) = PlaneStorage::on(&service.db, org_id, session).await? else {
        return Ok(false);
    };
    let Some(config) = config(&service.db, session).await? else {
        return Ok(false);
    };
    if !config.idle_pause_enabled {
        return Ok(false);
    }
    let context = service
        .plane_tool_context(session, PlaneSandboxStorage::new(storage))
        .await;
    let state = everruns_capabilities::session_sandbox::pause_session_sandbox(&context, &config)
        .await
        .map_err(|err| anyhow::anyhow!("{err:?}"))?;
    Ok(state.is_some())
}

/// The managed sandbox pinned for `session`, or `None` when it has none.
pub async fn config(
    db: &StorageBackend,
    session: SessionId,
) -> anyhow::Result<Option<SessionSandboxConfig>> {
    let Some(pinned) = db.get_primary_sandbox(session).await? else {
        return Ok(None);
    };
    if pinned.spec.target.kind != SandboxTargetKind::Managed {
        return Ok(None);
    }
    session_sandbox_config_from_capabilities(&[capability_for_sandbox(&pinned.spec)])
        .map_err(|err| anyhow::anyhow!(err))
}

/// A v2 session's storage as the sandbox lifecycle sees it: only the
/// sandbox's own state is read back.
pub struct PlaneSandboxStorage {
    storage: PlaneStorage,
}

impl PlaneSandboxStorage {
    pub fn new(storage: PlaneStorage) -> Arc<Self> {
        Arc::new(Self { storage })
    }

    fn scope(&self, session: SessionId) -> Result<()> {
        if session == self.storage.plane().session {
            Ok(())
        } else {
            Err(AgentLoopError::store("another session's storage"))
        }
    }
}

fn store_err(error: CommandError) -> AgentLoopError {
    AgentLoopError::store(error.to_string())
}

fn info_epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::<chrono::Utc>::UNIX_EPOCH
}

#[async_trait]
impl SessionStorageStore for PlaneSandboxStorage {
    async fn set_value(&self, session: SessionId, key: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        self.storage.set_value(key, value).await.map_err(store_err)
    }

    async fn get_value(&self, session: SessionId, key: &str) -> Result<Option<String>> {
        self.scope(session)?;
        self.storage.value(key).await.map_err(store_err)
    }

    async fn delete_value(&self, session: SessionId, key: &str) -> Result<bool> {
        self.scope(session)?;
        self.storage.delete_value(key).await.map_err(store_err)
    }

    async fn list_keys(&self, session: SessionId) -> Result<Vec<KeyInfo>> {
        self.scope(session)?;
        let listed = self.storage.listed().await.map_err(store_err)?;
        Ok(listed
            .values
            .into_keys()
            .map(|key| KeyInfo {
                key,
                created_at: info_epoch(),
                updated_at: info_epoch(),
            })
            .collect())
    }

    async fn set_secret(&self, session: SessionId, name: &str, value: &str) -> Result<()> {
        self.scope(session)?;
        self.storage
            .set_secret(name, value)
            .await
            .map_err(store_err)
    }

    /// THREAT[TM-TENANT]: the sandbox's state is the one secret a cell
    /// hands back, and only to the control plane, which sealed the org's
    /// key; a person's or agent's secrets are never read here.
    async fn get_secret(&self, session: SessionId, name: &str) -> Result<Option<String>> {
        self.scope(session)?;
        if name != SESSION_SANDBOX_SECRET_NAME {
            return Err(AgentLoopError::store(
                "only the sandbox's state is read from a data plane",
            ));
        }
        let mut secrets = self.storage.secrets().await.map_err(store_err)?;
        Ok(secrets.remove(name))
    }

    async fn delete_secret(&self, session: SessionId, name: &str) -> Result<bool> {
        self.scope(session)?;
        self.storage.delete_secret(name).await.map_err(store_err)
    }

    async fn list_secrets(&self, session: SessionId) -> Result<Vec<SecretInfo>> {
        self.scope(session)?;
        let listed = self.storage.listed().await.map_err(store_err)?;
        Ok(listed
            .secrets
            .into_iter()
            .map(|name| SecretInfo {
                name,
                created_at: info_epoch(),
                updated_at: info_epoch(),
            })
            .collect())
    }
}
