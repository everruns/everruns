//! The whole loop reaching a decision, offline.
//!
//! `foreman demo` is a real run, so CI cannot use it: a live worker costs money
//! and is free to go a different way each time. This is the same run with both
//! ends replaced — a scripted worker that really edits the fixture through the
//! same Bashkit capability, and a decision service that answers from a table.
//! Everything between them is the code a live run executes, so what this pins
//! is the finish path: worker, independent verification, FINISH, and a
//! repository that actually changed.
//!
//! It is a test rather than a mode of the binary on purpose. Scaffolding that
//! ships alongside an example gets read as part of it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;
use std::process::Command;
use std::time::Duration;

use async_trait::async_trait;
use everruns::{
    AgentLoopError, DecisionAnswer, DecisionOutcome, DecisionRequest, Decisions, DecisionsService,
    LlmSimConfig, Model, SimToolCall, SimTurn,
};
use serde_json::Value;

use everruns_foreman_agent::factory::{Factory, Status};
use everruns_foreman_agent::foreman::{DIMENSIONS, Foreman};
use everruns_foreman_agent::observation::{Observation, VerificationResult};
use everruns_foreman_agent::policy::Config;
use everruns_foreman_agent::worker::Crew;
use everruns_foreman_agent::{agent, fixture};

/// Bounds for a scripted worker, which finishes in seconds rather than minutes.
///
/// Only the clocks move, and they are pinned rather than overridable because
/// the scripted worker's own timing is fixed. Every threshold and budget is
/// what a live run uses, `FOREMAN_*` overrides included, because the policy is
/// the part worth seeing unchanged.
pub fn config() -> Config {
    Config {
        min_assessment_interval: Duration::from_millis(1_000),
        periodic_assessment: Duration::from_millis(2_500),
        overall_timeout: Duration::from_secs(180),
        ..Config::from_env()
    }
}

/// What the demo's worker is, named as plainly as the header can say it.
///
/// The header is the one place a reader learns whether they are watching a
/// model or a script, so it says "scripted" rather than borrowing the live
/// run's model name for a run that never contacts a provider.
pub const WORKER_MODEL: &str = "scripted";

/// A worker that implements the tiers, then checks its own work.
pub fn worker() -> Model {
    let step = |text: &str, commands: &str, id: &str| SimTurn::Mixed {
        text: text.to_owned(),
        tool_calls: vec![SimToolCall {
            name: "bash".to_owned(),
            arguments: serde_json::json!({ "commands": commands }),
            id: Some(id.to_owned()),
        }],
    };
    Model::simulated_with_config(
        LlmSimConfig::scripted(vec![
            step(
                "Reading the rate table, and running the suite to see what green looks like.",
                "cat lib/rates.sh; bash tests/run.sh",
                "call_read",
            ),
            step(
                "Replacing the flat rate with weight tiers.",
                include_str!("resources/write_rates.sh"),
                "call_rates",
            ),
            step(
                "Covering the tier boundaries, which is where tiered pricing goes wrong.",
                include_str!("resources/write_tests.sh"),
                "call_tests",
            ),
            step("Running the suite.", "bash tests/run.sh", "call_run_tests"),
            SimTurn::Assistant(
                "Weight tiers replace the flat rate, and every tier edge is covered by a \
                 test. The suite passes: 9 passed, 0 failed."
                    .to_owned(),
            ),
            // A second coding pass, for the run where the policy asks for one
            // after verification. A worker started on a finished repository
            // should read it and say so, not repeat its own summary.
            step(
                "Re-reading what the last pass left behind, and re-running its suite.",
                "sed -n '1,30p' lib/rates.sh; bash tests/run.sh",
                "call_reread",
            ),
            SimTurn::Assistant(
                "Nothing left to change: the tier table and the boundary tests are already \
                 in place, and the zone surcharge still rides on top."
                    .to_owned(),
            ),
        ])
        .with_response_delay(Duration::from_millis(900)),
    )
}

