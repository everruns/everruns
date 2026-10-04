//! Deterministic actions, thresholds, and budgets. The decision model returns only numbers.

use std::time::Duration;

use serde::Serialize;

use crate::foreman::Assessment;

/// Thresholds and limits. Defaults are Foreman's.
#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// Floor between two assessments, however noisy the worker is.
    pub min_assessment_interval: Duration,
    /// Assess at least this often while work is quiet.
    pub periodic_assessment: Duration,
    /// Budget for one assessment call.
    pub assessment_budget: Duration,
    /// Budget for one run of the repository's own tests.
    pub test_timeout: Duration,
    /// Budget for the whole run.
    pub overall_timeout: Duration,
    /// Total workers, verifier included.
    pub max_workers: usize,
    /// Fresh attempts allowed after a stop.
    pub max_retries: usize,
    /// Ceiling on supervisory decisions.
    pub max_iterations: usize,

    /// `needs_human` at or above this escalates.
    pub human: f64,
    /// `work_off_track` at or above this stops the worker.
    pub off_track: f64,
    /// `worker_stuck` at or above this stops the worker.
    pub stuck: f64,
    /// `needs_verification` at or above this starts a verifier.
    pub verification: f64,
    /// `implementation_complete` required before verification is worth it.
    pub implementation_for_verification: f64,
    /// `ready_to_finish` required to finish.
    pub finish: f64,
    /// `requirements_satisfied` required to finish.
    pub requirements: f64,
    /// `tests_sufficient` required to finish.
    pub tests: f64,

    /// Characters of `git diff` an observation may carry.
    pub diff_limit: usize,
    /// Characters of any one captured output tail.
    pub output_limit: usize,
    /// Recent session events an observation may carry.
    pub event_limit: usize,
    /// Workers of history an observation may carry.
    pub history_limit: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            min_assessment_interval: Duration::from_secs(5),
            periodic_assessment: Duration::from_secs(30),
            assessment_budget: Duration::from_secs(10),
            test_timeout: Duration::from_secs(120),
            overall_timeout: Duration::from_secs(1_800),
            max_workers: 3,
            max_retries: 1,
            max_iterations: 20,

            human: 0.80,
            off_track: 0.80,
            stuck: 0.80,
            verification: 0.65,
            implementation_for_verification: 0.75,
            finish: 0.85,
            requirements: 0.80,
            tests: 0.75,

            diff_limit: 20_000,
            output_limit: 12_000,
            event_limit: 30,
            history_limit: 10,
        }
    }
}

impl Config {
    /// Apply the environment overrides, leaving every unset one at its default.
    pub fn from_env() -> Self {
        let mut config = Self::default();
        if let Some(seconds) = seconds("FOREMAN_ASSESSMENT_MIN_INTERVAL_SECONDS") {
            config.min_assessment_interval = seconds;
        }
        if let Some(seconds) = seconds("FOREMAN_PERIODIC_ASSESSMENT_SECONDS") {
            config.periodic_assessment = seconds;
        }
        if let Some(seconds) = seconds("FOREMAN_ASSESSMENT_TIMEOUT_SECONDS") {
            config.assessment_budget = seconds;
        }
        if let Some(seconds) = seconds("FOREMAN_TEST_TIMEOUT_SECONDS") {
            config.test_timeout = seconds;
        }
        if let Some(seconds) = seconds("FOREMAN_OVERALL_TIMEOUT_SECONDS") {
            config.overall_timeout = seconds;
        }
        if let Some(count) = count("FOREMAN_MAX_WORKERS") {
            config.max_workers = count.max(1);
        }
        if let Some(count) = count("FOREMAN_MAX_RETRIES") {
            config.max_retries = count;
        }
        if let Some(count) = count("FOREMAN_MAX_ITERATIONS") {
            config.max_iterations = count.max(1);
        }
        config
    }
}

fn seconds(name: &str) -> Option<Duration> {
    let value = std::env::var(name).ok()?.parse::<f64>().ok()?;
    (value.is_finite() && value > 0.0).then(|| Duration::from_secs_f64(value))
}

fn count(name: &str) -> Option<usize> {
    std::env::var(name).ok()?.parse::<usize>().ok()
}

/// What the supervisor is allowed to do about an assessment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Let the active worker keep working.
    Continue,
    /// Begin a coding pass because work remains.
    StartWorker,
    /// Launch one independent verification pass.
    StartVerifier,
    /// Cancel a stuck or off-track worker's turn.
    StopWorker,
    /// Start a fresh coding worker after a stopped attempt.
    RetryWorker,
    /// Declare the job complete.
    Finish,
    /// Stop autonomous work and ask for a person.
    Escalate,
}

