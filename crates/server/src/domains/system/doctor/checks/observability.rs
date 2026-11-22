use super::Findings;
use crate::domains::system::doctor::{
    CheckArea, CheckStatus, DoctorCheck, SystemDoctor, last_metrics_scrape,
};
use crate::domains::system::monitoring_detail::{self, DetailConfig};
use chrono::{DateTime, Utc};
use std::time::Duration;

const AREA: CheckArea = CheckArea::Observability;

/// The timeline sampler records a minute every minute; ten without one is a
/// stopped sampler, not a slow one.
const HISTORY_SILENCE: chrono::TimeDelta = chrono::TimeDelta::minutes(10);

/// Monitoring's minute history in PostgreSQL.
fn history_check(
    (oldest, newest): (Option<DateTime<Utc>>, Option<DateTime<Utc>>),
    now: DateTime<Utc>,
) -> DoctorCheck {
    let check = DoctorCheck::new(
        AREA,
        "observability.monitoring_history",
        "Monitoring history",
    );
    match (oldest, newest) {
        (Some(_), Some(newest)) if now - newest > HISTORY_SILENCE => check
            .warning(format!(
                "Monitoring history stopped at {}.",
                newest.format("%Y-%m-%d %H:%M UTC")
            ))
            .why("The Timeline shows a gap from then on, so load and errors go unrecorded.")
            .fix("Check the monitoring-timeline background job in the server logs."),
        (Some(oldest), Some(_)) => check.ok(format!(
            "Recorded every minute since {}, kept permanently.",
            oldest.format("%Y-%m-%d")
        )),
        _ => check.info(
            "No Monitoring history yet: the server records one row a minute once it has run a minute.",
        ),
    }
}

async fn detail_probe(config: &DetailConfig) -> Option<Result<(), String>> {
    match config {
        DetailConfig::Prometheus { url } => Some(monitoring_detail::probe(url).await),
        _ => None,
    }
}

/// The optional 10-second detail store.
fn detail_check(config: &DetailConfig, probe: Option<Result<(), String>>) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "observability.monitoring_detail", "Monitoring detail");
    let host = config.display_host().unwrap_or_default();
    match (config, probe) {
        (DetailConfig::Off, _) => check.info(
            "Monitoring keeps one-minute history in PostgreSQL. To add 10-second shard detail, \
             set MONITORING_DETAIL=prometheus and MONITORING_DETAIL_URL to VictoriaMetrics or \
             Prometheus.",
        ),
        (DetailConfig::Invalid(why), _) => check
            .warning(format!("Monitoring detail is misconfigured: {why}."))
            .why("The Shards tab shows minute history only.")
            .fix("Set MONITORING_DETAIL=prometheus with an http(s) MONITORING_DETAIL_URL, or unset both."),
        (DetailConfig::Prometheus { .. }, Some(Ok(()))) => {
            check.ok(format!("Reads 10-second shard detail from {host}."))
        }
        (DetailConfig::Prometheus { .. }, probe) => check
            .warning(format!(
                "{host} did not answer: {}.",
                probe.and_then(Result::err).unwrap_or_else(|| "not probed".into())
            ))
            .why("The Shards tab shows minute history only, without per-node detail.")
            .fix("Check that the store is up and that MONITORING_DETAIL_URL points at its query API."),
    }
}

