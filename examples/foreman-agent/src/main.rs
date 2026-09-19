//! `foreman run --repo … --job …`, the way Foreman itself is started.
//!
//! The binary is only the wiring: read the arguments, build the crew and the
//! supervisor from credentials in the environment, and hand both to
//! [`everruns_foreman_agent::run::supervise`]. Everything it needs is in the
//! library beside it.
//!
//! `demo` runs exactly the same way over a bundled fixture. Nothing about it is
//! simulated — it is `run` with the repository and the job already chosen, so a
//! first run needs a key and nothing else.

use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Parser;
use everruns::{Classifier, Model};
use everruns_example_demo::shell as demo;
use everruns_foreman_agent::cli::{Cli, Command, Demo, Run};
use everruns_foreman_agent::factory::{Outcome, Status};
use everruns_foreman_agent::foreman::Foreman;
use everruns_foreman_agent::policy::Config;
use everruns_foreman_agent::worker::{Crew, ExternalAgent};
use everruns_foreman_agent::{agent, fixture, run};

#[tokio::main]
async fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Run(options) => start(options).await,
        Command::Demo(options) => start_demo(options).await,
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

    let external = options.external().context("reading --worker-command")?;
    let outcome = start_factory(job, &options.repo, external, options.tests.clone()).await?;
    run::report(&outcome);
    run::settle(&outcome)
}

/// The same run, over a repository the example brought with it.
async fn start_demo(options: Demo) -> Result<()> {
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

    let external = options.external().context("reading --worker-command")?;
    let outcome = start_factory(
        fixture::JOB,
        &workspace,
        external,
        Some(fixture::TESTS.to_owned()),
    )
    .await?;
    run::report(&outcome);

    // A fixed starting state is what makes the ending checkable: the job named
    // a rate schedule, so the repository either prices by weight now or does
    // not. Nothing here reads the supervisor's opinion of the work.
    demo::section("REPOSITORY ON DISK");
    for check in fixture::verify(&workspace) {
        demo::check(check.passed, check.label);
    }
    // The last word is not a reading of the tests but a run of them.
    demo::check(
        fixture::tests_pass(&workspace),
        "`bash tests/run.sh` passes",
    );
    if outcome.status == Status::Finished && !fixture::changed(&workspace) {
        bail!("factory finished without changing the repository");
    }
    run::settle(&outcome)
}

/// Build a factory from the environment and run it, rendered.
///
/// Both subcommands land here, because both are the same run: what `run` and
/// `demo` differ on is already decided by the time they call it.
async fn start_factory(
    job: &str,
    workspace: &Path,
    external: Option<ExternalAgent>,
    tests: Option<String>,
) -> Result<Outcome> {
    let config = Config::from_env();
    let crew = match external {
        Some(external) => Crew::External(external),
        None => sessions(workspace)?,
    };
    // TYPESAFE_API_KEY, declared by the TypeSafe integration.
    let classifier = Classifier::new(agent::FOREMAN_MODEL, everruns::TypeSafeAI::from_env()?);
    let foreman = Foreman::new(classifier, config.assessment_budget);

    Ok(run::supervise(
        job,
        agent::FOREMAN_MODEL,
        workspace,
        crew,
        foreman,
        config,
        tests,
    )
    .await)
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
