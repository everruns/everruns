// Sandbox history retention background job.
//
// Decision: deleted Sandboxes stay visible on the Sandboxes page as history
// (migration 175), then are purged with their instances, checkpoints and
// lifecycle log once they have been deleted longer than
// SANDBOX_HISTORY_RETENTION_DAYS (default 30, 0 keeps them forever).

use crate::cluster_jobs::ClusterJob;
use sqlx::PgPool;
use std::time::Duration;
use tracing::{error, info};

const DEFAULT_RETENTION_DAYS: u64 = 30;

/// Read SANDBOX_HISTORY_RETENTION_DAYS from env.
pub fn retention_days_from_env() -> u64 {
    std::env::var("SANDBOX_HISTORY_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

/// Durable schedule name of the Sandbox history retention job.
pub const SANDBOX_HISTORY_RETENTION_SCHEDULE: &str = "sandbox-history-retention";
/// Activity type the Sandbox history retention schedule enqueues.
pub const SANDBOX_HISTORY_RETENTION_ACTIVITY: &str = "sandbox_history_retention";

/// The hourly purge of long-deleted Sandboxes as a cluster-once job
/// (`cluster_jobs.rs`). Disabled without a PostgreSQL pool or with a
/// retention of 0 days.
pub fn retention_job(pool: Option<PgPool>, retention_days: u64) -> ClusterJob {
    let disabled = ClusterJob::disabled(
        SANDBOX_HISTORY_RETENTION_SCHEDULE,
        SANDBOX_HISTORY_RETENTION_ACTIVITY,
    );
    if retention_days == 0 {
        info!("Sandbox history retention disabled (SANDBOX_HISTORY_RETENTION_DAYS=0)");
        return disabled;
    }
    let Some(pool) = pool else {
        return disabled;
    };
    ClusterJob::every(
        SANDBOX_HISTORY_RETENTION_SCHEDULE,
        SANDBOX_HISTORY_RETENTION_ACTIVITY,
        "Purges Sandboxes deleted longer than SANDBOX_HISTORY_RETENTION_DAYS.",
        Duration::from_secs(3600),
        move || {
            let pool = pool.clone();
            Box::pin(async move {
                match purge_deleted_sandboxes(&pool, retention_days).await {
                    Ok(0) => {}
                    Ok(purged) => info!(purged, retention_days, "Purged deleted Sandbox history"),
                    Err(e) => error!("Sandbox history retention failed: {e}"),
                }
            })
        },
    )
}

/// Delete Sandboxes deleted more than `retention_days` ago. Instances,
/// checkpoints and lifecycle rows cascade.
pub async fn purge_deleted_sandboxes(
    pool: &PgPool,
    retention_days: u64,
) -> Result<u64, sqlx::Error> {
    let days = i32::try_from(retention_days).unwrap_or(i32::MAX);
    let mut tx = pool.begin().await?;
    // The current-checkpoint pointer restricts deletes; clear it first.
    sqlx::query(
        "UPDATE sandboxes SET current_checkpoint_id = NULL \
         WHERE deleted_at < now() - make_interval(days => $1) AND current_checkpoint_id IS NOT NULL",
    )
    .bind(days)
    .execute(&mut *tx)
    .await?;
    let purged =
        sqlx::query("DELETE FROM sandboxes WHERE deleted_at < now() - make_interval(days => $1)")
            .bind(days)
            .execute(&mut *tx)
            .await?
            .rows_affected();
    tx.commit().await?;
    Ok(purged)
}
