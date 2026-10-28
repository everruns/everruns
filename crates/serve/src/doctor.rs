//! `GET /v1/doctor`: whether this serve app is set up right and working, in
//! the everruns server's system doctor report shape so the same tools read
//! both.
//!
//! Decisions:
//! - The server's doctor judges a fleet (workers, NATS, S3, replicas); serve is
//!   one process, so it checks only what a serve app depends on: its session
//!   store, a route for every agent's model, its schedules keeping time, and
//!   whether its routes are open.
//! - Behind the same auth as the other agent routes: the report names
//!   settings, though never their values.
//! - Strictness follows the mode. What is fine in `dev` (open routes, the
//!   simulator) is a warning under `start`.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use chrono::{DateTime, Duration, Utc};
use serde::Serialize;

use crate::app::Mode;
use crate::gateway::{self, Route};
use crate::host::Host;

/// A schedule this late past its due time is not keeping time.
const SCHEDULE_GRACE: Duration = Duration::minutes(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Status {
    Ok,
    Info,
    Warning,
    Problem,
}

#[derive(Debug, Serialize)]
pub(crate) struct Check {
    id: &'static str,
    area: &'static str,
    title: &'static str,
    status: Status,
    summary: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    why: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fix: Option<String>,
    observed_at: DateTime<Utc>,
}

impl Check {
    fn new(area: &'static str, id: &'static str, title: &'static str) -> Self {
        Self {
            id,
            area,
            title,
            status: Status::Ok,
            summary: String::new(),
            why: None,
            fix: None,
            observed_at: Utc::now(),
        }
    }

    fn with(mut self, status: Status, summary: impl Into<String>) -> Self {
        self.status = status;
        self.summary = summary.into();
        self
    }

    fn why(mut self, why: impl Into<String>) -> Self {
        self.why = Some(why.into());
        self
    }

    fn fix(mut self, fix: impl Into<String>) -> Self {
        self.fix = Some(fix.into());
        self
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct Report {
    status: Status,
    checked_at: DateTime<Utc>,
    version: String,
    build_id: String,
    /// The serve mode (`dev`, `start`, `eval`), in the server report's place
    /// for the deployment grade.
    grade: &'static str,
    auth_mode: &'static str,
    setup: Vec<String>,
    checks: Vec<Check>,
}

pub(crate) async fn doctor(State(host): State<Arc<Host>>) -> Json<Report> {
    Json(report(&host))
}

pub(crate) fn report(host: &Host) -> Report {
    let manifest = host.app.manifest();
    let strict = host.mode == Mode::Start;
    let mut setup = vec![format!("{} {}", manifest.app.name, manifest.app.version)];
    if !host.persistent() {
        setup.push("Sessions in memory".to_string());
    }
    let mut checks = vec![store_check(host), auth_check(host.auth().is_some(), strict)];
    let models: Vec<(String, String)> = manifest
        .agents
        .iter()
        .map(|agent| (agent.name.clone(), agent.model.clone()))
        .collect();
    checks.push(models_check(&models, &host.gateway, strict));
    let schedules: Vec<ScheduleState> = host
        .app
        .inner
        .schedules
        .iter()
        .map(|entry| {
            let name = entry.registration.name;
            let last = host.store.schedule_due_at(name).ok().flatten();
            let next = last.and_then(|last| entry.schedule.after(&last).next());
            (name, next)
        })
        .collect();
    checks.push(schedules_check(&schedules, Utc::now()));
    checks.sort_by_key(|check| std::cmp::Reverse(check.status));
    let status = checks
        .iter()
        .map(|check| check.status)
        .filter(|status| *status > Status::Info)
        .max()
        .unwrap_or(Status::Ok);
    Report {
        status,
        checked_at: Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        build_id: host.build_id.clone(),
        grade: match host.mode {
            Mode::Dev => "dev",
            Mode::Start => "start",
            Mode::Eval => "eval",
        },
        auth_mode: if host.auth().is_some() {
            "bearer"
        } else {
            "none"
        },
        setup,
        checks,
    }
}

fn store_check(host: &Host) -> Check {
    let check = Check::new("database", "store.sessions", "Session store");
    match host.store.session("doctor-probe") {
        Ok(_) if host.persistent() => check.with(Status::Ok, "Sessions are stored on disk."),
        Ok(_) => check.with(
            Status::Info,
            "Sessions are kept in memory and are gone when the process stops.",
        ),
        Err(err) => check
            .with(
                Status::Problem,
                format!("The session store cannot be read: {err}."),
            )
            .why("No session can be created or continued.")
            .fix("Check that the data directory exists, is writable and has free space."),
    }
}

pub(crate) fn auth_check(configured: bool, strict: bool) -> Check {
    let check = Check::new("security", "security.auth", "Who may call the agents");
    match (configured, strict) {
        (true, _) => check.with(Status::Ok, "Agent routes need a bearer credential."),
        (false, true) => check
            .with(
                Status::Warning,
                "Agent routes are open to anyone who reaches this address.",
            )
            .why("Anyone can start sessions and spend this app's model budget.")
            .fix("Configure auth on the app (`App::auth`) or put it behind a gateway that does."),
        (false, false) => check.with(
            Status::Info,
            "Agent routes are open; fine on your own machine.",
        ),
    }
}

/// Every agent's model must resolve to something that can run it.
pub(crate) fn models_check(agents: &[(String, String)], env: &gateway::Env, strict: bool) -> Check {
    let check = Check::new("models", "models.routes", "Model routes");
    let mut missing = Vec::new();
    let mut simulated = Vec::new();
    for (agent, model) in agents {
        match gateway::route(model, env) {
            None => missing.push(format!("{agent} ({model})")),
            Some(Route::Simulator) => simulated.push(agent.as_str()),
            Some(_) => {}
        }
    }
    if !missing.is_empty() {
        let summary = format!("No credential can run: {}.", missing.join(", "));
        let check = if strict {
            check
                .with(Status::Problem, summary)
                .why("Messages to these agents fail.")
        } else {
            check
                .with(Status::Warning, summary)
                .why("These agents answer from the offline simulator, not a real model.")
        };
        return check.fix(
            "Set SERVE_GATEWAY_URL, OPENROUTER_API_KEY, or OPENAI_API_KEY for openai/ models.",
        );
    }
    if !simulated.is_empty() && strict {
        return check
            .with(
                Status::Warning,
                format!("These agents use the simulator: {}.", simulated.join(", ")),
            )
            .why("They answer with scripted replies, not a real model.");
    }
    check.with(
        Status::Ok,
        format!("Every agent's model has a route ({}).", agents.len()),
    )
}

/// A schedule's name and the first occurrence after the last one it handled
/// (`None` before its first record).
pub(crate) type ScheduleState<'a> = (&'a str, Option<DateTime<Utc>>);

/// Each schedule has fired for every occurrence that fell due, give or take
/// [`SCHEDULE_GRACE`].
pub(crate) fn schedules_check(schedules: &[ScheduleState], now: DateTime<Utc>) -> Check {
    let check = Check::new("control_plane", "control_plane.schedules", "Schedules");
    if schedules.is_empty() {
        return check.with(Status::Info, "This app has no schedules.");
    }
    let late: Vec<&str> = schedules
        .iter()
        .filter(|(_, next)| next.is_some_and(|next| next + SCHEDULE_GRACE < now))
        .map(|(name, _)| *name)
        .collect();
    if late.is_empty() {
        check.with(
            Status::Ok,
            format!("{} schedules are keeping time.", schedules.len()),
        )
    } else {
        check
            .with(
                Status::Warning,
                format!("Behind their due time: {}.", late.join(", ")),
            )
            .why("A run that is due has not happened; work it does is late.")
            .fix("Check this process's logs for the schedule, or whether the host cron calls it.")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(openrouter: bool) -> gateway::Env {
        gateway::Env {
            gateway_url: None,
            gateway_key: None,
            openrouter_key: openrouter.then(|| "key".to_string()),
            openai_key: None,
            aws_region: None,
        }
    }

    #[test]
    fn an_agent_without_a_model_route_fails_under_start() {
        let agents = vec![
            (
                "support".to_string(),
                "anthropic/claude-sonnet-5".to_string(),
            ),
            ("echo".to_string(), "sim".to_string()),
        ];
        let start = models_check(&agents, &env(false), true);
        assert_eq!(start.status, Status::Problem);
        assert!(
            start
                .summary
                .contains("support (anthropic/claude-sonnet-5)")
        );
        assert_eq!(
            models_check(&agents, &env(false), false).status,
            Status::Warning
        );
        // Routed, but one agent still runs on the simulator.
        assert_eq!(
            models_check(&agents, &env(true), true).status,
            Status::Warning
        );
        assert_eq!(models_check(&agents, &env(true), false).status, Status::Ok);
    }

    #[test]
    fn open_routes_are_a_warning_only_in_production() {
        assert_eq!(auth_check(false, true).status, Status::Warning);
        assert_eq!(auth_check(false, false).status, Status::Info);
        assert_eq!(auth_check(true, true).status, Status::Ok);
    }

    #[test]
    fn a_schedule_past_its_due_time_is_flagged() {
        let now = Utc::now();
        let on_time = [("digest", Some(now + Duration::hours(1)))];
        assert_eq!(schedules_check(&on_time, now).status, Status::Ok);
        let late = [("digest", Some(now - Duration::hours(1)))];
        let check = schedules_check(&late, now);
        assert_eq!(check.status, Status::Warning);
        assert!(check.summary.contains("digest"));
        // First boot has no record yet: not late.
        assert_eq!(schedules_check(&[("digest", None)], now).status, Status::Ok);
        assert_eq!(schedules_check(&[], now).status, Status::Info);
    }
}
