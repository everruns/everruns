//! CLI wiring for a real supervised run or the bundled demo.

use std::io::{self, BufRead, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use clap::Parser;
use everruns::{Decisions, Model};
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
async fn start(options: Run) -> Result<()> {
    if !options.repo.is_dir() {
        bail!("{} is not a directory", options.repo.display());
    }
    let job = options.job.trim();
    if job.is_empty() {
        bail!("a job cannot be empty");
    }

    let external = options.external().context("reading --worker-command")?;
    let workspace =
        everruns_foreman_agent::cli::repository(&options.repo).context("resolving --repo")?;
    let outcome = start_factory(
        job,
        &workspace,
        external,
        options.tests.clone(),
        options.test_image,
    )
    .await?;
    run::report(&outcome);
    run::settle(&outcome)
}

/// The same run, over a repository the example brought with it.
async fn start_demo(options: Demo) -> Result<()> {
    let temporary = options
        .repo
        .is_none()
        .then(|| {
            // Docker on macOS shares the home directory; /var/folders often is not shared.
            match std::env::var_os("HOME") {
                Some(home) => tempfile::Builder::new()
                    .prefix(".foreman-")
                    .tempdir_in(home),
                None => tempfile::tempdir(),
            }
        })
        .transpose()
        .context("creating a temporary workspace")?;
    let workspace = match (&options.repo, &temporary) {
        (Some(path), _) => path.clone(),
        (None, Some(directory)) => directory.path().join("shipkit"),
        (None, None) => bail!("no workspace"),
    };
    if workspace.exists() && std::fs::read_dir(&workspace)?.next().is_some() {
        bail!("demo --repo must name an empty scratch directory");
    }
    std::fs::create_dir_all(&workspace)?;
    let workspace = workspace.canonicalize()?;
    fixture::materialize(&workspace).context("materializing the fixture")?;

    let job = if options.interactive {
        demo::banner("FOREMAN · ready for a job");
        demo::field(
            "repository",
            "shipkit · flat shipping rates, 3 existing tests",
        );
        demo::field(
            "pricing",
            "700 / 1200 / 2400 / 4800 cents; zone surcharge stays",
        );
        demo::field("boundaries", "1000 g / 5000 g / 20000 g");
        demo::body(
            "Enter a shipping-rate job. These pricing requirements apply to the demo.",
            demo::DIM,
        );
        let request = read_job(&mut io::stdin().lock(), &mut io::stdout())?;
        println!();
        demo::field("request", &request);
        format!("{request}\n\nDemo requirements: {}", fixture::JOB)
    } else {
        fixture::JOB.to_owned()
    };
    let external = options.external().context("reading --worker-command")?;
    let outcome = start_factory(
        &job,
        &workspace,
        external,
        Some(fixture::TESTS.to_owned()),
        options.test_image.clone(),
    )
    .await?;
    run::report(&outcome);

    run::settle(&outcome)?;
    if outcome.status != Status::Finished {
        bail!("demo did not reach FINISH");
    }
    if !fixture::changed(&workspace) {
        bail!("factory finished without changing the repository");
    }

    demo::section("ACCEPTANCE CHECKS · fixed weight boundaries");
    let acceptance = everruns_foreman_agent::observation::run_tests(
        &workspace,
        fixture::ACCEPTANCE,
        &Config::from_env(),
        &options.test_image,
    )
    .await;
    demo::body(acceptance.output_tail.trim(), demo::DIM);
    demo::check(acceptance.passed, "fixed acceptance checks pass");
    if !acceptance.passed {
        bail!("demo failed fixed acceptance checks");
    }

    // The last word is an actual test run, independent of the model's verdict.
    demo::section("FINAL TEST RUN · bash tests/run.sh");
    let tests = everruns_foreman_agent::observation::run_tests(
        &workspace,
        fixture::TESTS,
        &Config::from_env(),
        &options.test_image,
    )
    .await;
    demo::body(tests.output_tail.trim(), demo::DIM);
    demo::check(tests.passed, "`bash tests/run.sh` passes");
    if !tests.passed {
        bail!("demo's final suite failed");
    }

    run::settle(&outcome)
}

fn read_job(input: &mut impl BufRead, output: &mut impl Write) -> io::Result<String> {
    write!(output, "\nJob: ")?;
    output.flush()?;
    let mut job = String::new();
    input.read_line(&mut job)?;
    let job = job.trim();
    if job.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a job cannot be empty",
        ));
    }
    Ok(job.to_owned())
}

#[cfg(test)]
#[path = "../tests/unit/demo.rs"]
mod tests;

/// Build a factory from the environment and run it, rendered.
async fn start_factory(
    job: &str,
    workspace: &Path,
    external: Option<ExternalAgent>,
    tests: Option<String>,
    test_image: String,
) -> Result<Outcome> {
    let config = Config::from_env();
    let crew = crew(workspace, external)?;
    // TYPESAFE_API_KEY, declared by the TypeSafe integration.
    let decisions = Decisions::new(agent::FOREMAN_MODEL, everruns::TypeSafeAI::from_env()?);
    let foreman = Foreman::new(decisions, config.assessment_budget);

    Ok(run::supervise(
        job,
        agent::FOREMAN_MODEL,
        workspace,
        crew,
        foreman,
        config,
        tests,
        test_image,
    )
    .await)
}

/// Everruns sessions over `repo`: one that may write, one that may not.
fn crew(repo: &Path, external: Option<ExternalAgent>) -> Result<Crew> {
    // OPENROUTER_API_KEY, declared by the OpenRouter driver itself.
    let model = || -> Result<Model> {
        Ok(Model::new(
            agent::WORKER_MODEL,
            everruns_drivers::openrouter::from_env("openrouter")?,
        ))
    };
    let verifier = agent::verifier(model()?, repo)?;
    Ok(match external {
        Some(worker) => Crew::external(worker, agent::WORKER_MODEL, verifier),
        None => Crew::sessions(
            agent::WORKER_MODEL,
            agent::worker(model()?, repo)?,
            verifier,
        ),
    })
}
