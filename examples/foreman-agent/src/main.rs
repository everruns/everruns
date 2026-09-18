//! A classifier supervising a coding agent it never has to stop.
//!
//! A Framework port of [Foreman](https://github.com/thruwire/foreman): a fast
//! decision model placed above a slower coding agent. The worker keeps its own
//! reason/act loop; the supervisor watches the session's canonical events,
//! turns bounded evidence into nine probabilities in one request, and hands
//! them to a policy written in ordinary Rust.
//!
//! Run from the repository checkout; see README.md for credentials and modes.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use everruns::{Classifier, LlmSimConfig, Model, SimToolCall, SimTurn};
use everruns_example_demo::shell as demo;

mod agent;
mod factory;
mod fixture;
mod foreman;
mod observation;
mod policy;
mod terminal;

use factory::{Factory, Status};
use foreman::Foreman;
use policy::Config;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(std::env::args_os().skip(1))?;

    let temporary = options
        .workspace
        .is_none()
        .then(tempfile::tempdir)
        .transpose()?;
    let workspace = match (&options.workspace, &temporary) {
        (Some(path), _) => path.clone(),
        (None, Some(directory)) => directory.path().join("shipkit"),
        (None, None) => return Err("no workspace".into()),
    };
    fs::create_dir_all(&workspace)?;
    fixture::materialize(&workspace)?;

    let job = options
        .job
        .clone()
        .unwrap_or_else(|| fixture::JOB.to_owned());
    let mut config = if options.live_worker {
        Config::from_env()
    } else {
        simulated_config()
    };
    let (worker_model, verifier_model) = if options.live_worker {
        // OPENROUTER_API_KEY, declared by the OpenRouter driver itself.
        (
            Model::new(
                agent::WORKER_MODEL,
                everruns_openrouter::from_env("openrouter")?,
            ),
            Model::new(
                agent::WORKER_MODEL,
                everruns_openrouter::from_env("openrouter")?,
            ),
        )
    } else {
        (
            Model::simulated_with_config(worker_script()),
            Model::simulated_with_config(verifier_script()),
        )
    };
    let supervisor = if options.live_foreman {
        // TYPESAFE_API_KEY, declared by the TypeSafe integration.
        let classifier = Classifier::new(agent::FOREMAN_MODEL, everruns::TypeSafeAI::from_env()?);
        // A real classifier needs room to answer; a scripted worker does not
        // wait around for it, so the floor between readings goes up with it.
        config.assessment_budget = config.assessment_budget.max(Duration::from_secs(10));
        config.min_assessment_interval = config.min_assessment_interval.max(Duration::from_secs(2));
        Foreman::jev(classifier, config.assessment_budget)
    } else {
        Foreman::Rehearsed
    };

    let factory = Factory::new(
        &job,
        &workspace,
        agent::worker(worker_model, &workspace)?,
        agent::verifier(verifier_model, &workspace)?,
        supervisor,
        config,
    )
    .watched_by(Arc::new(terminal::Terminal::new()));

    demo::banner("everruns · foreman");
    demo::field(
        "worker",
        &format!("{} on the bashkit shell", worker_label(options.live_worker)),
    );
    demo::field(
        "foreman",
        &format!("{} — nine questions, one request", factory.foreman_model()),
    );
    demo::field("repository", &workspace.display().to_string());
    demo::field("run", factory.run_id());
    demo::field("job", &headline(&job));

    let outcome = factory.run().await;
    report(&outcome, &workspace);

    // The success condition is that the factory reached a decision, not that it
    // reached a particular one: escalation is the supervisor doing its job, and
    // a person is standing right here. Only a run that ran out of clock decided
    // nothing at all.
    match outcome.status {
        Status::TimedOut => Err("factory ran out of time without deciding".into()),
        Status::Finished if !fixture::changed(&workspace) => {
            Err("factory finished without changing the repository".into())
        }
        _ => Ok(()),
    }
}

/// The job on one line that fits beside its label.
fn headline(job: &str) -> String {
    const WIDTH: usize = 88;
    let job = job.split_whitespace().collect::<Vec<_>>().join(" ");
    if job.chars().count() <= WIDTH {
        return job;
    }
    format!("{}…", job.chars().take(WIDTH - 1).collect::<String>())
}

fn worker_label(live: bool) -> String {
    if live {
        agent::WORKER_MODEL.to_owned()
    } else {
        "scripted simulator".to_owned()
    }
}

/// Bounds for a scripted worker, which finishes in seconds rather than minutes.
///
/// Only the clocks move, and they are pinned rather than overridable because
/// the scripted worker's own timing is fixed. Every threshold and every budget
/// is what a live run uses, `FOREMAN_*` overrides included, because the policy
/// is the part worth seeing unchanged.
fn simulated_config() -> Config {
    Config {
        min_assessment_interval: Duration::from_millis(1_000),
        periodic_assessment: Duration::from_millis(2_500),
        overall_timeout: Duration::from_secs(180),
        ..Config::from_env()
    }
}

fn worker_script() -> LlmSimConfig {
    let step = |text: &str, commands: &str, id: &str| SimTurn::Mixed {
        text: text.to_owned(),
        tool_calls: vec![SimToolCall {
            name: "bash".to_owned(),
            arguments: serde_json::json!({ "commands": commands }),
            id: Some(id.to_owned()),
        }],
    };
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
        // A second coding pass, for the run where the policy asks for one after
        // verification. A worker started on a finished repository should read
        // it and say so, not repeat its own summary.
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
    .with_response_delay(Duration::from_millis(900))
}