impl Action {
    /// The action as it is written in the timeline.
    pub fn label(self) -> &'static str {
        match self {
            Self::Continue => "CONTINUE",
            Self::StartWorker => "START_WORKER",
            Self::StartVerifier => "START_VERIFIER",
            Self::StopWorker => "STOP_WORKER",
            Self::RetryWorker => "RETRY_WORKER",
            Self::Finish => "FINISH",
            Self::Escalate => "ESCALATE",
        }
    }
}

/// A decision, with the evidence-independent reason it was reached.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Intervention {
    /// What to do.
    pub action: Action,
    /// Why, in one phrase.
    pub reason: String,
    /// The supervisory iteration that produced it.
    pub iteration: usize,
    /// The worker it applies to, when it applies to one.
    pub worker: Option<String>,
}

/// The lifecycle facts the policy needs. Nothing semantic lives here.
#[derive(Clone, Debug, Default)]
pub struct Floor {
    /// Supervisory iterations so far, this one included.
    pub iteration: usize,
    /// The worker currently running, if any.
    pub active_worker: Option<String>,
    /// Workers started so far, verifier included.
    pub workers_started: usize,
    /// Fresh attempts spent.
    pub retries: usize,
    /// Whether a verification pass has been launched.
    pub verification_started: bool,
    /// Whether a verification pass has reported.
    pub verification_completed: bool,
    /// None when no suite is configured; a known failure blocks completion.
    pub tests_passed: Option<bool>,
    /// The previous decision, which is what makes a retry distinguishable
    /// from an ordinary start.
    pub last_action: Option<Action>,
}

/// Decide what to do about `assessment`, given the floor and the config.
pub fn decide(floor: &Floor, assessment: &Assessment, config: &Config) -> Intervention {
    let iteration = floor.iteration.max(1);
    let decision = |action: Action, reason: &str, worker: Option<String>| Intervention {
        action,
        reason: reason.to_owned(),
        iteration,
        worker,
    };
    let active = floor.active_worker.clone();

    if assessment.needs_human >= config.human {
        return decision(
            Action::Escalate,
            "semantic assessment requires human input",
            None,
        );
    }

    if floor.iteration >= config.max_iterations {
        return decision(
            Action::Escalate,
            "maximum supervisory iterations reached",
            None,
        );
    }

    if active.is_some() && assessment.work_off_track >= config.off_track {
        return decision(
            Action::StopWorker,
            "active worker appears off track",
            active,
        );
    }

    if active.is_some() && assessment.worker_stuck >= config.stuck {
        return decision(Action::StopWorker, "active worker appears stuck", active);
    }

    if active.is_none() && floor.last_action == Some(Action::StopWorker) {
        let retry_allowed = floor.retries < config.max_retries;
        let worker_allowed = floor.workers_started < config.max_workers;
        if retry_allowed && worker_allowed {
            return decision(
                Action::RetryWorker,
                "retrying stopped worker with a fresh session",
                None,
            );
        }
        return decision(Action::Escalate, "worker retry limit reached", None);
    }

    let finish_ready = assessment.ready_to_finish >= config.finish
        && assessment.requirements_satisfied >= config.requirements
        && assessment.tests_sufficient >= config.tests;
    if active.is_none()
        && finish_ready
        && floor.verification_completed
        && floor.tests_passed != Some(false)
    {
        return decision(Action::Finish, "completion thresholds satisfied", None);
    }

    let should_verify = active.is_none()
        && (assessment.needs_verification >= config.verification || finish_ready)
        && assessment.implementation_complete >= config.implementation_for_verification
        && !floor.verification_started;
    if should_verify {
        if floor.workers_started >= config.max_workers {
            return decision(
                Action::Escalate,
                "verification needed but worker limit reached",
                None,
            );
        }
        return decision(
            Action::StartVerifier,
            "independent verification is warranted",
            None,
        );
    }

    if active.is_none() && floor.verification_started && !floor.verification_completed {
        return decision(
            Action::Escalate,
            "independent verification did not complete successfully",
            None,
        );
    }

    if active.is_none() {
        if floor.workers_started >= config.max_workers {
            return decision(
                Action::Escalate,
                "worker limit reached before completion",
                None,
            );
        }
        return decision(
            Action::StartWorker,
            if floor.workers_started == 0 {
                "dispatching the coding role"
            } else {
                "meaningful implementation work remains"
            },
            None,
        );
    }

    decision(Action::Continue, "active worker may continue", None)
}

#[cfg(test)]
#[path = "../tests/unit/policy.rs"]
mod tests;