/// A verifier that reads the result and reports on it.
pub fn verifier() -> Model {
    Model::simulated_with_config(
        LlmSimConfig::scripted(vec![
            SimTurn::Mixed {
                text: "Checking the tier table and the boundary tests against the job.".to_owned(),
                tool_calls: vec![SimToolCall {
                    name: "bash".to_owned(),
                    arguments: serde_json::json!({
                        "commands": "cat lib/rates.sh; bash tests/run.sh"
                    }),
                    id: Some("call_verify".to_owned()),
                }],
            },
            SimTurn::Assistant(
                "lib/rates.sh prices by weight, tests/run.sh pins every tier edge, and the \
                 suite reports 9 passed, 0 failed. The job is satisfied."
                    .to_owned(),
            ),
        ])
        .with_response_delay(Duration::from_millis(900)),
    )
}

/// The demo's answers, as a service rather than a fork in the code.
///
/// [`DecisionsService`] is the Framework's seam for "answer these questions
/// without asking a vendor", so the demo uses it instead of teaching
/// [`Foreman`](crate::foreman::Foreman) about a second kind of supervisor. The
/// code under test is then exactly the code a live run executes: the same nine
/// questions, the same request, the same parsing of the answers.
///
/// What it answers is a table in `resources/scripted/readings.json`, keyed by what
/// is on the floor rather than by how many times it has been asked — so the
/// demo walks the same CONTINUE → START_VERIFIER → FINISH path whether the
/// worker takes one second or ten.
pub struct Readings {
    readings: BTreeMap<String, BTreeMap<String, f64>>,
}

/// What the demo reports as the model that answered.
///
/// The file, because that is the honest answer: nothing resolves a model for a
/// table, and naming it says where a reader can go to change the numbers.
pub const READINGS_MODEL: &str = "readings.json";

impl Readings {
    /// Load the readings table.
    pub fn load() -> Result<Self, serde_json::Error> {
        serde_json::from_str(include_str!("resources/readings.json"))
            .map(|readings| Self { readings })
    }

    /// A decision service backed by this service, ready for a `Foreman`.
    pub fn decisions() -> Result<Decisions, serde_json::Error> {
        Ok(Decisions::new(READINGS_MODEL, Self::load()?))
    }

    /// Which reading the floor calls for.
    ///
    /// Reads the observation as JSON, which is the same thing a real service
    /// receives — the stub sees exactly what the model sees, and nothing more.
    fn phase(state: &Value) -> &'static str {
        let empty = Vec::new();
        let array = |key: &str| state.get(key).and_then(Value::as_array).unwrap_or(&empty);
        if array("verification_results")
            .iter()
            .any(|result| result.get("passed").and_then(Value::as_bool) == Some(true))
        {
            return "verified";
        }
        if array("active_workers").is_empty() {
            return "unchecked";
        }
        if array("changed_files").is_empty() {
            "started"
        } else {
            "editing"
        }
    }
}

#[async_trait]
impl DecisionsService for Readings {
    fn is_configured(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        READINGS_MODEL
    }

    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome, AgentLoopError> {
        let phase = Self::phase(&request.state);
        let reading = self.readings.get(phase).ok_or_else(|| {
            AgentLoopError::llm(format!("readings.json has no entry for '{phase}'"))
        })?;
        let answers = request
            .questions
            .iter()
            .map(|(id, _)| {
                let probability = reading.get(id).copied().ok_or_else(|| {
                    AgentLoopError::llm(format!("readings.json '{phase}' has no '{id}'"))
                })?;
                Ok((id.clone(), DecisionAnswer::Noul { probability }))
            })
            .collect::<Result<BTreeMap<_, _>, AgentLoopError>>()?;
        Ok(DecisionOutcome {
            model: READINGS_MODEL.to_owned(),
            answers,
            ..DecisionOutcome::default()
        })
    }
}

#[test]
fn the_scripted_worker_writes_the_files_the_job_asks_for() {
    // The demo's edits are real shell, run by the same capability a live
    // worker uses; keeping them in files means they can be read and run.
    let rates = include_str!("resources/write_rates.sh");
    let tests = include_str!("resources/write_tests.sh");
    assert!(rates.contains("lib/rates.sh"));
    assert!(tests.contains("tests/run.sh"));
    assert!(tests.to_lowercase().contains("boundar"));
}