fn verifier_script() -> LlmSimConfig {
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
    .with_response_delay(Duration::from_millis(900))
}

fn report(outcome: &factory::Outcome, workspace: &std::path::Path) {
    demo::section("FACTORY");
    demo::field("status", outcome.status.label());
    demo::field("readings", &outcome.iterations.to_string());
    demo::field("workers", &outcome.workers.len().to_string());
    demo::field("elapsed", &format!("{:.0}s", outcome.elapsed.as_secs_f64()));
    if let Some(intervention) = &outcome.last_intervention {
        demo::field(
            "decision",
            &format!("{} — {}", intervention.action.label(), intervention.reason),
        );
    }
    if outcome.status == Status::Escalated {
        demo::field("asking for", "a person — see the decision above");
    }
    if let Some(assessment) = &outcome.last_assessment {
        demo::field(
            "final reading",
            &format!(
                "ready_to_finish {:.2} · requirements_satisfied {:.2} · tests_sufficient {:.2}",
                assessment.ready_to_finish,
                assessment.requirements_satisfied,
                assessment.tests_sufficient,
            ),
        );
    }
    for failure in &outcome.failures {
        demo::field("failure", failure);
    }

    if !outcome.verification.is_empty() {
        demo::section("INDEPENDENT VERIFICATION");
        for result in &outcome.verification {
            demo::check(result.passed, &result.worker_id);
            demo::body(result.summary.trim(), demo::DIM);
        }
    }

    demo::section("REPOSITORY ON DISK");
    for check in fixture::verify(workspace) {
        demo::check(check.passed, check.label);
    }
}

#[derive(Debug)]
struct Options {
    live_worker: bool,
    live_foreman: bool,
    job: Option<String>,
    workspace: Option<PathBuf>,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut live_worker = false;
        let mut live_foreman = false;
        let mut job = None;
        let mut workspace = None;
        let mut arguments = arguments.into_iter();
        while let Some(argument) = arguments.next() {
            if argument == "--live" {
                live_worker = true;
                live_foreman = true;
            } else if argument == "--live-foreman" {
                live_foreman = true;
            } else if argument == "--job" {
                let value = arguments
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .filter(|value| !value.trim().is_empty())
                    .ok_or_else(|| invalid("--job needs a job description"))?;
                job = Some(value.trim().to_owned());
            } else if workspace.is_none() && !argument.to_string_lossy().starts_with("--") {
                workspace = Some(PathBuf::from(argument));
            } else {
                return Err(invalid(
                    "usage: foreman-agent [--live | --live-foreman] [--job JOB] [WORKSPACE]",
                ));
            }
        }
        Ok(Self {
            live_worker,
            live_foreman,
            job,
            workspace,
        })
    }
}

fn invalid(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> io::Result<Options> {
        Options::parse(arguments.iter().map(OsString::from))
    }

    #[test]
    fn arguments_parse_in_any_order() {
        let options = parse(&["--job", "  Fix the rates.  ", "/tmp/shipkit", "--live"]).unwrap();
        assert!(options.live_worker);
        assert!(options.live_foreman);
        assert_eq!(options.job.as_deref(), Some("Fix the rates."));
        assert_eq!(options.workspace, Some(PathBuf::from("/tmp/shipkit")));
    }

    #[test]
    fn the_supervisor_can_be_live_while_the_worker_is_not() {
        // Supervision is the cheap half: a real classifier can watch a scripted
        // worker, which is what makes the recorded demo a real reading.
        let options = parse(&["--live-foreman"]).unwrap();
        assert!(!options.live_worker);
        assert!(options.live_foreman);
    }

    #[test]
    fn defaults_are_an_offline_run_on_a_temporary_workspace() {
        let options = parse(&[]).unwrap();
        assert!(!options.live_worker);
        assert!(!options.live_foreman);
        assert!(options.job.is_none());
        assert!(options.workspace.is_none());
    }

    #[test]
    fn the_headline_is_one_line_that_fits_beside_its_label() {
        assert_eq!(headline("Add\n  tiers."), "Add tiers.");
        let headline = headline(fixture::JOB);
        assert!(headline.chars().count() <= 88);
        assert!(!headline.contains('\n'));
    }

    #[test]
    fn a_job_flag_without_a_job_is_rejected() {
        assert_eq!(
            parse(&["--job"]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            parse(&["--job", "   "]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            parse(&["--unknown"]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

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

    #[tokio::test]
    async fn the_offline_run_finishes_after_verifying_its_own_work() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("shipkit");
        fs::create_dir_all(&root).unwrap();
        fixture::materialize(&root).unwrap();

        let outcome = Factory::new(
            fixture::JOB,
            &root,
            agent::worker(Model::simulated_with_config(worker_script()), &root).unwrap(),
            agent::verifier(Model::simulated_with_config(verifier_script()), &root).unwrap(),
            Foreman::Rehearsed,
            simulated_config(),
        )
        .run()
        .await;

        assert_eq!(outcome.status, Status::Finished, "{:?}", outcome.failures);
        // One coding worker, then one independent verification pass.
        assert_eq!(outcome.workers.len(), 2);
        assert_eq!(outcome.verification.len(), 1);
        assert!(outcome.verification[0].passed);
        assert!(fixture::changed(&root));
        assert!(fixture::verify(&root).iter().all(|check| check.passed));
    }
}
