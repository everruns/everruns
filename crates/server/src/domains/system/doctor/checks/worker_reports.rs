//! Checks on what workers report about themselves: the build they run, how
//! long a drain takes, the worker's own setup checks, and workers turned away
//! for a wrong token.

use super::plural;
use crate::domains::system::doctor::{CheckArea, CheckStatus, DoctorCheck};
use chrono::{DateTime, Duration, Utc};
use everruns_durable::WorkerInfo;
use everruns_internal_protocol::WORKER_PROTOCOL_VERSION;
use serde::Deserialize;
use std::collections::BTreeMap;

const AREA: CheckArea = CheckArea::Workers;

/// A drain longer than this is stuck: a worker's own shutdown grace is
/// seconds, and an operator drain is meant to be followed by a stop.
const STUCK_DRAIN: Duration = Duration::minutes(15);

/// Self-check reports older than this are ignored: a live worker sends one
/// every five minutes.
const STALE_REPORT: Duration = Duration::minutes(15);

/// A rejected worker call this recent means a worker is locked out now.
const RECENT_REJECTION: Duration = Duration::minutes(15);

/// Check id of the worker's NATS report (`everruns_worker::self_checks`).
const NATS_SELF_CHECK: &str = "nats";

pub(crate) fn versions_check(live: &[WorkerInfo], server_version: &str) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.versions", "Worker versions");
    if live.is_empty() {
        return check.info("No workers, so no versions to compare.");
    }
    let old_protocol: Vec<&str> = live
        .iter()
        .filter(|worker| {
            worker
                .protocol_version()
                .is_some_and(|version| version < WORKER_PROTOCOL_VERSION)
        })
        .map(|worker| worker.id.as_str())
        .collect();
    if !old_protocol.is_empty() {
        return check
            .problem(format!(
                "{} an older worker protocol than this server: {}.",
                plural(old_protocol.len(), "worker speaks", "workers speak"),
                old_protocol.join(", ")
            ))
            .why("Work handed to those workers can fail in ways that look random.")
            .fix(format!(
                "Upgrade those workers to {server_version}, the server's version."
            ));
    }
    let mut by_version: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for worker in live {
        let version = worker.version.as_deref().unwrap_or("unknown");
        by_version
            .entry(version)
            .or_default()
            .push(worker.id.as_str());
    }
    let behind: Vec<String> = by_version
        .iter()
        .filter(|(version, _)| **version != server_version)
        .map(|(version, ids)| match *version {
            "unknown" => format!(
                "{} (too old to say)",
                plural(ids.len(), "worker", "workers")
            ),
            version => format!("{} on {version}", plural(ids.len(), "worker", "workers")),
        })
        .collect();
    if behind.is_empty() {
        return check.ok(format!(
            "Every worker runs {server_version}, like the server."
        ));
    }
    check
        .warning(format!(
            "The server runs {server_version}; {}.",
            behind.join(", ")
        ))
        .why(
            "Expected for a few minutes during a deploy. Left like this, old workers may \
             not understand what newer servers ask of them.",
        )
        .fix("Finish the rollout, or restart the workers on the server's version.")
}

pub(crate) fn drain_check(draining: &[WorkerInfo], now: DateTime<Utc>) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.draining", "Draining workers");
    if draining.is_empty() {
        return check.ok("No worker is draining.");
    }
    let stuck: Vec<String> = draining
        .iter()
        .filter_map(|worker| {
            let since = worker.draining_since()?;
            (now - since >= STUCK_DRAIN)
                .then(|| format!("{} ({} min)", worker.id, (now - since).num_minutes()))
        })
        .collect();
    if stuck.is_empty() {
        return check.info(format!(
            "{} finishing current work and taking nothing new.",
            plural(draining.len(), "worker is", "workers are")
        ));
    }
    check
        .warning(format!(
            "{} draining for a long time: {}.",
            plural(stuck.len(), "worker has been", "workers have been"),
            stuck.join(", ")
        ))
        .why("A draining worker takes no new work, so it is capacity you pay for and cannot use.")
        .fix("Resume it on the Workers page, or stop it if it was meant to go away.")
}

pub(crate) fn rejected_logins_check(
    rejected: u64,
    last: Option<DateTime<Utc>>,
    strict: bool,
    now: DateTime<Utc>,
) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.rejected_logins", "Workers turned away");
    let fix = "Set WORKER_GRPC_AUTH_TOKEN to the same value on the server and every worker, \
               then restart the workers that log authentication errors.";
    match last {
        Some(at) if now - at < RECENT_REJECTION => check
            .problem_if(
                strict,
                format!(
                    "This server refused {} for a missing or wrong token, the last {} min ago.",
                    plural(rejected as usize, "worker call", "worker calls"),
                    (now - at).num_minutes()
                ),
            )
            .why("A worker with the wrong token cannot take any work.")
            .fix(fix),
        Some(at) => check
            .info(format!(
                "This server refused {} for a wrong token, none since {}.",
                plural(rejected as usize, "worker call", "worker calls"),
                at.format("%Y-%m-%d %H:%M UTC")
            ))
            .fix(fix),
        None => check.ok("No worker was turned away since this server started."),
    }
}

