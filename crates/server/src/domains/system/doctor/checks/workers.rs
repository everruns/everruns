use super::{Findings, plural};
use crate::domains::system::doctor::{CheckArea, DoctorCheck, SystemDoctor};
use chrono::Utc;
use everruns_durable::{CircuitState, WORKER_HEARTBEAT_TIMEOUT_SECS, WorkerFilter, WorkerInfo};
use std::collections::BTreeSet;

const AREA: CheckArea = CheckArea::Workers;

/// Load share of total capacity that counts as running hot.
const BUSY_SHARE: f64 = 0.8;

/// A runnable session waiting this long means workers are not keeping up.
const SLOW_START_MS: i64 = 60_000;
const STALLED_START_MS: i64 = 5 * 60_000;

/// The shortest worker token accepted without a warning (256 bits of hex).
const MIN_TOKEN_LEN: usize = 32;

pub(super) async fn run(doctor: &SystemDoctor) -> Findings {
    let mut found = Findings::default();
    found.check(auth_check(doctor));

    let Some(store) = doctor.workflow_store.as_ref() else {
        found.check(
            DoctorCheck::new(AREA, "workers.connected", "Workers connected")
                .info("This server runs without the durable store, so workers cannot be checked."),
        );
        return found;
    };

    let workers = match store.list_workers(WorkerFilter::active()).await {
        Ok(workers) => workers,
        Err(error) => {
            found.check(
                DoctorCheck::new(AREA, "workers.connected", "Workers connected")
                    .problem(format!("The worker registry cannot be read: {error}."))
                    .why("Without it nobody knows which workers can take work.")
                    .fix("Check the database checks above."),
            );
            return found;
        }
    };
    let live = fresh(workers);
    let groups: BTreeSet<&str> = live
        .iter()
        .filter_map(|worker| worker.worker_group.as_deref())
        .collect();
    found.fact(if groups.len() > 1 {
        format!(
            "{} in {} groups",
            plural(live.len(), "worker", "workers"),
            groups.len()
        )
    } else {
        plural(live.len(), "worker", "workers")
    });

    let connected = DoctorCheck::new(AREA, "workers.connected", "Workers connected");
    found.check(if live.is_empty() {
        connected
            .problem("No worker is connected.")
            .why("Agents do not run: every message waits until a worker connects.")
            .fix(
                "Start a worker, and check its log for connection or authentication errors \
                 (WORKER_GRPC_ADDRESS, WORKER_GRPC_AUTH_TOKEN).",
            )
    } else {
        connected.ok(format!(
            "{} connected and sending heartbeats.",
            plural(live.len(), "worker is", "workers are")
        ))
    });

    let health = store.get_system_health().await.ok();
    found.check(capacity_check(
        &live,
        health.as_ref().map(|h| h.pending_tasks),
    ));
    found.check(start_delay_check(doctor).await);
    if let Some(health) = &health {
        found.check(dead_letter_check(health.dlq_size));
    }
    if let Ok(breakers) = store.list_circuit_breakers().await {
        let open: Vec<String> = breakers
            .into_iter()
            .filter(|breaker| breaker.state == CircuitState::Open)
            .map(|breaker| breaker.key)
            .collect();
        found.check(circuit_check(&open));
    }
    found
}

/// Workers that heartbeated recently enough to count as alive.
fn fresh(workers: Vec<WorkerInfo>) -> Vec<WorkerInfo> {
    let cutoff = Utc::now() - chrono::Duration::seconds(WORKER_HEARTBEAT_TIMEOUT_SECS);
    workers
        .into_iter()
        .filter(|worker| worker.last_heartbeat_at > cutoff)
        .collect()
}

fn auth_check(doctor: &SystemDoctor) -> DoctorCheck {
    let env = &doctor.env;
    let check = DoctorCheck::new(AREA, "workers.authentication", "Worker authentication");
    if env.dev_mode {
        return check.info("The worker runs inside the server, so it needs no token.");
    }
    match env.worker_token_len {
        None => check
            .problem_if(env.strict(), "WORKER_GRPC_AUTH_TOKEN is not set.")
            .why(
                "Anyone who can reach the worker port can pose as a worker and read \
                 agent work, credentials included.",
            )
            .fix(
                "Generate a token (`openssl rand -hex 32`) and set WORKER_GRPC_AUTH_TOKEN \
                 to the same value on the server and every worker.",
            ),
        Some(len) if len < MIN_TOKEN_LEN => check
            .warning(format!("The worker token is only {len} characters long."))
            .why("A short token can be guessed.")
            .fix("Use at least 32 random characters (`openssl rand -hex 32`)."),
        Some(_) => check.ok("Workers must present a token to connect."),
    }
}

