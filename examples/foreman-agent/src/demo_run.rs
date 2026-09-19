//! The deterministic demo: the same runtime, with nothing to pay for.
//!
//! A scripted worker really edits the fixture through the same Bashkit
//! capability a live worker uses, and the supervisor reads the evidence it
//! leaves behind. Only the two ends are simulated; everything between them —
//! the event stream, the observation, the policy, the interventions — is the
//! code a live run executes.

use std::collections::BTreeMap;
use std::time::Duration;

use async_trait::async_trait;
use everruns::{
    AgentLoopError, ClassificationAnswer, ClassificationOutcome, ClassificationRequest, Classifier,
    ClassifierService, LlmSimConfig, Model, SimToolCall, SimTurn,
};
use serde_json::Value;

use crate::policy::Config;

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
                "Reading the rate table and the tests that cover it.",
                "cat src/rates.py; ls tests",
                "call_read",
            ),
            step(
                "Replacing the flat rate with weight tiers.",
                include_str!("resources/demo/write_rates.sh"),
                "call_rates",
            ),
            step(
                "Covering the tier boundaries, which is where tiered pricing goes wrong.",
                include_str!("resources/demo/write_tests.sh"),
                "call_tests",
            ),
            SimTurn::Assistant(
                "Weight tiers replace the flat rate, and the boundary at each tier edge is \
                 covered by a test. The zone surcharge is unchanged."
                    .to_owned(),
            ),
            // A second coding pass, for the run where the policy asks for one
            // after verification. A worker started on a finished repository
            // should read it and say so, not repeat its own summary.
            step(
                "Re-reading what the last pass left behind.",
                "sed -n '1,20p' src/rates.py; sed -n '1,14p' tests/test_tiers.py",
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
                        "commands": "cat src/rates.py; cat tests/test_tiers.py"
                    }),
                    id: Some("call_verify".to_owned()),
                }],
            },
            SimTurn::Assistant(
                "src/rates.py prices by weight and tests/test_tiers.py pins both tier edges. \
                 The job is satisfied."
                    .to_owned(),
            ),
        ])
        .with_response_delay(Duration::from_millis(900)),
    )
}

/// The supervisor the demo runs against, as a service rather than a fork in
/// the code.
///
/// [`ClassifierService`] is the Framework's seam for "answer these questions
/// without asking a vendor", so the demo uses it instead of teaching
/// [`Foreman`](crate::foreman::Foreman) about a second kind of supervisor. The
/// code under test is then exactly the code a live run executes: the same nine
/// questions, the same request, the same parsing of the answers.
///
/// What it answers is a table in `resources/demo/readings.json`, keyed by what
/// is on the floor rather than by how many times it has been asked — so the
/// demo walks the same CONTINUE → START_VERIFIER → FINISH path whether the
/// worker takes one second or ten.
pub struct Rehearsed {
    readings: BTreeMap<String, BTreeMap<String, f64>>,
}

/// The model name the demo reports, since nothing resolves one for it.
pub const REHEARSED_MODEL: &str = "rehearsed";

impl Rehearsed {
    /// Load the readings table.
    pub fn load() -> Result<Self, serde_json::Error> {
        serde_json::from_str(include_str!("resources/demo/readings.json"))
            .map(|readings| Self { readings })
    }

    /// A classifier backed by this service, ready for a `Foreman`.
    pub fn classifier() -> Result<Classifier, serde_json::Error> {
        Ok(Classifier::new(REHEARSED_MODEL, Self::load()?))
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
impl ClassifierService for Rehearsed {
    fn is_configured(&self) -> bool {
        true
    }

    fn name(&self) -> &'static str {
        REHEARSED_MODEL
    }

    async fn evaluate(
        &self,
        request: ClassificationRequest,
    ) -> Result<ClassificationOutcome, AgentLoopError> {
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
                Ok((id.clone(), ClassificationAnswer::Noul { probability }))
            })
            .collect::<Result<BTreeMap<_, _>, AgentLoopError>>()?;
        Ok(ClassificationOutcome {
            model: REHEARSED_MODEL.to_owned(),
            answers,
            ..ClassificationOutcome::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::observation::Observation;

    #[test]
    fn the_scripted_worker_writes_the_files_the_job_asks_for() {
        // The demo's edits are real shell, run by the same capability a live
        // worker uses; keeping them in files means they can be read and run.
        let rates = include_str!("resources/demo/write_rates.sh");
        let tests = include_str!("resources/demo/write_tests.sh");
        assert!(rates.contains("src/rates.py"));
        assert!(tests.contains("tests/test_tiers.py"));
        assert!(tests.to_lowercase().contains("boundar"));
    }

    #[test]
    fn every_reading_answers_every_question() {
        // A table missing an id would fail a run halfway through; catching it
        // here costs nothing.
        let readings = Rehearsed::load().unwrap();
        for (phase, reading) in &readings.readings {
            for dimension in &crate::foreman::DIMENSIONS {
                let value = reading
                    .get(dimension.id)
                    .unwrap_or_else(|| panic!("{phase} has no {}", dimension.id));
                assert!((0.0..=1.0).contains(value), "{phase}/{}", dimension.id);
            }
            assert_eq!(reading.len(), crate::foreman::DIMENSIONS.len(), "{phase}");
        }
    }

    #[tokio::test]
    async fn the_rehearsed_service_answers_the_nine_questions() {
        let foreman =
            crate::foreman::Foreman::new(Rehearsed::classifier().unwrap(), Duration::from_secs(5));
        let assessment = foreman.assess(&Observation::sample()).await.unwrap();
        // The sample observation has a worker on the floor and nothing changed
        // yet, so this is the opening reading.
        assert_eq!(assessment.implementation_complete, 0.31);
        assert_eq!(assessment.meaningful_progress, 0.88);
    }

    #[test]
    fn the_phase_follows_the_floor_not_the_call_count() {
        let phase = |observation: &Observation| {
            Rehearsed::phase(&serde_json::to_value(observation).unwrap())
        };
        let mut observation = Observation::sample();
        assert!(!observation.active_workers.is_empty());
        assert_eq!(phase(&observation), "started");

        observation.changed_files.push("src/rates.py".into());
        assert_eq!(phase(&observation), "editing");

        observation.active_workers.clear();
        assert_eq!(phase(&observation), "unchecked");

        observation
            .verification_results
            .push(crate::observation::VerificationResult {
                worker_id: "worker-2".into(),
                passed: true,
                summary: "checked".into(),
            });
        assert_eq!(phase(&observation), "verified");
    }

    #[test]
    fn demo_clocks_are_pinned_but_budgets_are_not() {
        let config = config();
        assert_eq!(config.min_assessment_interval, Duration::from_millis(1_000));
        // Thresholds and budgets stay exactly what a live run uses.
        assert_eq!(config.finish, Config::default().finish);
        assert_eq!(config.max_workers, Config::from_env().max_workers);
    }
}
