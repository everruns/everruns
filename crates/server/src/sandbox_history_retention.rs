// Sandbox history retention background job.
//
// Decision: deleted Sandboxes stay visible on the Sandboxes page as history
// (migration 175), then are purged with their instances, checkpoints and
// lifecycle log once they have been deleted longer than
// SANDBOX_HISTORY_RETENTION_DAYS (default 30, 0 keeps them forever).

use sqlx::PgPool;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{error, info};

const DEFAULT_RETENTION_DAYS: u64 = 30;

/// Read SANDBOX_HISTORY_RETENTION_DAYS from env.
pub fn retention_days_from_env() -> u64 {
    std::env::var("SANDBOX_HISTORY_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

/// Spawn the hourly purge of long-deleted Sandboxes.
pub fn spawn_retention_task(pool: PgPool, retention_days: u64) -> Option<JoinHandle<()>> {
    if retention_days == 0 {
        info!("Sandbox history retention disabled (SANDBOX_HISTORY_RETENTION_DAYS=0)");
        return None;
    }
    Some(tokio::spawn(async move {
        let mut ticker = tokio::time::interval(Duration::from_secs(3600));
        loop {
            ticker.tick().await;
            match purge_deleted_sandboxes(&pool, retention_days).await {
                Ok(0) => {}
                Ok(purged) => info!(purged, retention_days, "Purged deleted Sandbox history"),
                Err(e) => error!("Sandbox history retention failed: {e}"),
            }
        }
    }))
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