/// Prometheus scrapes every 15 to 60 seconds; five minutes of silence means
/// nothing is scraping.
const SCRAPE_SILENCE: Duration = Duration::from_secs(5 * 60);

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let mut found = Findings::default();
    let env = &doctor.env;

    let metrics = DoctorCheck::new(AREA, "observability.metrics", "Metrics");
    let metrics = if !env.metrics_enabled {
        metrics.info("Metrics are turned off (METRICS_ENABLED=false).")
    } else if !env.metrics_exposed {
        metrics.info(
            "Metrics are collected but not exposed, so nothing can watch them. Set \
                 METRICS_ADDR to serve /metrics on an internal port.",
        )
    } else {
        match last_metrics_scrape() {
            Some(at) => {
                let ago = (Utc::now() - at).to_std().unwrap_or_default();
                if ago > SCRAPE_SILENCE {
                    metrics
                        .warning(format!(
                            "Nothing has read /metrics for {} minutes.",
                            ago.as_secs() / 60
                        ))
                        .why("If Prometheus stopped scraping, alerts built on it stay silent.")
                        .fix("Check the Prometheus target for this server.")
                } else {
                    metrics.ok(format!(
                        "/metrics was read {} seconds ago.",
                        ago.as_secs()
                    ))
                }
            }
            None if doctor.uptime() > SCRAPE_SILENCE => metrics
                .warning("Metrics are exposed, but nothing has read /metrics since the server started.")
                .why("Nobody is watching: slowdowns and errors go unnoticed until someone complains.")
                .fix("Add this server as a Prometheus scrape target."),
            None => metrics.info("Metrics are exposed; nothing has read them yet since the server started."),
        }
    };
    if metrics.status == CheckStatus::Ok {
        found.fact("Prometheus metrics");
    }
    found.check(metrics);

    let traces = DoctorCheck::new(AREA, "observability.traces", "Traces");
    let exporting = env.otlp_endpoint && !env.otel_disabled;
    if exporting {
        found.fact("OpenTelemetry traces");
    }
    found.check(if exporting {
        traces.ok("Traces are exported over OpenTelemetry.")
    } else {
        traces.info(
            "Traces are not exported. Set OTEL_EXPORTER_OTLP_ENDPOINT to send them to \
             your tracing backend.",
        )
    });

    let (extent, shards, probe) = tokio::join!(
        doctor.db.monitoring_history_extent(),
        doctor.db.monitoring_shards_latest(),
        detail_probe(DetailConfig::from_env()),
    );
    let now = Utc::now();
    if let Ok(extent) = extent {
        found.check(history_check(extent, now));
    }
    if let Ok(shards) = shards {
        let reporting = shards
            .iter()
            .filter(|c| now - c.minute <= chrono::TimeDelta::minutes(3))
            .count();
        if reporting > 0 {
            found.fact(super::plural(
                reporting,
                "shard reporting",
                "shards reporting",
            ));
        }
    }
    found.check(detail_check(DetailConfig::from_env(), probe));

    found.check(detection_check(doctor));
    found.check(classifier_check(doctor));
    found.check(inspector_check(doctor));

    let logs = DoctorCheck::new(AREA, "observability.log_level", "Log level");
    let verbose = env
        .rust_log
        .as_deref()
        .is_some_and(|filter| filter.contains("debug") || filter.contains("trace"));
    found.check(if verbose && env.strict() {
        logs.warning("Debug logging is on.")
            .why("Logs grow fast, cost more to keep, and may hold more detail than intended.")
            .fix("Set RUST_LOG=info, or limit debug to the module you are investigating.")
    } else {
        logs.ok(format!(
            "Logging at `{}`.",
            env.rust_log.as_deref().unwrap_or("info")
        ))
    });
    found
}

/// The built-in detectors' sweep runs every 15 minutes on each server; this
/// long without one means detection stopped.
const DETECTION_SILENCE: Duration = Duration::from_secs(40 * 60);

/// Whether the built-in detectors (runaway schedules, tool loops, spend
/// spikes) are still sweeping on this server.
fn detection_check(doctor: &SystemDoctor) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "observability.detection", "Built-in detection");
    if doctor.preflight {
        return check.info("Detection runs inside the server; a command-line run cannot see it.");
    }
    match crate::domains::health_issues::behavior::last_sweep() {
        Some(sweep) => {
            let ago = (Utc::now() - sweep.at).to_std().unwrap_or_default();
            if let Some(error) = sweep.error {
                check
                    .warning(format!("The last detection sweep failed: {error}."))
                    .why("Runaway sessions and tool loops go unreported until it works again.")
                    .fix(
                        "Check the server log for \"health sweep failed\" and the database's load.",
                    )
            } else if ago > DETECTION_SILENCE {
                check
                    .warning(format!(
                        "No detection sweep has finished for {} minutes.",
                        ago.as_secs() / 60
                    ))
                    .why("Runaway sessions and tool loops go unreported while it is stuck.")
                    .fix("Restart the server; check its log for the sweep's errors.")
            } else {
                check.ok(format!(
                    "Detection last swept {} minutes ago.",
                    ago.as_secs() / 60
                ))
            }
        }
        None if doctor.uptime() > DETECTION_SILENCE => check
            .warning("No detection sweep has finished since the server started.")
            .why("Runaway sessions and tool loops go unreported.")
            .fix("Check the server log for \"health sweep failed\"."),
        None => check.info("Detection has not swept yet since the server started."),
    }
}

