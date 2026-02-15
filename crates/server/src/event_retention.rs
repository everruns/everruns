// Event retention background job
// Decision: Archive events older than EVENT_RETENTION_DAYS to archived_events table
// Decision: Uses session variable app.archival_bypass to bypass append-only trigger
// Decision: Processes in batches to avoid long-running transactions

use sqlx::PgPool;
use std::time::Duration;
use tracing::{debug, error, info};

/// Default retention period in days (0 = disabled)
const DEFAULT_RETENTION_DAYS: u64 = 0;

/// Max events to archive per batch
const BATCH_SIZE: i64 = 1000;

/// Read EVENT_RETENTION_DAYS from env. Returns 0 (disabled) if unset.
pub fn retention_days_from_env() -> u64 {
    std::env::var("EVENT_RETENTION_DAYS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_RETENTION_DAYS)
}

/// Spawn the event retention background task.
/// Runs every hour and archives events older than the configured retention period.
pub fn spawn_retention_task(pool: PgPool, retention_days: u64) {
    if retention_days == 0 {
        info!("Event retention disabled (EVENT_RETENTION_DAYS=0 or unset)");
        return;
    }

    info!(
        retention_days = retention_days,
        "Starting event retention background task"
    );

    tokio::spawn(async move {
        let interval = Duration::from_secs(3600); // 1 hour
        let mut ticker = tokio::time::interval(interval);
        ticker.tick().await; // skip first immediate tick

        loop {
            ticker.tick().await;
            match archive_old_events(&pool, retention_days).await {
                Ok(count) => {
                    if count > 0 {
                        info!(
                            archived = count,
                            retention_days = retention_days,
                            "Archived old events"
                        );
                    } else {
                        debug!("No events to archive");
                    }
                }
                Err(e) => {
                    error!("Event retention job failed: {}", e);
                }
            }
        }
    });
}

/// Archive events older than `retention_days` from `events` to `archived_events`.
/// Returns the total number of events archived.
async fn archive_old_events(pool: &PgPool, retention_days: u64) -> Result<i64, sqlx::Error> {
    let cutoff = chrono::Utc::now() - chrono::Duration::days(retention_days as i64);
    let mut total_archived: i64 = 0;

    loop {
        let mut tx = pool.begin().await?;

        // Copy batch to archived_events
        let inserted = sqlx::query_scalar::<_, i64>(
            "WITH batch AS (
                SELECT id, session_id, sequence, event_type, data, ts, context, metadata, tags, created_at
                FROM events
                WHERE created_at < $1
                ORDER BY created_at
                LIMIT $2
            ),
            inserted AS (
                INSERT INTO archived_events (id, session_id, sequence, event_type, data, ts, context, metadata, tags, created_at)
                SELECT id, session_id, sequence, event_type, data, ts, context, metadata, tags, created_at
                FROM batch
                ON CONFLICT (id) DO NOTHING
                RETURNING 1
            )
            SELECT COUNT(*)::bigint FROM inserted",
        )
        .bind(cutoff)
        .bind(BATCH_SIZE)
        .fetch_one(&mut *tx)
        .await?;

        if inserted == 0 {
            tx.rollback().await?;
            break;
        }

        // Enable archival bypass and delete the archived events
        sqlx::query("SET LOCAL app.archival_bypass = 'true'")
            .execute(&mut *tx)
            .await?;

        let deleted = sqlx::query_scalar::<_, i64>(
            "WITH batch AS (
                SELECT id FROM events
                WHERE created_at < $1
                ORDER BY created_at
                LIMIT $2
            )
            DELETE FROM events WHERE id IN (SELECT id FROM batch)
            RETURNING 1",
        )
        .bind(cutoff)
        .bind(BATCH_SIZE)
        .fetch_all(&mut *tx)
        .await?;

        tx.commit().await?;

        let batch_count = deleted.len() as i64;
        total_archived += batch_count;

        if batch_count < BATCH_SIZE {
            break;
        }

        // Brief pause between batches to reduce lock contention
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    Ok(total_archived)
}

// Compile-time sanity checks
const _: () = {
    assert!(DEFAULT_RETENTION_DAYS == 0, "default should be disabled");
    assert!(
        BATCH_SIZE > 0 && BATCH_SIZE <= 10000,
        "batch size out of range"
    );
};