#[test]
fn every_reading_answers_every_question() {
    // A table missing an id would fail a run halfway through; catching it
    // here costs nothing.
    let readings = Readings::load().unwrap();
    for (phase, reading) in &readings.readings {
        for dimension in &DIMENSIONS {
            let value = reading
                .get(dimension.id)
                .unwrap_or_else(|| panic!("{phase} has no {}", dimension.id));
            assert!((0.0..=1.0).contains(value), "{phase}/{}", dimension.id);
        }
        assert_eq!(reading.len(), DIMENSIONS.len(), "{phase}");
    }
}

#[tokio::test]
async fn the_readings_service_answers_the_nine_questions() {
    let foreman = Foreman::new(Readings::decisions().unwrap(), Duration::from_secs(5));
    let assessment = foreman.assess(&Observation::sample()).await.unwrap();
    // The sample observation has a worker on the floor and nothing changed
    // yet, so this is the opening reading.
    assert_eq!(assessment.implementation_complete, 0.31);
    assert_eq!(assessment.meaningful_progress, 0.88);
}

#[test]
fn the_phase_follows_the_floor_not_the_call_count() {
    let phase =
        |observation: &Observation| Readings::phase(&serde_json::to_value(observation).unwrap());
    let mut observation = Observation::sample();
    assert!(!observation.active_workers.is_empty());
    assert_eq!(phase(&observation), "started");

    observation.changed_files.push("src/rates.py".into());
    assert_eq!(phase(&observation), "editing");

    observation.active_workers.clear();
    assert_eq!(phase(&observation), "unchecked");

    observation.verification_results.push(VerificationResult {
        worker_id: "worker-2".into(),
        passed: true,
        summary: "checked".into(),
    });
    assert_eq!(phase(&observation), "verified");
}

#[test]
fn scripted_clocks_are_pinned_but_budgets_are_not() {
    let config = config();
    assert_eq!(config.min_assessment_interval, Duration::from_millis(1_000));
    // Thresholds and budgets stay exactly what a live run uses.
    assert_eq!(config.finish, Config::default().finish);
    assert_eq!(config.max_workers, Config::from_env().max_workers);
}

#[tokio::test]
async fn the_factory_finishes_after_verifying_its_own_work() {
    // The one end-to-end proof CI can afford: a coding worker, an independent
    // verification pass, FINISH, and a repository that really changed.
    let workspace = tempfile::tempdir().unwrap();
    let root = workspace.path().join("shipkit");
    std::fs::create_dir_all(&root).unwrap();
    fixture::materialize(&root).unwrap();

    let crew = Crew::sessions(
        WORKER_MODEL,
        agent::worker(worker(), &root).unwrap(),
        agent::verifier(verifier(), &root).unwrap(),
    );
    let outcome = Factory::new(
        fixture::JOB,
        &root,
        crew,
        Foreman::new(Readings::decisions().unwrap(), config().assessment_budget),
        config(),
    )
    .run()
    .await;

    assert_eq!(outcome.status, Status::Finished, "{:?}", outcome.failures);
    // One coding worker, then one independent verification pass.
    assert_eq!(outcome.workers.len(), 2);
    assert_eq!(outcome.verification.len(), 1);
    assert!(outcome.verification[0].passed);
    // Not the supervisor's opinion of the work — the work.
    assert!(fixture::changed(&root));
    assert!(fixture::verify(&root).iter().all(|check| check.passed));
    assert!(fixture::tests_pass(&root));
}

#[test]
fn the_scripted_edits_leave_a_repository_that_passes_its_own_suite() {
    // The scripted worker's edits are the same shell a live worker runs, so
    // running them here checks the fixture, the checks, and the scripts at
    // once — without starting a factory.
    let root = tempfile::tempdir().unwrap();
    fixture::materialize(root.path()).unwrap();
    assert!(fixture::tests_pass(root.path()), "the fixture starts green");

    for script in [
        include_str!("resources/write_rates.sh"),
        include_str!("resources/write_tests.sh"),
    ] {
        let status = Command::new("bash")
            .arg("-c")
            .arg(script)
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(status.status.success(), "scripted edit failed");
    }

    assert!(fixture::changed(root.path()));
    for check in fixture::verify(root.path()) {
        assert!(check.passed, "{}", check.label);
    }
    // And the suite it left behind still passes — including the boundaries
    // the job asked for.
    assert!(fixture::tests_pass(root.path()));
}