/// One finding from a worker's self-check report.
#[derive(Debug, Deserialize)]
struct SelfCheck {
    id: String,
    #[serde(default)]
    title: String,
    status: String,
    #[serde(default)]
    summary: String,
    #[serde(default)]
    fix: Option<String>,
}

fn status_of(raw: &str) -> CheckStatus {
    match raw {
        "ok" => CheckStatus::Ok,
        "warning" => CheckStatus::Warning,
        "problem" => CheckStatus::Problem,
        _ => CheckStatus::Info,
    }
}

/// One kind of self-check as each worker reported it, by worker id.
type Findings<'a> = Vec<(&'a str, SelfCheck)>;

/// One check per kind of self-check, merged across workers. `server_nats`
/// says whether this server delivers live updates through NATS.
pub(crate) fn self_report_checks(
    live: &[WorkerInfo],
    server_nats: bool,
    now: DateTime<Utc>,
) -> Vec<DoctorCheck> {
    let mut by_check: BTreeMap<String, (String, Findings<'_>)> = BTreeMap::new();
    for worker in live {
        let Some((report, at)) = worker.self_checks() else {
            continue;
        };
        if now - at > STALE_REPORT {
            continue;
        }
        let Ok(found) = serde_json::from_value::<Vec<SelfCheck>>(report.clone()) else {
            continue;
        };
        for finding in found {
            let entry = by_check
                .entry(finding.id.clone())
                .or_insert_with(|| (finding.title.clone(), Vec::new()));
            entry.1.push((worker.id.as_str(), finding));
        }
    }
    by_check
        .into_iter()
        .map(|(id, (title, findings))| {
            if id == NATS_SELF_CHECK {
                nats_check(&findings, server_nats)
            } else {
                merged_check(&id, &title, &findings)
            }
        })
        .collect()
}

fn merged_check(id: &str, title: &str, findings: &[(&str, SelfCheck)]) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, &format!("workers.self.{id}"), title);
    let statuses: Vec<CheckStatus> = findings
        .iter()
        .map(|(_, finding)| status_of(&finding.status))
        .collect();
    let worst = statuses
        .iter()
        .copied()
        .filter(|status| *status > CheckStatus::Info)
        .max();
    let Some(worst) = worst else {
        // All healthy, or all "not used": the first summary speaks for all.
        let summary = findings
            .first()
            .map(|(_, finding)| finding.summary.clone())
            .unwrap_or_default();
        return if statuses.contains(&CheckStatus::Ok) {
            check.ok(summary)
        } else {
            check.info(summary)
        };
    };
    let bad: Vec<&(&str, SelfCheck)> = findings
        .iter()
        .filter(|(_, finding)| status_of(&finding.status) == worst)
        .collect();
    let (_, first) = bad[0];
    let summary = if bad.len() == findings.len() && findings.len() == 1 {
        first.summary.clone()
    } else {
        let names: Vec<&str> = bad.iter().map(|(worker, _)| *worker).collect();
        format!(
            "{} of {} workers: {} ({})",
            bad.len(),
            findings.len(),
            first.summary.trim_end_matches('.'),
            names.join(", ")
        )
    };
    let check = match worst {
        CheckStatus::Problem => check.problem(summary),
        _ => check.warning(summary),
    };
    match &first.fix {
        Some(fix) => check.fix(fix.clone()),
        None => check,
    }
}

fn nats_check(findings: &[(&str, SelfCheck)], server_nats: bool) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.self.nats", "Live updates from workers");
    let (with, without): (Vec<_>, Vec<_>) = findings
        .iter()
        .partition(|(_, finding)| status_of(&finding.status) == CheckStatus::Ok);
    let names = |list: &[&(&str, SelfCheck)]| {
        list.iter()
            .map(|(worker, _)| *worker)
            .collect::<Vec<_>>()
            .join(", ")
    };
    match (server_nats, with.is_empty(), without.is_empty()) {
        (true, _, true) => check.ok("Workers and server both use NATS for live updates."),
        (false, true, _) => check.info("Neither workers nor server use NATS for live updates."),
        (true, _, false) => check
            .warning(format!(
                "The server uses NATS but {} not: {}.",
                plural(without.len(), "worker does", "workers do"),
                names(&without)
            ))
            .why("Those workers save every bit of streamed output through the server, so replies stream more slowly.")
            .fix("Set NATS_URL on those workers to the server's value."),
        (false, false, _) => check
            .problem(format!(
                "{} NATS_URL but the server does not use NATS: {}.",
                plural(with.len(), "worker has", "workers have"),
                names(&with)
            ))
            .why(
                "Those workers send streamed output without waiting for the server and drop \
                 pieces when it falls behind, which is only safe when the server carries live \
                 updates over NATS. Here the server saves every piece, so streamed replies \
                 can arrive with gaps.",
            )
            .fix("Set the same NATS_URL on the server, or remove it from those workers."),
    }
}
