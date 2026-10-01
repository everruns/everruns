//! Provider-side lifecycle of OpenAI Agents API sessions (EVE-1126).
//!
//! Everruns owns the record; OpenAI holds a session per Everruns session that
//! selected the backend. Deleting the Everruns session (directly, or by an
//! agent, harness, or organization cascade), replacing the provider session,
//! or releasing it for retention records a tombstone in the same transaction
//! (`agents_api_provider_deletions`, migration 156). This task:
//!
//! - **Retention.** With `AGENTS_API_SESSION_RETENTION_DAYS` set, releases
//!   provider sessions idle for that long; the next turn starts a new one
//!   from the Everruns record.
//! - **Deletion.** Claims due tombstones, resolves the owning provider's
//!   *current* key, and calls `DELETE /agents/sessions/{id}`. A missing
//!   session counts as deleted, so retries converge. A failure is retried
//!   with exponential backoff; a tombstone whose provider no longer exists,
//!   or that keeps failing, is kept as `failed` for operators.
//!
//! Tombstones carry no credentials and failures record a stable code, never a
//! provider response body. Claims are leases (`next_attempt_at`) taken with
//! `FOR UPDATE SKIP LOCKED`, so several server replicas share the queue.
//! Design: `knowledge/execution/openai-agents-api-runtime.md#session-lifecycle`.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use everruns_host::openai_agents_api::AgentsApiClient;
use everruns_host::openai_agents_api::backend::official_endpoint;
use everruns_host::openai_agents_api::lifecycle::{delete_provider_session, deletion_failure_code};
use everruns_provider::driver_registry::{DriverId, ProviderConfig};
use everruns_provider::runtime_provider::ProviderKey;
use sqlx::PgPool;
use tokio::task::JoinHandle;
use uuid::Uuid;

use crate::services::ProviderResolverService;
use crate::storage::PgAgentsApiStore;

/// How often the task runs.
const INTERVAL: Duration = Duration::from_secs(60);
/// Tombstones claimed per pass.
const BATCH: i64 = 50;
/// Checkpoints examined per retention pass.
const RETENTION_BATCH: i64 = 200;
/// A claimed tombstone is invisible to other replicas for this long.
const CLAIM_FOR: Duration = Duration::from_secs(300);
/// Attempts before a tombstone is marked failed (about a week of backoff).
pub const MAX_DELETE_ATTEMPTS: i32 = 40;
/// Longest wait between attempts.
const MAX_BACKOFF: Duration = Duration::from_secs(6 * 3600);

/// Configuration read from the environment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct AgentsApiLifecycleConfig {
    /// Release provider sessions idle this long; `None` keeps them until the
    /// Everruns session is deleted.
    pub retention: Option<Duration>,
}

impl AgentsApiLifecycleConfig {
    /// `AGENTS_API_SESSION_RETENTION_DAYS` (0 or unset: disabled).
    pub fn from_env() -> Self {
        Self::from_retention_days(
            std::env::var("AGENTS_API_SESSION_RETENTION_DAYS")
                .ok()
                .as_deref(),
        )
    }

    fn from_retention_days(raw: Option<&str>) -> Self {
        let days = raw
            .and_then(|value| value.trim().parse::<u64>().ok())
            .unwrap_or(0);
        Self {
            retention: (days > 0).then(|| Duration::from_secs(days * 86_400)),
        }
    }
}

/// One queued provider session deletion.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ProviderDeletionRow {
    pub id: Uuid,
    pub org_id: i64,
    pub session_id: Uuid,
    pub provider_key: Option<String>,
    pub provider_session_id: String,
    /// Attempts including the current one.
    pub attempts: i32,
}

