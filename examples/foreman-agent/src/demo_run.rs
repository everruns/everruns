//! The deterministic demo: the same runtime, with nothing to pay for.
//!
//! A scripted worker really edits the fixture through the same Bashkit
//! capability a live worker uses, and the supervisor reads the evidence it
//! leaves behind. Only the two ends are simulated; everything between them —
//! the event stream, the observation, the policy, the interventions — is the
//! code a live run executes.

use std::time::Duration;

use everruns::{LlmSimConfig, Model, SimToolCall, SimTurn};

use crate::foreman::Assessment;
use crate::observation::Observation;
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

/// What a supervisor would plausibly say about this evidence.
///
/// These numbers are demo collateral, not supervisor behavior, so they live
/// here beside the scripted worker rather than inside [`Foreman`].
///
/// Three readings, chosen by what is on the floor rather than by how many times
/// it has been asked: work under way, work finished but unchecked, and work an
/// independent pass has now looked at. The offline demo therefore walks the
/// same CONTINUE → START_VERIFIER → FINISH path whether the worker takes one
/// second or ten.
pub fn reading(observation: &Observation) -> Assessment {
    let verified = observation
        .verification_results
        .iter()
        .any(|result| result.passed);
    let working = !observation.active_workers.is_empty();
    let touched = !observation.changed_files.is_empty();

    if verified {
        Assessment {
            implementation_complete: 0.98,
            tests_sufficient: 0.96,
            requirements_satisfied: 0.97,
            needs_verification: 0.04,
            ready_to_finish: 0.98,
            meaningful_progress: 0.98,
            worker_stuck: 0.00,
            work_off_track: 0.01,
            needs_human: 0.01,
        }
    } else if working {
        Assessment {
            implementation_complete: if touched { 0.72 } else { 0.31 },
            tests_sufficient: if touched { 0.28 } else { 0.10 },
            requirements_satisfied: if touched { 0.61 } else { 0.22 },
            needs_verification: if touched { 0.44 } else { 0.11 },
            ready_to_finish: if touched { 0.18 } else { 0.02 },
            meaningful_progress: if touched { 0.94 } else { 0.88 },
            worker_stuck: 0.03,
            work_off_track: 0.02,
            needs_human: 0.01,
        }
    } else {
        Assessment {
            implementation_complete: 0.96,
            tests_sufficient: 0.91,
            requirements_satisfied: 0.92,
            needs_verification: 0.93,
            ready_to_finish: 0.68,
            meaningful_progress: 0.95,
            worker_stuck: 0.01,
            work_off_track: 0.01,
            needs_human: 0.01,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn the_reading_follows_the_floor_not_the_call_count() {
        let mut observation = Observation::sample();
        assert!(!observation.active_workers.is_empty());
        assert!(reading(&observation).ready_to_finish < 0.5);

        observation.active_workers.clear();
        let idle = reading(&observation);
        assert!(idle.needs_verification > 0.65);
        assert!(idle.implementation_complete > 0.75);

        observation
            .verification_results
            .push(crate::observation::VerificationResult {
                worker_id: "worker-2".into(),
                passed: true,
                summary: "checked".into(),
            });
        let verified = reading(&observation);
        assert!(verified.ready_to_finish > 0.85);
        assert!(verified.needs_verification < 0.65);
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
