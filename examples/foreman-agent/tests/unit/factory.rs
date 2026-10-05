use super::*;

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use everruns::{
    AgentLoopError, DecisionAnswer, DecisionOutcome, DecisionRequest, Decisions, DecisionsService,
    LlmSimConfig, Model,
};
use serde_json::Value;

use crate::foreman::DIMENSIONS;
use crate::worker::ExternalAgent;

/// A decision service that answers from a closure over the observation.
///
/// The stub receives the state as JSON, exactly as a vendor's service does,
/// so a test drives the run through the same request and parsing a live
/// reading uses.
/// How a test answers one reading.
type Reply = Box<dyn Fn(&Value) -> Result<Assessment, AgentLoopError> + Send + Sync>;

struct Answering(Reply);

#[async_trait::async_trait]
impl DecisionsService for Answering {
    fn is_configured(&self) -> bool {
        true
    }

    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome, AgentLoopError> {
        let assessment = (self.0)(&request.state)?;
        Ok(DecisionOutcome {
            model: "test".to_owned(),
            answers: DIMENSIONS
                .iter()
                .map(|dimension| {
                    (
                        dimension.id.to_owned(),
                        DecisionAnswer::Noul {
                            probability: assessment.value(dimension.id),
                        },
                    )
                })
                .collect::<BTreeMap<_, _>>(),
            ..DecisionOutcome::default()
        })
    }
}

fn answering(
    answer: impl Fn(&Value) -> Result<Assessment, AgentLoopError> + Send + Sync + 'static,
) -> Foreman {
    Foreman::new(
        Decisions::new("test", Answering(Box::new(answer))),
        Duration::from_secs(5),
    )
}

/// Whether a worker was still on the floor when this reading was taken.
fn working(state: &Value) -> bool {
    state
        .get("active_workers")
        .and_then(Value::as_array)
        .is_some_and(|workers| !workers.is_empty())
}

/// A worker that takes long enough that "during" is unambiguous.
fn slow_worker() -> Model {
    Model::simulated_with_config(
        LlmSimConfig::fixed("Finished.").with_response_delay(Duration::from_secs(2)),
    )
}

fn sessions(root: &std::path::Path, model: impl Fn() -> Model) -> Crew {
    Crew::sessions(
        "simulated",
        crate::agent::worker(model(), root).unwrap(),
        crate::agent::verifier(model(), root).unwrap(),
    )
}

/// Quick clocks, one worker: the run ends as soon as that worker does.
fn brisk() -> Config {
    Config {
        min_assessment_interval: Duration::from_millis(100),
        periodic_assessment: Duration::from_millis(200),
        overall_timeout: Duration::from_secs(30),
        max_workers: 1,
        ..Config::default()
    }
}

fn healthy() -> Assessment {
    Assessment {
        meaningful_progress: 0.9,
        ..Assessment::default()
    }
}

#[tokio::test]
async fn readings_land_while_the_worker_is_still_working() {
    // The whole architectural claim: supervision runs *during* the work,
    // not after it. A reading that sees an active worker is a reading taken
    // before that worker's turn resolved.
    let workspace = tempfile::tempdir().unwrap();
    let live = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&live);
    let foreman = answering(move |state| {
        if working(state) {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        Ok(healthy())
    });

    let outcome = Factory::new(
        "Take your time.",
        workspace.path(),
        sessions(workspace.path(), slow_worker),
        foreman,
        brisk(),
    )
    .run()
    .await;

    let live = live.load(Ordering::SeqCst);
    assert!(
        live >= 2,
        "expected several readings during the turn, got {live}"
    );
    // And the worker really did keep working through them.
    assert!(outcome.workers[0].elapsed() >= Duration::from_secs(2));
    assert_eq!(outcome.workers[0].status, WorkerStatus::Completed);
}

#[tokio::test]
async fn a_stuck_worker_is_stopped_retried_once_and_then_escalated() {
    let workspace = tempfile::tempdir().unwrap();
    // One reading, held for the whole run: the worker is stuck. The policy
    // still has to walk stop → retry → escalate rather than repeat itself.
    let foreman = answering(|_| {
        Ok(Assessment {
            worker_stuck: 0.95,
            meaningful_progress: 0.05,
            ..Assessment::default()
        })
    });
    let config = Config {
        min_assessment_interval: Duration::from_millis(50),
        periodic_assessment: Duration::from_millis(150),
        overall_timeout: Duration::from_secs(20),
        max_retries: 1,
        ..Config::default()
    };
    let endless = || {
        Model::simulated_with_config(
            LlmSimConfig::fixed("Still working on it.")
                .with_response_delay(Duration::from_secs(30)),
        )
    };

    let outcome = Factory::new(
        "Keep going forever.",
        workspace.path(),
        sessions(workspace.path(), endless),
        foreman,
        config,
    )
    .run()
    .await;

    assert_eq!(outcome.status, Status::Escalated);
    // The original worker, and exactly one fresh attempt after it.
    assert_eq!(outcome.workers.len(), 2);
    assert!(outcome.workers.iter().all(|worker| !worker.is_active()));
    assert_eq!(
        outcome.last_intervention.map(|i| i.action),
        Some(Action::Escalate)
    );
}

