//! A classifier supervising a coding agent it never has to stop.
//!
//! A Framework port of [Foreman](https://github.com/thruwire/foreman): a fast
//! decision model placed above a slower coding agent. The worker keeps its own
//! loop — an Everruns session, or an external CLI like Codex or yolop — while
//! the supervisor turns bounded evidence into nine probabilities in one request
//! and hands them to a policy written in ordinary Rust.
//!
//! ```text
//! foreman run --repo ./my-project --job "Add rate limiting, and test it."
//! foreman demo
//! ```
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use clap::Parser;
use everruns::{Classifier, Model};
use everruns_example_demo::shell as demo;

mod agent;
mod cli;
mod demo_run;
mod factory;
mod fixture;
mod foreman;
mod observation;
mod policy;
mod terminal;
mod worker;

use cli::{Cli, Command, Demo, Run};
use factory::{Factory, Outcome, Status};
use foreman::Foreman;
use policy::Config;
use worker::Crew;

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run(run) => start(run).await,
        Command::Demo(demo) => rehearse(demo).await,
    }
}

/// Supervise real work in a repository the caller named.
///
/// The fixture is never materialized here: `--repo` is somebody's project, and
/// the only thing that writes to it is the worker.
async fn start(run: Run) -> Result<()> {
    if !run.repo.is_dir() {
        bail!("{} is not a directory", run.repo.display());
    }
    let job = run.job.trim();
    if job.is_empty() {
        bail!("a job cannot be empty");
    }

    let config = Config::from_env();
    let crew = match run.external().context("reading --worker-command")? {
        Some(external) => Crew::External(external),
        None => sessions(&run.repo)?,
    };
    // TYPESAFE_API_KEY, declared by the TypeSafe integration.
    let classifier = Classifier::new(agent::FOREMAN_MODEL, everruns::TypeSafeAI::from_env()?);
    let foreman = Foreman::jev(classifier, config.assessment_budget);

    let outcome = supervise(job, &run.repo, crew, foreman, config).await;
    report(&outcome);
    settle(&outcome)
}

/// Walk the same runtime over a disposable fixture.
async fn rehearse(options: Demo) -> Result<()> {
    let temporary = options
        .repo
        .is_none()
        .then(tempfile::tempdir)
        .transpose()
        .context("creating a temporary workspace")?;
    let workspace = match (&options.repo, &temporary) {
        (Some(path), _) => path.clone(),
        (None, Some(directory)) => directory.path().join("shipkit"),
        (None, None) => bail!("no workspace"),
    };
    std::fs::create_dir_all(&workspace)?;
    fixture::materialize(&workspace).context("materializing the fixture")?;

    let mut config = demo_run::config();
    let foreman = if options.live_foreman {
        let classifier = Classifier::new(agent::FOREMAN_MODEL, everruns::TypeSafeAI::from_env()?);
        // A real classifier needs room to answer, and a scripted worker does
        // not wait around for it, so the floor between readings goes up too.
        config.assessment_budget = config
            .assessment_budget
            .max(std::time::Duration::from_secs(10));
        config.min_assessment_interval = config
            .min_assessment_interval
            .max(std::time::Duration::from_secs(2));
        Foreman::jev(classifier, config.assessment_budget)
    } else {
        Foreman::answering("rehearsed", |observation| {
            Ok(demo_run::reading(observation))
        })
    };
    let crew = Crew::sessions(
        agent::worker(demo_run::worker(), &workspace)?,
        agent::verifier(demo_run::verifier(), &workspace)?,
    );

    let outcome = supervise(fixture::JOB, &workspace, crew, foreman, config).await;
    report(&outcome);

    demo::section("REPOSITORY ON DISK");
    for check in fixture::verify(&workspace) {
        demo::check(check.passed, check.label);
    }
    if outcome.status == Status::Finished && !fixture::changed(&workspace) {
        bail!("factory finished without changing the repository");
    }
    settle(&outcome)
}

/// Everruns sessions over `repo`: one that may write, one that may not.
fn sessions(repo: &Path) -> Result<Crew> {
    // OPENROUTER_API_KEY, declared by the OpenRouter driver itself.
    let model = || -> Result<Model> {
        Ok(Model::new(
            agent::WORKER_MODEL,
            everruns_openrouter::from_env("openrouter")?,
        ))
    };
    Ok(Crew::sessions(
        agent::worker(model()?, repo)?,
        agent::verifier(model()?, repo)?,
    ))
}

/// Run one factory, rendered.
async fn supervise(
    job: &str,
    workspace: &Path,
    crew: Crew,
    foreman: Foreman,
    config: Config,
) -> Outcome {
    let factory = Factory::new(job, workspace, crew, foreman, config)
        .watched_by(Arc::new(terminal::Terminal::new()));

    demo::banner("everruns · foreman");
    demo::field("worker", &factory.crew_label());
    demo::field(
        "foreman",
        &format!("{} — nine questions, one request", factory.foreman_model()),
    );
    demo::field("repository", &workspace.display().to_string());
    demo::field("run", factory.run_id());
    demo::field("job", &headline(job));

    factory.run().await
}

/// The exit status is whether the factory decided, not which way.
///
/// Escalation is the supervisor doing its job, and a person is standing right
/// here. Only a run that ran out of clock decided nothing at all.
fn settle(outcome: &Outcome) -> Result<()> {
    match outcome.status {
        Status::TimedOut => bail!("factory ran out of time without deciding"),
        _ => Ok(()),
    }
}

fn report(outcome: &Outcome) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_headline_is_one_line_that_fits_beside_its_label() {
        assert_eq!(headline("Add\n  tiers."), "Add tiers.");
        let headline = headline(fixture::JOB);
        assert!(headline.chars().count() <= 88);
        assert!(!headline.contains('\n'));
    }

    #[tokio::test]
    async fn the_demo_finishes_after_verifying_its_own_work() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("shipkit");
        std::fs::create_dir_all(&root).unwrap();
        fixture::materialize(&root).unwrap();

        let crew = Crew::sessions(
            agent::worker(demo_run::worker(), &root).unwrap(),
            agent::verifier(demo_run::verifier(), &root).unwrap(),
        );
        let outcome = Factory::new(
            fixture::JOB,
            &root,
            crew,
            Foreman::answering("rehearsed", |o| Ok(demo_run::reading(o))),
            demo_run::config(),
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
