//! Checks across server replicas, from their heartbeats in `system_instances`.

use super::{Findings, plural};
use crate::domains::system::doctor::instances::{
    self, HEARTBEAT_INTERVAL, Instance, InstanceRow, KIND_SERVER, LIVE_WINDOW, RECENT_WINDOW,
};
use crate::domains::system::doctor::{CheckArea, DoctorCheck, SystemDoctor};
use chrono::{DateTime, TimeDelta, Utc};
use std::collections::BTreeMap;

const AREA: CheckArea = CheckArea::ControlPlane;

/// Two versions side by side for longer than this is no longer a rollout.
const ROLLOUT: TimeDelta = TimeDelta::minutes(10);

/// The server replicas as the heartbeats show them.
#[derive(Debug, Default)]
pub(crate) struct Replicas {
    pub now: DateTime<Utc>,
    /// Seen within [`LIVE_WINDOW`].
    pub live: Vec<InstanceRow>,
    /// Seen within [`RECENT_WINDOW`] but not since.
    pub gone: Vec<InstanceRow>,
    /// Whether the heartbeats could be read at all.
    pub known: bool,
}

impl Replicas {
    pub fn from_rows(now: DateTime<Utc>, rows: Vec<InstanceRow>) -> Self {
        let (live, gone) = rows
            .into_iter()
            .partition(|row| now - row.last_seen_at <= LIVE_WINDOW);
        Self {
            now,
            live,
            gone,
            known: true,
        }
    }

    /// How many servers run. `this_id` is this server, counted even when its
    /// first beat is not in yet; `None` for a command-line run.
    pub fn live_count(&self, this_id: Option<&str>) -> usize {
        let missing = this_id.is_some_and(|id| !self.live.iter().any(|row| row.id == id));
        self.live.len() + usize::from(missing)
    }
}

/// This process's instance id, unless the doctor runs from the command line.
pub(crate) fn this_id(doctor: &SystemDoctor) -> Option<&'static str> {
    (!doctor.preflight).then(|| Instance::this_server().id.as_str())
}

/// Read the heartbeats once for every check that needs them.
pub(crate) async fn load(doctor: &SystemDoctor) -> Replicas {
    match instances::recent(doctor.db.pool(), KIND_SERVER, RECENT_WINDOW).await {
        Ok((now, rows)) => Replicas::from_rows(now, rows),
        // The migrations check reports a missing table; nothing to add here.
        Err(error) => {
            tracing::debug!(%error, "System doctor could not read server heartbeats");
            Replicas::default()
        }
    }
}

pub(super) fn run(doctor: &SystemDoctor, replicas: &Replicas) -> Findings {
    let mut found = Findings::default();
    if !replicas.known {
        return found;
    }
    let this = this_id(doctor);
    let count = replicas.live_count(this);
    if count > 1 {
        found.fact(plural(count, "API server", "API servers"));
    }
    let uptime = TimeDelta::from_std(doctor.uptime()).unwrap_or_default();
    found.check(servers_check(replicas, this, uptime));
    if count > 1 {
        found.check(versions_check(replicas));
    }
    found
}

pub(crate) fn servers_check(
    replicas: &Replicas,
    this_id: Option<&str>,
    uptime: TimeDelta,
) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "control_plane.servers", "API servers");
    let Some(this_id) = this_id else {
        // A command-line run: report what heartbeats, nothing about itself.
        return match replicas.live.len() {
            0 => check.info("No server is running against this database."),
            1 => check.ok(format!(
                "One server is running: {}.",
                labels(&replicas.live).join(", ")
            )),
            count => check.ok(format!(
                "{count} servers are running: {}.",
                labels(&replicas.live).join(", ")
            )),
        };
    };
    let beats_missing = !replicas.live.iter().any(|row| row.id == this_id);
    // Two missed beats: the first one is written at start.
    let grace = TimeDelta::from_std(HEARTBEAT_INTERVAL * 2).unwrap_or_default();
    if beats_missing && uptime > grace {
        return check
            .warning("This server has not recorded its heartbeat.")
            .why(
                "Other servers cannot see it, so the replica and version checks here are \
                 incomplete.",
            )
            .fix("Check this server's log for \"Failed to record this server's heartbeat\".");
    }
    let mut summary = match replicas.live.len().max(1) {
        1 => "One server is running.".to_string(),
        count => format!(
            "{count} servers are running: {}.",
            labels(&replicas.live).join(", ")
        ),
    };
    if !replicas.gone.is_empty() {
        summary.push_str(&format!(
            " {} in the last hour: {}.",
            plural(replicas.gone.len(), "server stopped", "servers stopped"),
            labels(&replicas.gone).join(", ")
        ));
    }
    check.ok(summary)
}

pub(crate) fn versions_check(replicas: &Replicas) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "control_plane.server_versions", "Server versions");
    // Each version and when its first running replica started.
    let mut versions: BTreeMap<&str, (DateTime<Utc>, usize)> = BTreeMap::new();
    for row in &replicas.live {
        let entry = versions
            .entry(row.version.as_str())
            .or_insert((row.started_at, 0));
        entry.0 = entry.0.min(row.started_at);
        entry.1 += 1;
    }
    if versions.len() <= 1 {
        let version = versions.keys().next().copied().unwrap_or("unknown");
        return check.ok(format!("Every server runs {version}."));
    }
    let listed: Vec<String> = versions
        .iter()
        .map(|(version, (_, count))| {
            format!("{} on {version}", plural(*count, "server", "servers"))
        })
        .collect();
    // The mix began when the last version to arrive first started.
    let mixed_since = versions
        .values()
        .map(|(started, _)| *started)
        .max()
        .unwrap_or(replicas.now);
    let mixed_for = replicas.now - mixed_since;
    let check = if mixed_for > ROLLOUT {
        check.problem(format!(
            "Servers have run different versions for {} minutes: {}.",
            mixed_for.num_minutes(),
            listed.join(", ")
        ))
    } else {
        check.warning(format!(
            "Servers run different versions: {}. Expected while a deploy rolls out.",
            listed.join(", ")
        ))
    };
    check
        .why(
            "Requests land on either version, so people see behavior change from one click \
             to the next, and an older server may not read what a newer one wrote.",
        )
        .fix("Finish or roll back the deploy so every server runs one version.")
}

fn labels(rows: &[InstanceRow]) -> Vec<String> {
    rows.iter()
        .map(|row| format!("{} ({})", row.label(), row.version))
        .collect()
}