#[tokio::test]
async fn a_supervisor_that_cannot_see_escalates_rather_than_guessing() {
    let workspace = tempfile::tempdir().unwrap();
    let outcome = Factory::new(
        "Anything.",
        workspace.path(),
        sessions(workspace.path(), slow_worker),
        answering(|_| Err(AgentLoopError::llm("decision service is down"))),
        brisk(),
    )
    .run()
    .await;

    assert_eq!(outcome.status, Status::Escalated);
    assert!(!outcome.failures.is_empty());
    assert!(outcome.workers.iter().all(|worker| !worker.is_active()));
}

fn external_crew(root: &std::path::Path, worker: ExternalAgent) -> Crew {
    Crew::external(
        worker,
        "simulated",
        crate::agent::verifier(Model::simulated("Checked."), root).unwrap(),
    )
}

/// A stand-in for Codex or yolop: a real child process that streams.
fn stand_in(script: &str) -> ExternalAgent {
    let argv: Vec<String> = ["bash", "-c", script, "foreman-worker", "{mission}"]
        .iter()
        .map(|word| (*word).to_owned())
        .collect();
    ExternalAgent {
        label: "stand-in".to_owned(),
        coding: argv.clone(),
        credential_environment: &[],
    }
}

#[tokio::test]
async fn an_external_worker_is_watched_while_it_runs() {
    // Neither Codex nor yolop is installed in CI, and neither needs to be:
    // what this proves is the path they travel — a child process whose
    // output reaches the supervisor before the process exits.
    let workspace = tempfile::tempdir().unwrap();
    let live = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&live);
    let foreman = answering(move |state| {
        if working(state) {
            counter.fetch_add(1, Ordering::SeqCst);
        }
        Ok(healthy())
    });

    let outcome = Factory::new(
        "Do the thing.",
        workspace.path(),
        external_crew(
            workspace.path(),
            stand_in(r#"printf '{"type":"read_file"}\n'; sleep 2; printf 'done\n'"#),
        ),
        foreman,
        brisk(),
    )
    .run()
    .await;

    assert!(
        live.load(Ordering::SeqCst) >= 2,
        "an external worker must be observable while it runs"
    );
    let worker = &outcome.workers[0];
    assert_eq!(worker.status, WorkerStatus::Completed);
    assert!(worker.output.as_str().contains("done"));
    // A JSONL line names its own step, so it counts as one.
    assert_eq!(worker.last_tool.as_deref(), Some("read_file"));
    // The mission reached the process as one argument.
    assert!(worker.output.as_str().contains("Do the thing.") || worker.tool_calls == 1);
}

#[tokio::test]
async fn a_missing_external_agent_fails_by_name_rather_than_hanging() {
    let workspace = tempfile::tempdir().unwrap();
    let mut agent = stand_in("true");
    "no-such-coding-agent-on-this-machine".clone_into(&mut agent.coding[0]);

    let outcome = Factory::new(
        "Do the thing.",
        workspace.path(),
        external_crew(workspace.path(), agent),
        answering(|_| Ok(healthy())),
        brisk(),
    )
    .run()
    .await;

    assert_eq!(outcome.workers[0].status, WorkerStatus::Failed);
    let failure = outcome.failures.join(" ");
    assert!(
        failure.contains("no-such-coding-agent-on-this-machine"),
        "the failure should name the missing binary: {failure}"
    );
}

