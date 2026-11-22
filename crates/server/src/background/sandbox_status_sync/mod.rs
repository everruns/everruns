//! Bounded, read-only provider polling for the hosted Sandbox fleet.
//!
//! Runtime state remains on its owning plane. Only the control-plane fleet's
//! observation changes, fenced against concurrent lifecycle writes. Polling
//! has a separate clock so it never postpones idle pause or retention.

use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use everruns_contracts::session_sandbox::{SessionSandboxStatus, SessionSandboxStatusResponse};
use everruns_contracts::typed_id::SessionId;
use futures::{StreamExt, stream};
use sqlx::PgPool;
use uuid::Uuid;

use crate::background::cluster_jobs::ClusterJob;
use crate::domains::session_sandbox::SessionSandboxService;

pub const SANDBOX_STATUS_SYNC_SCHEDULE: &str = "sandbox-status-sync";
pub const SANDBOX_STATUS_SYNC_ACTIVITY: &str = "sandbox_status_sync";
const PERIOD: Duration = Duration::from_secs(60);
const CHECK_TIMEOUT: Duration = Duration::from_secs(15);
// Three waves of four bounded reads leave headroom before the next minute.
const BATCH: i64 = 12;
const CONCURRENCY: usize = 4;

pub fn sync_job(pool: PgPool, service: Option<Arc<SessionSandboxService>>) -> ClusterJob {
    let Some(service) = service else {
        return ClusterJob::disabled(SANDBOX_STATUS_SYNC_SCHEDULE, SANDBOX_STATUS_SYNC_ACTIVITY);
    };
    ClusterJob::every(
        SANDBOX_STATUS_SYNC_SCHEDULE,
        SANDBOX_STATUS_SYNC_ACTIVITY,
        "Refreshes Sandbox fleet state from providers without starting or stopping compute.",
        PERIOD,
        move || {
            let pool = pool.clone();
            let service = service.clone();
            Box::pin(async move {
                if let Err(error) = sync(&pool, &service).await {
                    tracing::warn!(%error, "Sandbox provider status sync failed");
                }
            })
        },
    )
}

#[derive(Debug, sqlx::FromRow)]
struct StatusCandidate {
    id: Uuid,
    session_id: SessionId,
    org_id: i64,
    generation: i64,
    updated_at: DateTime<Utc>,
}

/// Oldest checks first, including failures, so one provider cannot starve the
/// rest. A newly created managed Sandbox is discovered even before it has a
/// control-plane instance, as happens with a shard-owned session.
async fn candidates(pool: &PgPool, limit: i64) -> sqlx::Result<Vec<StatusCandidate>> {
    sqlx::query_as(
        r#"
        SELECT s.id, s.session_id, s.org_id, s.generation, s.updated_at
        FROM sandboxes s
        JOIN sessions sess ON sess.id = s.session_id AND sess.deleted_at IS NULL
        WHERE s.deleted_at IS NULL AND s.role = 'primary'
          AND s.profile_snapshot->'target'->>'kind' = 'managed'
          AND s.observed_state NOT IN ('deleted', 'deleting')
          AND (s.provider_status_checked_at IS NULL
               OR s.provider_status_checked_at < now() - interval '5 minutes')
        ORDER BY s.provider_status_checked_at ASC NULLS FIRST, s.id
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

async fn sync(pool: &PgPool, service: &SessionSandboxService) -> anyhow::Result<()> {
    let pending = candidates(pool, BATCH).await?;
    stream::iter(pending)
        .for_each_concurrent(CONCURRENCY, |candidate| async move {
            let observation = tokio::time::timeout(
                CHECK_TIMEOUT,
                service.provider_status(candidate.org_id, candidate.session_id),
            )
            .await;
            let status = match observation {
                Ok(Ok(status)) => status,
                Ok(Err(_)) | Err(_) => {
                    // Provider errors can contain credentials. The operator's
                    // explicit diagnostic read provides scrubbed details.
                    tracing::debug!(sandbox_id = %candidate.id, "Sandbox provider status unavailable; retaining recorded state");
                    None
                }
            };
            if let Err(error) = record_observation(pool, &candidate, status).await {
                tracing::warn!(sandbox_id = %candidate.id, %error, "Sandbox provider status save failed");
            }
        })
        .await;
    Ok(())
}

/// Update the fleet projection, preserving runtime configuration, desired
/// state, instances and activity. A lifecycle write during the provider read
/// wins. Failed reads advance only the polling clock.
async fn record_observation(
    pool: &PgPool,
    candidate: &StatusCandidate,
    status: Option<SessionSandboxStatusResponse>,
) -> sqlx::Result<bool> {
    let observed = status.as_ref().map(|status| match status.session_status {
        SessionSandboxStatus::Running => "ready",
        SessionSandboxStatus::Paused => "paused",
        SessionSandboxStatus::Lost => "lost",
    });
    let result = sqlx::query(
        r#"
        UPDATE sandboxes
        SET provider_status_checked_at = now(),
            provider_status_observed_at = CASE WHEN $5::text IS NOT NULL
                THEN now() ELSE provider_status_observed_at END,
            provider_status_observed_state = COALESCE($5, provider_status_observed_state),
            provider_status_external_id = COALESCE($6, provider_status_external_id)
        WHERE id = $1 AND org_id = $2 AND generation = $3
          AND updated_at = $4 AND deleted_at IS NULL
          AND observed_state NOT IN ('deleted', 'deleting')
        "#,
    )
    .bind(candidate.id)
    .bind(candidate.org_id)
    .bind(candidate.generation)
    .bind(candidate.updated_at)
    .bind(observed)
    .bind(status.as_ref().map(|status| status.external_id.as_str()))
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests;
