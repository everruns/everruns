use super::Findings;
use crate::domains::system::doctor::{
    CheckArea, CheckStatus, DoctorCheck, SystemDoctor, last_metrics_scrape,
};
use chrono::Utc;
use std::time::Duration;

const AREA: CheckArea = CheckArea::Observability;

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

    found.check(detection_check(doctor));

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