pub(crate) fn capacity_check(live: &[WorkerInfo], pending_tasks: Option<usize>) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.capacity", "Worker capacity");
    if live.is_empty() {
        return check.info("No workers, so no capacity to measure.");
    }
    let capacity: u64 = live.iter().map(|w| u64::from(w.max_concurrency)).sum();
    let load: u64 = live.iter().map(|w| u64::from(w.current_load)).sum();
    let refusing: Vec<String> = live
        .iter()
        .filter(|worker| !worker.accepting_tasks)
        .map(|worker| match &worker.backpressure_reason {
            Some(reason) => format!("{} ({reason})", worker.id),
            None => worker.id.clone(),
        })
        .collect();
    let pending = pending_tasks.unwrap_or_default();
    let share = if capacity == 0 {
        1.0
    } else {
        load as f64 / capacity as f64
    };
    let usage = format!("{load} of {capacity} slots in use");
    if refusing.len() == live.len() {
        check
            .problem(format!(
                "Every worker refuses new work: {}.",
                refusing.join(", ")
            ))
            .why("New work queues up until a worker recovers.")
            .fix("Look at the reasons above in the worker logs; a resource limit is usual.")
    } else if share >= 1.0 && pending > 0 {
        check
            .problem(format!(
                "Workers are full ({usage}) and {pending} tasks wait."
            ))
            .why("Agents answer late: each waiting task starts only when a slot frees up.")
            .fix("Add workers, or raise their concurrency if they have resources to spare.")
    } else if share >= BUSY_SHARE || !refusing.is_empty() {
        let refusing = if refusing.is_empty() {
            String::new()
        } else {
            format!(" Refusing work: {}.", refusing.join(", "))
        };
        check
            .warning(format!("Workers are busy ({usage}).{refusing}"))
            .why("A burst of traffic will queue instead of starting right away.")
            .fix("Add workers before the load grows further.")
    } else {
        check.ok(format!("Workers have room ({usage})."))
    }
}

async fn start_delay_check(doctor: &SystemDoctor) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.start_delay", "Waiting sessions");
    let totals = match doctor.db.monitoring_totals(None, Utc::now()).await {
        Ok(totals) => totals,
        Err(_) => return check.info("Waiting sessions cannot be counted right now."),
    };
    let waiting = totals.runnable;
    match totals.oldest_runnable_ms {
        Some(age) if age >= STALLED_START_MS => check
            .problem(format!(
                "{} waiting for a worker; the oldest for {} minutes.",
                plural(waiting as usize, "session is", "sessions are"),
                age / 60_000
            ))
            .why("These conversations are stuck: nobody is picking them up.")
            .fix("Check that workers are connected and not full."),
        Some(age) if age >= SLOW_START_MS => check
            .warning(format!(
                "{} waiting for a worker; the oldest for {} seconds.",
                plural(waiting as usize, "session is", "sessions are"),
                age / 1000
            ))
            .why("Replies start late.")
            .fix("Add workers if this persists."),
        _ => check.ok("Sessions are picked up without delay."),
    }
}

fn dead_letter_check(size: usize) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.dead_letters", "Failed tasks");
    if size == 0 {
        check.ok("No task has failed permanently.")
    } else {
        check
            .warning(format!(
                "{} failed permanently and wait in the dead-letter queue.",
                plural(size, "task has", "tasks have")
            ))
            .why("Whatever those tasks were doing did not happen.")
            .fix("Review them on the Queues page, then retry or discard them.")
    }
}

fn circuit_check(open: &[String]) -> DoctorCheck {
    let check = DoctorCheck::new(AREA, "workers.circuit_breakers", "Circuit breakers");
    if open.is_empty() {
        check.ok("No circuit breaker is open.")
    } else {
        check
            .warning(format!(
                "{} open: {}.",
                plural(open.len(), "circuit breaker is", "circuit breakers are"),
                open.join(", ")
            ))
            .why("Calls behind an open breaker fail fast until it closes.")
            .fix("Find why the protected calls fail; the breaker closes once they succeed.")
    }
}