#[tokio::test]
async fn an_external_worker_is_killed_when_the_policy_stops_it() {
    let workspace = tempfile::tempdir().unwrap();
    let outcome = Factory::new(
        "Loop forever.",
        workspace.path(),
        // Nothing about this process ends on its own.
        external_crew(workspace.path(), stand_in("while true; do sleep 1; done")),
        answering(|_| {
            Ok(Assessment {
                worker_stuck: 0.95,
                ..Assessment::default()
            })
        }),
        Config {
            min_assessment_interval: Duration::from_millis(50),
            periodic_assessment: Duration::from_millis(150),
            overall_timeout: Duration::from_secs(20),
            max_retries: 0,
            ..Config::default()
        },
    )
    .run()
    .await;

    assert_eq!(outcome.status, Status::Escalated);
    assert_eq!(outcome.workers[0].status, WorkerStatus::Stopped);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_verifier_that_finishes_mid_reading_is_recorded_not_escalated() {
    // A worker's record flips to finished before its `Finished` signal is
    // absorbed. A reading that straddles that gap must still count the
    // verifier as on the floor, or the policy sees "verification started,
    // nothing active, nothing completed" and escalates, dropping the result.
    let workspace = tempfile::tempdir().unwrap();
    let verifying = |state: &Value| {
        state
            .get("active_workers")
            .and_then(Value::as_array)
            .is_some_and(|workers| {
                workers
                    .iter()
                    .any(|worker| worker["worker_kind"] == "verifier")
            })
    };
    let verified = |state: &Value| {
        state
            .get("verification_results")
            .and_then(Value::as_array)
            .is_some_and(|results| results.iter().any(|result| result["passed"] == true))
    };
    let foreman = answering(move |state| {
        if verifying(state) {
            // Hold the reading open until the verifier's turn has resolved.
            std::thread::sleep(Duration::from_millis(1_500));
            return Ok(healthy());
        }
        let done = if verified(state) { 0.99 } else { 0.5 };
        Ok(Assessment {
            implementation_complete: 0.95,
            needs_verification: 0.95,
            ready_to_finish: done,
            requirements_satisfied: done,
            tests_sufficient: done,
            ..healthy()
        })
    });
    let crew = Crew::external(
        stand_in("true"),
        "simulated",
        crate::agent::verifier(
            Model::simulated_with_config(
                LlmSimConfig::fixed("Checked.").with_response_delay(Duration::from_millis(500)),
            ),
            workspace.path(),
        )
        .unwrap(),
    );

    let outcome = Factory::new(
        "Do the thing.",
        workspace.path(),
        crew,
        foreman,
        Config {
            max_workers: 2,
            ..brisk()
        },
    )
    .run()
    .await;

    assert_eq!(outcome.status, Status::Finished, "{:?}", outcome.failures);
    assert_eq!(outcome.verification.len(), 1);
    assert!(outcome.verification[0].passed);
}

#[test]
fn remaining_time_shrinks_and_never_goes_negative() {
    let budget = Duration::from_secs(30);
    assert_eq!(remaining(budget, None), Duration::ZERO);
    assert_eq!(
        remaining(budget, Some(Duration::from_secs(10))),
        Duration::from_secs(20)
    );
    assert_eq!(
        remaining(budget, Some(Duration::from_secs(90))),
        Duration::ZERO
    );
}

#[test]
fn the_active_worker_is_the_one_still_running() {
    let mut workers = vec![
        WorkerRecord::new("worker-1", WorkerKind::Coding, 1, 100),
        WorkerRecord::new("worker-2", WorkerKind::Coding, 2, 100),
    ];
    workers[0].status = WorkerStatus::Completed;
    assert_eq!(active_id(&workers).as_deref(), Some("worker-2"));

    workers[1].status = WorkerStatus::Stopped;
    assert_eq!(active_id(&workers), None);
    assert!(active_ids(&workers).is_empty());
}

#[test]
fn the_event_window_is_bounded() {
    let events = Mutex::new(VecDeque::new());
    for n in 0..500 {
        note(&events, format!("event-{n}"));
    }
    let events = lock(&events);
    assert_eq!(events.len(), 200);
    assert_eq!(events.back().map(String::as_str), Some("event-499"));
}

#[test]
fn run_ids_are_short_and_distinct_enough_to_name_a_run() {
    let first = run_id();
    std::thread::sleep(Duration::from_millis(2));
    assert_eq!(first.len(), 12);
    assert_ne!(first, run_id());
}

#[cfg(unix)]
#[tokio::test]
async fn stopping_an_external_worker_stops_children_holding_output_pipes() {
    let workspace = tempfile::tempdir().unwrap();
    let outcome = Factory::new(
        "Stop the process tree.",
        workspace.path(),
        external_crew(workspace.path(), stand_in("sleep 60 & wait")),
        answering(|_| {
            Ok(Assessment {
                worker_stuck: 0.95,
                ..Assessment::default()
            })
        }),
        Config {
            overall_timeout: Duration::from_secs(2),
            max_retries: 0,
            ..brisk()
        },
    )
    .run()
    .await;
    assert_eq!(outcome.status, Status::Escalated);
    assert_eq!(outcome.workers[0].status, WorkerStatus::Stopped);
}

#[tokio::test]
async fn completed_tool_output_reaches_the_supervisor_during_the_turn() {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("observation.txt"),
        "result-only-evidence",
    )
    .unwrap();
    let model = || {
        Model::simulated_with_config(
            LlmSimConfig::scripted(vec![
                everruns::SimTurn::ToolCalls(vec![everruns::SimToolCall {
                    name: "bash".to_owned(),
                    arguments: serde_json::json!({"commands": "cat observation.txt"}),
                    id: Some("read_evidence".to_owned()),
                }]),
                everruns::SimTurn::Assistant("Done.".to_owned()),
            ])
            .with_response_delay(Duration::from_millis(300)),
        )
    };
    let seen = Arc::new(AtomicUsize::new(0));
    let recorded = Arc::clone(&seen);
    Factory::new(
        "Read a file.",
        workspace.path(),
        sessions(workspace.path(), model),
        answering(move |state| {
            if working(state)
                && state["latest_worker_output"].as_str().is_some_and(|text| {
                    text.contains("result-only-evidence") && text.contains("exit_code")
                })
            {
                recorded.fetch_add(1, Ordering::SeqCst);
            }
            Ok(healthy())
        }),
        brisk(),
    )
    .run()
    .await;
    assert!(
        seen.load(Ordering::SeqCst) > 0,
        "completed tool result and exit status were missing from live evidence"
    );
}
