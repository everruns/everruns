//! Ownership history is diagnostic data, retained for one day in bounded batches.
use super::cluster_jobs::ClusterJob;
use sqlx::PgPool;
use std::time::Duration;

pub fn retention_job(pool: PgPool) -> ClusterJob {
    ClusterJob::every(
        "actor-history-retention",
        "actor_history_retention",
        "Prunes actor ownership history older than one day.",
        Duration::from_secs(60),
        move || {
            let pool = pool.clone();
            Box::pin(async move {
                for _ in 0..20 {
                    match sqlx::query(
                    "DELETE FROM session_actor_lease_history WHERE id IN \
                     (SELECT id FROM session_actor_lease_history \
                      WHERE occurred_at < now() - interval '1 day' ORDER BY occurred_at LIMIT 10000)",
                    ).execute(&pool).await {
                        Ok(result) if result.rows_affected() < 10000 => break,
                        Ok(_) => {},
                        Err(error) => {
                            tracing::error!(%error, "Actor ownership history retention failed");
                            break;
                        }
                    }
                }
            })
        },
    )
}
