use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;

use crate::domains::reporting::ReportingService;
use crate::storage::StorageBackend;

const DEFAULT_PROJECTOR_INTERVAL_SECS: u64 = 5;
const DEFAULT_PROJECTOR_LIMIT: i64 = 500;
const DEFAULT_REPAIR_INTERVAL_SECS: u64 = 300;
const DEFAULT_REPAIR_LIMIT: i64 = 1_000;

#[derive(Debug, Clone)]
struct ReportingBackgroundConfig {
    projector_interval: Duration,
    projector_limit: i64,
    repair_interval: Duration,
    repair_limit: i64,
}

impl ReportingBackgroundConfig {
    fn from_env() -> Self {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    fn from_lookup(mut lookup: impl FnMut(&str) -> Option<String>) -> Self {
        Self {
            projector_interval: Duration::from_secs(value_u64(
                lookup("REPORTING_PROJECTOR_INTERVAL_SECS"),
                DEFAULT_PROJECTOR_INTERVAL_SECS,
            )),
            projector_limit: value_i64(
                lookup("REPORTING_PROJECTOR_LIMIT"),
                DEFAULT_PROJECTOR_LIMIT,
            ),
            repair_interval: Duration::from_secs(value_u64(
                lookup("REPORTING_REPAIR_INTERVAL_SECS"),
                DEFAULT_REPAIR_INTERVAL_SECS,
            )),
            repair_limit: value_i64(lookup("REPORTING_REPAIR_LIMIT"), DEFAULT_REPAIR_LIMIT),
        }
    }
}

pub fn spawn_reporting_background_task(db: Arc<StorageBackend>) -> Vec<JoinHandle<()>> {
    if !env_bool("REPORTING_BACKGROUND_ENABLED", true) {
        tracing::info!("Reporting background task disabled");
        return Vec::new();
    }

    if !matches!(db.as_ref(), StorageBackend::Postgres(_)) {
        tracing::debug!("Reporting background task disabled for non-Postgres storage");
        return Vec::new();
    }

    let config = ReportingBackgroundConfig::from_env();
    if config.projector_interval.is_zero() && config.repair_interval.is_zero() {
        tracing::info!("Reporting background task disabled by zero intervals");
        return Vec::new();
    }

    let mut handles = Vec::new();
    if !config.projector_interval.is_zero() {
        let service = ReportingService::new(db.clone());
        let interval_secs = config.projector_interval.as_secs();
        let limit = config.projector_limit;
        handles.push(tokio::spawn(async move {
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
        }));
    }

    if !config.repair_interval.is_zero() {
        let service = ReportingService::new(db);
        let interval_secs = config.repair_interval.as_secs();
        let limit = config.repair_limit;
        handles.push(tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(interval_secs));
            tracing::info!(
                interval_secs,
                limit,
                "Started bounded reporting event repair background task"
            );

            loop {
                interval.tick().await;
                match service.repair_missing_event_projections(limit).await {
                    Ok(enqueued) if enqueued > 0 => {
                        tracing::info!(
                            enqueued,
                            "Reporting event repair enqueued missing projection work"
                        );
                    }
                    Ok(_) => {}
                    Err(error) => {
                        tracing::error!(%error, "Reporting event repair failed");
                    }
                }
            }
        }));
    }

    handles
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

fn value_u64(value: Option<String>, default: u64) -> u64 {
    value
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

fn value_i64(value: Option<String>, default: i64) -> i64 {
    value
        .and_then(|value| value.parse().ok())
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_schedule_bounded_repair_not_full_backfill() {
        let mut requested = Vec::new();
        let config = ReportingBackgroundConfig::from_lookup(|name| {
            requested.push(name.to_string());
            None
        });

        assert_eq!(config.projector_interval, Duration::from_secs(5));
        assert_eq!(config.projector_limit, 500);
        assert_eq!(config.repair_interval, Duration::from_secs(300));
        assert_eq!(config.repair_limit, 1_000);
        assert!(requested.contains(&"REPORTING_REPAIR_INTERVAL_SECS".to_string()));
        assert!(requested.contains(&"REPORTING_REPAIR_LIMIT".to_string()));
        assert!(!requested.iter().any(|name| name.contains("BACKFILL")));
    }

    #[test]
    fn bounded_repair_schedule_can_be_disabled() {
        let config = ReportingBackgroundConfig::from_lookup(|name| match name {
            "REPORTING_REPAIR_INTERVAL_SECS" => Some("0".to_string()),
            "REPORTING_REPAIR_LIMIT" => Some("42".to_string()),
            _ => None,
        });

        assert!(config.repair_interval.is_zero());
        assert_eq!(config.repair_limit, 42);
    }
}
