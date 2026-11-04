//! Monitoring → Timeline recording (knowledge/operations/monitoring.md).
//!
//! Decision: a cluster-once job every minute takes up to five population
//! samples on 10-second boundaries, folding each into its minute, and
//! recloses the last few finished minutes so late events and reconciled model
//! costs land. A durable schedule firing every 10 seconds would create a
//! durable task per sample; one run per minute keeps that to 1,440 a day.
//! Short peaks between samples are missed; the turn counters are exact.
//!
//! A second job backfills earlier history a week at a time from turn records,
//! newest first, until it reaches the first committed turn.

use super::cluster_jobs::ClusterJob;
use crate::storage::{StorageBackend, TimelineWorkerSample};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use everruns_durable::persistence::WORKER_HEARTBEAT_TIMEOUT_SECS;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, warn};

pub const MONITORING_TIMELINE_SCHEDULE: &str = "monitoring-timeline";
pub const MONITORING_TIMELINE_ACTIVITY: &str = "monitoring_timeline";
pub const MONITORING_TIMELINE_BACKFILL_SCHEDULE: &str = "monitoring-timeline-backfill";
pub const MONITORING_TIMELINE_BACKFILL_ACTIVITY: &str = "monitoring_timeline_backfill";

const SAMPLE_EVERY_SECS: i64 = 10;
/// Samples per run; the run ends before the next one fires.
const SAMPLES_PER_RUN: usize = 5;
/// Finished minutes reclosed on every run.
const RECLOSE_MINUTES: i64 = 3;
/// History rebuilt per backfill run.
const BACKFILL_DAYS_PER_RUN: i64 = 7;

pub fn sampler_job(db: Arc<StorageBackend>) -> ClusterJob {
    ClusterJob::every(
        MONITORING_TIMELINE_SCHEDULE,
        MONITORING_TIMELINE_ACTIVITY,
        "Records parallel sessions, turns and worker load per minute for Monitoring → Timeline.",
        Duration::from_secs(60),
        move || {
            let db = db.clone();
            Box::pin(async move {
                let now = Utc::now();
                if let Err(error) = db
                    .timeline_close_minutes(now - ChronoDuration::minutes(RECLOSE_MINUTES), now)
                    .await
                {
                    warn!(%error, "Monitoring timeline: closing minutes failed");
                }
                for i in 0..SAMPLES_PER_RUN {
                    if i > 0 {
                        tokio::time::sleep(until_next_boundary(Utc::now())).await;
                    }
                    if let Err(error) = sample(&db, Utc::now()).await {
                        warn!(%error, "Monitoring timeline: sample failed");
                    }
                }
            })
        },
    )
}

pub fn backfill_job(db: Arc<StorageBackend>) -> ClusterJob {
    ClusterJob::every(
        MONITORING_TIMELINE_BACKFILL_SCHEDULE,
        MONITORING_TIMELINE_BACKFILL_ACTIVITY,
        "Rebuilds Monitoring → Timeline history recorded before sampling began from turn records.",
        Duration::from_secs(300),
        move || {
            let db = db.clone();
            Box::pin(async move {
                match backfill_step(&db).await {
                    Ok(Some(day)) => debug!(%day, "Monitoring timeline backfilled"),
                    Ok(None) => {}
                    Err(error) => warn!(%error, "Monitoring timeline backfill failed"),
                }
            })
        },
    )
}

/// Take and record one sample.
pub async fn sample(db: &StorageBackend, now: DateTime<Utc>) -> anyhow::Result<()> {
    let orgs = db.timeline_sample(now).await?;
    let fresh = now - ChronoDuration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);
    let workers: Vec<TimelineWorkerSample> = db
        .monitoring_workers()
        .await?
        .into_iter()
        // The server's own cluster-job pool registers as a worker too; it runs
        // no sessions, so its slots are not execution capacity.
        .filter(|w| {
            w.last_heartbeat_at >= fresh && w.worker_group.as_deref() != Some("server-jobs")
        })
        .map(|w| TimelineWorkerSample {
            id: w.id,
            load: w.current_load,
            capacity: w.max_concurrency,
        })
        .collect();
    db.timeline_record_sample(now, &orgs, Some(&workers)).await
}

/// Backfill up to a week before the earliest recorded minute. Returns the
/// start of what was rebuilt, `None` when there is nothing left to do.
pub async fn backfill_step(db: &StorageBackend) -> anyhow::Result<Option<DateTime<Utc>>> {
    let Some(earliest) = db.timeline_earliest_minute().await? else {
        return Ok(None); // Sampling has not started yet.
    };
    let Some(first_turn) = db.timeline_earliest_turn().await? else {
        return Ok(None);
    };
    if earliest <= first_turn {
        return Ok(None);
    }
    let from = (earliest - ChronoDuration::days(BACKFILL_DAYS_PER_RUN))
        .max(crate::storage::timeline_minute(first_turn));
    db.timeline_backfill(from, earliest).await?;
    Ok(Some(from))
}

fn until_next_boundary(now: DateTime<Utc>) -> Duration {
    let ms = now.timestamp_millis().rem_euclid(SAMPLE_EVERY_SECS * 1000);
    Duration::from_millis((SAMPLE_EVERY_SECS * 1000 - ms) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_for_the_next_ten_second_boundary() {
        let at = DateTime::parse_from_rfc3339("2026-11-01T06:00:03.250Z")
            .unwrap()
            .with_timezone(&Utc);
        assert_eq!(until_next_boundary(at), Duration::from_millis(6750));
    }

    #[test]
    fn a_run_finishes_before_the_next_one_fires() {
        assert!((SAMPLES_PER_RUN as i64 - 1) * SAMPLE_EVERY_SECS < 60);
    }
}
