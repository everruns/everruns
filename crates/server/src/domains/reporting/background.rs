use std::sync::Arc;
use std::time::Duration;

use crate::domains::reporting::ReportingService;
use crate::storage::StorageBackend;

const DEFAULT_PROJECTOR_INTERVAL_SECS: u64 = 5;
const DEFAULT_PROJECTOR_LIMIT: i64 = 500;
const DEFAULT_BACKFILL_INTERVAL_SECS: u64 = 300;
const DEFAULT_BACKFILL_LIMIT: i64 = 1_000;

#[derive(Debug, Clone)]
struct ReportingBackgroundConfig {
    projector_interval: Duration,
    projector_limit: i64,
    backfill_interval: Duration,
    backfill_limit: i64,
}

impl ReportingBackgroundConfig {
    fn from_env() -> Self {
        Self {
            projector_interval: Duration::from_secs(env_u64(
                "REPORTING_PROJECTOR_INTERVAL_SECS",
                DEFAULT_PROJECTOR_INTERVAL_SECS,
            )),
            projector_limit: env_i64("REPORTING_PROJECTOR_LIMIT", DEFAULT_PROJECTOR_LIMIT),
            backfill_interval: Duration::from_secs(env_u64(
                "REPORTING_BACKFILL_INTERVAL_SECS",
                DEFAULT_BACKFILL_INTERVAL_SECS,
            )),
            backfill_limit: env_i64("REPORTING_BACKFILL_LIMIT", DEFAULT_BACKFILL_LIMIT),
        }
    }
}

pub fn spawn_reporting_background_task(db: Arc<StorageBackend>) {
    if !env_bool("REPORTING_BACKGROUND_ENABLED", true) {
        tracing::info!("Reporting background task disabled");
        return;
    }

    if !matches!(db.as_ref(), StorageBackend::Postgres(_)) {
        tracing::debug!("Reporting background task disabled for non-Postgres storage");
        return;
    }

    let config = ReportingBackgroundConfig::from_env();
    if config.projector_interval.is_zero() && config.backfill_interval.is_zero() {
        tracing::info!("Reporting background task disabled by zero intervals");
        return;
    }

    if !config.projector_interval.is_zero() {
        let service = ReportingService::new(db.clone());
        let interval_secs = config.projector_interval.as_secs();
        let limit = config.projector_limit;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            tracing::info!(
                interval_secs,
                limit,
                "Started reporting projector background task"
            );

            loop {
                interval.tick().await;
                match service.run_projector_due(limit).await {
                    Ok(result) if result.claimed > 0 => {
                        tracing::info!(
                            claimed = result.claimed,
                            completed = result.completed,
                            failed = result.failed,
                            "Reporting projector run completed"
                        );
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "Reporting projector run failed");
                    }
                }
            }
        });
    }

    if !config.backfill_interval.is_zero() {
        let service = ReportingService::new(db);
        let interval_secs = config.backfill_interval.as_secs();
        let limit = config.backfill_limit;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            tracing::info!(
                interval_secs,
                limit,
                "Started reporting backfill background task"
            );

            loop {
                interval.tick().await;
                match service.backfill_missing(None, limit).await {
                    Ok(result) if result.enqueued > 0 => {
                        tracing::info!(
                            enqueued = result.enqueued,
                            events = result.events,
                            sessions = result.sessions,
                            llm_generations = result.llm_generations,
                            usage_ledger = result.usage_ledger,
                            "Reporting backfill enqueued missing projection work"
                        );
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "Reporting backfill failed");
                    }
                }
            }
        });
    }
}

fn env_bool(name: &str, default: bool) -> bool {
    std::env::var(name)
        .ok()
        .and_then(|value| match value.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Some(true),
            "0" | "false" | "no" | "off" => Some(false),
            _ => None,
        })
        .unwrap_or(default)
}

fn env_u64(name: &str, default: u64) -> u64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn env_i64(name: &str, default: i64) -> i64 {
    std::env::var(name)
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}