/// Whether the built-in session classifiers are reading sessions on this
/// server, and how many sessions are suspected of abusing the platform.
fn classifier_check(doctor: &SystemDoctor) -> DoctorCheck {
    use crate::domains::health_issues::classifiers::last_sweep;
    let check = DoctorCheck::new(AREA, "observability.classifiers", "Session classifiers");
    if doctor.preflight {
        return check
            .info("Classifiers run inside the server; a command-line run cannot see them.");
    }
    let Some(sweep) = last_sweep() else {
        return check.info("The session classifiers have not swept yet since the server started.");
    };
    if sweep.open_abuse > 0 {
        return check
            .warning(format!(
                "{} sessions are suspected of abusing the platform.",
                sweep.open_abuse
            ))
            .why("A decision model judged them likely spam, scraping, credential attacks or similar.")
            .fix(
                "Review each one from its organization's Health page. Nothing is stopped \
                 automatically; stop or suspend only after a person confirms.",
            );
    }
    if sweep.failed > 0 && sweep.classified == 0 {
        return check
            .warning(format!(
                "The last classifier sweep could not read any session: {}.",
                sweep.last_error.unwrap_or_default()
            ))
            .why("Abuse, stuck agents and injected instructions go unreported until it works.")
            .fix("Check the decision service's configuration and the server log.");
    }
    if sweep.classified == 0 && sweep.unavailable > 0 {
        return check
            .info("No decision model answers session classification for the sessions read.")
            .fix("Configure the deployment's decision service to turn the classifiers on.");
    }
    check.ok(format!(
        "The last sweep classified {} sessions.",
        sweep.classified
    ))
}

/// Whether the Inspector is writing findings up on this server, and how many
/// abuse detections across the install wait for a person.
fn inspector_check(doctor: &SystemDoctor) -> DoctorCheck {
    use crate::domains::health_issues::inspector::last_sweep;
    let check = DoctorCheck::new(AREA, "observability.inspector", "Finding write-ups");
    if doctor.preflight {
        return check
            .info("The Inspector runs inside the server; a command-line run cannot see it.");
    }
    let Some(sweep) = last_sweep() else {
        return check.info("The Inspector has not swept yet since the server started.");
    };
    if sweep.open_platform > 0 {
        return check
            .warning(format!(
                "{} abuse detections across the install wait for a person.",
                sweep.open_platform
            ))
            .why("Each is a lead, written up by the Inspector, that a session may be abusing the platform.")
            .fix(
                "Review them with `everruns system detections` (GET /v1/system/detections). \
                 Nothing is suspended automatically on this evidence.",
            );
    }
    if sweep.failed > 0 && sweep.written == 0 {
        return check
            .warning(format!(
                "The last Inspector sweep could not write any finding up: {}.",
                sweep.last_error.unwrap_or_default()
            ))
            .why("Findings still notify, but without an account of what happened.")
            .fix("Check the organizations' default models and the server log.");
    }
    check.ok(format!(
        "The last sweep wrote up {} findings.",
        sweep.written
    ))
}

#[cfg(test)]
mod monitoring_tests {
    use super::*;
    use crate::domains::system::doctor::CheckStatus;

    #[test]
    fn history_is_ok_while_minutes_arrive_and_warns_when_they_stop() {
        let now = Utc::now();
        let day = chrono::TimeDelta::days(1);
        assert_eq!(history_check((None, None), now).status, CheckStatus::Info);
        let live = history_check((Some(now - day), Some(now)), now);
        assert_eq!(live.status, CheckStatus::Ok);
        let stopped = history_check((Some(now - day), Some(now - HISTORY_SILENCE * 2)), now);
        assert_eq!(stopped.status, CheckStatus::Warning);
    }

    #[test]
    fn detail_is_optional_and_a_dead_store_warns() {
        assert_eq!(
            detail_check(&DetailConfig::Off, None).status,
            CheckStatus::Info
        );
        let bad = DetailConfig::Invalid("MONITORING_DETAIL_URL is not set".into());
        assert_eq!(detail_check(&bad, None).status, CheckStatus::Warning);
        let vm = DetailConfig::Prometheus {
            url: "http://vm:8428".into(),
        };
        assert_eq!(detail_check(&vm, Some(Ok(()))).status, CheckStatus::Ok);
        let down = detail_check(&vm, Some(Err("connection refused".into())));
        assert_eq!(down.status, CheckStatus::Warning);
        assert!(down.summary.contains("http://vm:8428"), "{}", down.summary);
    }
}
