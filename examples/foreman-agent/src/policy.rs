//! The deterministic half: what the numbers are allowed to do.
//!
//! The classifier estimates; it never commands. Every threshold, every
//! resource limit, and the whole vocabulary of legal actions live here, in
//! ordinary Rust that can be read, tested, and recalibrated without touching
//! how evidence is assessed.
//!
//! The ordering is safety-first, and deliberate: a person is needed before a
//! limit is reached, a limit before a productivity judgment.

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
    /// The previous decision, which is what makes a retry distinguishable
    /// from an ordinary start.
    pub last_action: Option<Action>,
}

/// Decide what to do about `assessment`, given the floor and the config.
///
/// Reading order is the policy: human need, iteration bounds, off-track and
/// stuck workers, retry handling, completion, verification, then continued work.
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
    let verification_resolved =
        floor.verification_completed || assessment.needs_verification < config.verification;
    if active.is_none() && finish_ready && verification_resolved {
        return decision(Action::Finish, "completion thresholds satisfied", None);
    }

    let should_verify = active.is_none()
        && assessment.needs_verification >= config.verification
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
            "meaningful implementation work remains",
            None,
        );
    }

    decision(Action::Continue, "active worker may continue", None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn working() -> Floor {
        Floor {
            iteration: 1,
            active_worker: Some("worker-1".into()),
            workers_started: 1,
            ..Floor::default()
        }
    }

    fn idle() -> Floor {
        Floor {
            iteration: 1,
            workers_started: 1,
            ..Floor::default()
        }
    }

    fn complete() -> Assessment {
        Assessment {
            implementation_complete: 0.95,
            tests_sufficient: 0.9,
            requirements_satisfied: 0.92,
            needs_verification: 0.1,
            ready_to_finish: 0.93,
            meaningful_progress: 0.9,
            ..Assessment::default()
        }
    }

    #[test]
    fn a_person_is_needed_before_anything_else() {
        let assessment = Assessment {
            needs_human: 0.81,
            // Every other signal says finish; the human threshold still wins.
            ..complete()
        };
        assert_eq!(
            decide(&idle(), &assessment, &Config::default()).action,
            Action::Escalate
        );
    }

    #[test]
    fn the_iteration_ceiling_escalates_even_on_healthy_evidence() {
        let floor = Floor {
            iteration: 20,
            ..working()
        };
        let decision = decide(&floor, &complete(), &Config::default());
        assert_eq!(decision.action, Action::Escalate);
        assert_eq!(decision.reason, "maximum supervisory iterations reached");
    }

    #[test]
    fn drift_outranks_stuckness_and_names_the_worker() {
        let assessment = Assessment {
            work_off_track: 0.9,
            worker_stuck: 0.95,
            ..Assessment::default()
        };
        let decision = decide(&working(), &assessment, &Config::default());
        assert_eq!(decision.action, Action::StopWorker);
        assert_eq!(decision.reason, "active worker appears off track");
        assert_eq!(decision.worker.as_deref(), Some("worker-1"));
    }

    #[test]
    fn a_stuck_worker_is_stopped() {
        let assessment = Assessment {
            worker_stuck: 0.85,
            ..Assessment::default()
        };
        assert_eq!(
            decide(&working(), &assessment, &Config::default()).action,
            Action::StopWorker
        );
    }

    #[test]
    fn an_idle_floor_cannot_be_stopped() {
        let assessment = Assessment {
            worker_stuck: 0.99,
            work_off_track: 0.99,
            ..Assessment::default()
        };
        assert_eq!(
            decide(&idle(), &assessment, &Config::default()).action,
            Action::StartWorker
        );
    }

    #[test]
    fn a_stop_is_followed_by_one_retry_then_escalation() {
        let mut floor = Floor {
            last_action: Some(Action::StopWorker),
            ..idle()
        };
        assert_eq!(
            decide(&floor, &Assessment::default(), &Config::default()).action,
            Action::RetryWorker
        );

        floor.retries = 1;
        let decision = decide(&floor, &Assessment::default(), &Config::default());
        assert_eq!(decision.action, Action::Escalate);
        assert_eq!(decision.reason, "worker retry limit reached");
    }

    #[test]
    fn verification_is_started_once_and_only_once() {
        let assessment = Assessment {
            implementation_complete: 0.9,
            needs_verification: 0.8,
            ..Assessment::default()
        };
        assert_eq!(
            decide(&idle(), &assessment, &Config::default()).action,
            Action::StartVerifier
        );

        let floor = Floor {
            verification_started: true,
            ..idle()
        };
        assert_eq!(
            decide(&floor, &assessment, &Config::default()).action,
            Action::StartWorker
        );
    }

    #[test]
    fn verification_waits_for_implementation() {
        let assessment = Assessment {
            implementation_complete: 0.4,
            needs_verification: 0.9,
            ..Assessment::default()
        };
        assert_eq!(
            decide(&idle(), &assessment, &Config::default()).action,
            Action::StartWorker
        );
    }

    #[test]
    fn finishing_needs_completion_and_a_resolved_verification() {
        let config = Config::default();
        assert_eq!(decide(&idle(), &complete(), &config).action, Action::Finish);

        // Same completion evidence, but verification is now wanted and has not
        // happened: the job is not finishable yet.
        let wants_verification = Assessment {
            needs_verification: 0.9,
            ..complete()
        };
        assert_eq!(
            decide(&idle(), &wants_verification, &config).action,
            Action::StartVerifier
        );

        let verified = Floor {
            verification_started: true,
            verification_completed: true,
            workers_started: 2,
            ..idle()
        };
        assert_eq!(
            decide(&verified, &wants_verification, &config).action,
            Action::Finish
        );
    }

    #[test]
    fn weak_tests_hold_a_finish_back() {
        let assessment = Assessment {
            tests_sufficient: 0.3,
            ..complete()
        };
        assert_eq!(
            decide(&idle(), &assessment, &Config::default()).action,
            Action::StartWorker
        );
    }

    #[test]
    fn the_worker_budget_escalates_rather_than_starting_a_fourth() {
        let floor = Floor {
            workers_started: 3,
            ..idle()
        };
        let decision = decide(&floor, &Assessment::default(), &Config::default());
        assert_eq!(decision.action, Action::Escalate);
        assert_eq!(decision.reason, "worker limit reached before completion");
    }

    #[test]
    fn an_unremarkable_floor_lets_the_worker_work() {
        let assessment = Assessment {
            meaningful_progress: 0.9,
            implementation_complete: 0.4,
            ..Assessment::default()
        };
        assert_eq!(
            decide(&working(), &assessment, &Config::default()).action,
            Action::Continue
        );
    }

    #[test]
    fn environment_overrides_land_on_the_defaults_they_name() {
        // Read through the same helpers the override path uses; setting process
        // environment in a threaded test runner is not safe.
        assert_eq!(count("FOREMAN_UNSET_LIMIT_FOR_TEST"), None);
        assert_eq!(seconds("FOREMAN_UNSET_SECONDS_FOR_TEST"), None);
        let config = Config::default();
        assert_eq!(config.max_workers, 3);
        assert_eq!(config.periodic_assessment, Duration::from_secs(30));
    }
}