/// Result of one deletion attempt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletionAttempt {
    /// The provider session is gone (now or earlier).
    Deleted,
    /// Try again later; the code says why.
    Retry(&'static str),
    /// No retry can succeed (the provider was deleted, the id is invalid).
    Abandon(&'static str),
}

/// Deletes one provider session with the owning provider's credentials.
#[async_trait]
pub trait ProviderSessionDeleter: Send + Sync {
    async fn delete(&self, row: &ProviderDeletionRow) -> DeletionAttempt;
}

/// Production deleter: resolves the provider's current key at delete time,
/// so a key rotated on the same Everruns provider still works.
pub struct ResolvingDeleter {
    resolver: Arc<ProviderResolverService>,
}

impl ResolvingDeleter {
    pub fn new(resolver: Arc<ProviderResolverService>) -> Self {
        Self { resolver }
    }
}

#[async_trait]
impl ProviderSessionDeleter for ResolvingDeleter {
    async fn delete(&self, row: &ProviderDeletionRow) -> DeletionAttempt {
        let Some(provider_key) = row.provider_key.as_deref() else {
            return DeletionAttempt::Abandon("provider_unknown");
        };
        let resolved = match self
            .resolver
            .resolve_runtime_provider_config(row.org_id, provider_key)
            .await
        {
            Ok(Some(resolved)) => resolved,
            // The provider (and its credentials) was deleted: nothing can
            // reach the session any more.
            Ok(None) => return DeletionAttempt::Abandon("provider_deleted"),
            Err(error) => {
                tracing::warn!(%error, "Agents API deletion: provider lookup failed");
                return DeletionAttempt::Retry("provider_lookup_failed");
            }
        };
        let mut config = ProviderConfig::for_provider(
            ProviderKey::new(provider_key),
            resolved
                .provider_type
                .to_ascii_lowercase()
                .parse()
                .unwrap_or(DriverId::OpenAI),
        );
        config.api_key = resolved.api_key;
        config.base_url = resolved.base_url;
        // THREAT[TM-LLM-044]: the key only ever goes to the official API, as
        // for turns; a provider re-pointed at a gateway waits for a fix.
        let Some(endpoint) = official_endpoint(&config) else {
            return DeletionAttempt::Retry("provider_credentials_unavailable");
        };
        match delete_provider_session(
            &AgentsApiClient::from_endpoint(endpoint),
            &row.provider_session_id,
        )
        .await
        {
            Ok(_) => DeletionAttempt::Deleted,
            Err(error) => match deletion_failure_code(&error) {
                (code, true) => DeletionAttempt::Retry(code),
                (code, false) => DeletionAttempt::Abandon(code),
            },
        }
    }
}

/// Backoff before attempt `attempts + 1`: 1, 2, 4 ... minutes, capped.
pub fn backoff(attempts: i32) -> Duration {
    let exponent = attempts.clamp(1, 20) as u32 - 1;
    Duration::from_secs(60u64.saturating_mul(1u64 << exponent)).min(MAX_BACKOFF)
}

/// Claim up to `limit` due tombstones; each claim counts as an attempt.
pub async fn claim_due_deletions(
    pool: &PgPool,
    limit: i64,
) -> sqlx::Result<Vec<ProviderDeletionRow>> {
    sqlx::query_as(
        r#"
        UPDATE agents_api_provider_deletions d
        SET attempts = d.attempts + 1,
            next_attempt_at = clock_timestamp() + make_interval(secs => $2),
            updated_at = clock_timestamp()
        WHERE d.id IN (
            SELECT id FROM agents_api_provider_deletions
            WHERE state = 'pending' AND next_attempt_at <= clock_timestamp()
            ORDER BY next_attempt_at
            LIMIT $1
            FOR UPDATE SKIP LOCKED
        )
        RETURNING d.id, d.org_id, d.session_id, d.provider_key, d.provider_session_id, d.attempts
        "#,
    )
    .bind(limit)
    .bind(CLAIM_FOR.as_secs_f64())
    .fetch_all(pool)
    .await
}

/// Record the outcome of one attempt.
pub async fn settle_deletion(
    pool: &PgPool,
    row: &ProviderDeletionRow,
    attempt: DeletionAttempt,
) -> sqlx::Result<()> {
    match attempt {
        DeletionAttempt::Deleted => {
            sqlx::query("DELETE FROM agents_api_provider_deletions WHERE id = $1")
                .bind(row.id)
                .execute(pool)
                .await?;
        }
        DeletionAttempt::Retry(code) if row.attempts < MAX_DELETE_ATTEMPTS => {
            sqlx::query(
                "UPDATE agents_api_provider_deletions SET last_error = $2, \
                 next_attempt_at = clock_timestamp() + make_interval(secs => $3), \
                 updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(row.id)
            .bind(code)
            .bind(backoff(row.attempts).as_secs_f64())
            .execute(pool)
            .await?;
        }
        DeletionAttempt::Retry(code) | DeletionAttempt::Abandon(code) => {
            tracing::error!(
                org_id = row.org_id,
                session_id = %row.session_id,
                code,
                attempts = row.attempts,
                "Agents API provider session could not be deleted; kept as failed"
            );
            sqlx::query(
                "UPDATE agents_api_provider_deletions SET state = 'failed', last_error = $2, \
                 updated_at = clock_timestamp() WHERE id = $1",
            )
            .bind(row.id)
            .bind(code)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// One deletion pass; returns how many provider sessions were deleted.
pub async fn drain_deletions(
    pool: &PgPool,
    deleter: &dyn ProviderSessionDeleter,
) -> sqlx::Result<usize> {
    let mut deleted = 0;
    for row in claim_due_deletions(pool, BATCH).await? {
        let attempt = deleter.delete(&row).await;
        if attempt == DeletionAttempt::Deleted {
            deleted += 1;
        }
        settle_deletion(pool, &row, attempt).await?;
    }
    Ok(deleted)
}

/// Start the lifecycle task under the server's supervisor. Both runtime modes
/// can drive the backend, so both run it; a deployment without PostgreSQL or
/// an encryption key cannot run the backend and gets no task.
pub(crate) fn track(
    supervisor: &mut crate::supervised_task::TaskSupervisor,
    ctx: &crate::app_builder::ServerContext,
) {
    let (Some(pool), Some(encryption)) = (ctx.db.background_pool(), ctx.encryption.as_ref()) else {
        return;
    };
    let resolver = ProviderResolverService::new(ctx.db.clone(), ctx.encryption.clone())
        .with_driver_registry((*ctx.driver_registry).clone());
    supervisor.track_optional(
        "agents_api_lifecycle",
        spawn_agents_api_lifecycle_task(
            pool.clone(),
            PgAgentsApiStore::new(pool.clone(), encryption.clone()),
            Arc::new(ResolvingDeleter::new(Arc::new(resolver))),
            AgentsApiLifecycleConfig::from_env(),
        ),
    );
}

/// Spawn the lifecycle task.
pub fn spawn_agents_api_lifecycle_task(
    pool: PgPool,
    store: PgAgentsApiStore,
    deleter: Arc<dyn ProviderSessionDeleter>,
    config: AgentsApiLifecycleConfig,
) -> Option<JoinHandle<()>> {
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            if let Some(retention) = config.retention {
                match store
                    .release_idle_provider_sessions(retention, RETENTION_BATCH)
                    .await
                {
                    Ok(0) => {}
                    Ok(released) => tracing::info!(
                        released,
                        "Agents API retention released idle provider sessions"
                    ),
                    Err(error) => tracing::warn!(%error, "Agents API retention pass failed"),
                }
            }
            match drain_deletions(&pool, deleter.as_ref()).await {
                Ok(0) => {}
                Ok(deleted) => tracing::info!(deleted, "Agents API provider sessions deleted"),
                Err(error) => tracing::warn!(%error, "Agents API deletion pass failed"),
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_from_a_minute_and_caps_at_six_hours() {
        assert_eq!(backoff(1), Duration::from_secs(60));
        assert_eq!(backoff(2), Duration::from_secs(120));
        assert_eq!(backoff(4), Duration::from_secs(480));
        assert_eq!(backoff(30), MAX_BACKOFF);
        assert_eq!(backoff(0), Duration::from_secs(60));
    }

    #[test]
    fn retention_is_off_unless_configured() {
        let parse = |raw| AgentsApiLifecycleConfig::from_retention_days(raw).retention;
        assert_eq!(parse(None), None);
        assert_eq!(parse(Some("0")), None);
        assert_eq!(parse(Some("soon")), None);
        assert_eq!(parse(Some(" 30 ")), Some(Duration::from_secs(30 * 86_400)));
    }
}
