//! `foreman run --repo … --job …`, the way Foreman itself is started.
//!
//! The binary is only the wiring: read the arguments, build the crew and the
//! supervisor from credentials in the environment, and hand both to
//! [`everruns_foreman_agent::run::supervise`]. Everything it needs is in the
//! library beside it.

use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Parser;
use everruns::{Classifier, Model};
use everruns_foreman_agent::cli::{Cli, Command, Run};
use everruns_foreman_agent::foreman::Foreman;
use everruns_foreman_agent::policy::Config;
use everruns_foreman_agent::worker::Crew;
use everruns_foreman_agent::{agent, run};

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run(options) => start(options).await,
    }
}

/// Supervise real work in a repository the caller named.
///
/// Nothing is materialized here: `--repo` is somebody's project, and the only
/// thing that writes to it is the worker.
async fn start(options: Run) -> Result<()> {
    if !options.repo.is_dir() {
        bail!("{} is not a directory", options.repo.display());
    }
    let job = options.job.trim();
    if job.is_empty() {
        bail!("a job cannot be empty");
    }

    let config = Config::from_env();
    let crew = match options.external().context("reading --worker-command")? {
        Some(external) => Crew::External(external),
        None => sessions(&options.repo)?,
    };
    // TYPESAFE_API_KEY, declared by the TypeSafe integration.
    let classifier = Classifier::new(agent::FOREMAN_MODEL, everruns::TypeSafeAI::from_env()?);
    let foreman = Foreman::new(classifier, config.assessment_budget);

    let outcome = run::supervise(
        job,
        agent::FOREMAN_MODEL,
        &options.repo,
        crew,
        foreman,
        config,
        options.tests.clone(),
    )
    .await;
    run::report(&outcome);
    run::settle(&outcome)
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
        agent::WORKER_MODEL,
        agent::worker(model()?, repo)?,
        agent::verifier(model()?, repo)?,
    ))
}
