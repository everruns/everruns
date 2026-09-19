//! The same runtime, with nothing to pay for.
//!
//! `foreman demo` walks a scripted worker over a disposable fixture while a
//! classifier service answers from a table. Only those two ends are simulated:
//! the event stream, the observation, the policy, the interventions, and the
//! rendering all come from `everruns-foreman-agent`, unchanged.
//!
//! ```text
//! foreman-demo
//! foreman-demo --live-foreman   # scripted worker, real supervisor
//! ```
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::Parser;
use everruns::Classifier;
use everruns_example_demo::shell as demo;
use everruns_foreman_agent::factory::Status;
use everruns_foreman_agent::foreman::Foreman;
use everruns_foreman_agent::worker::Crew;
use everruns_foreman_agent::{agent, run};

mod fixture;
mod scripted;

/// `foreman-demo`
#[derive(Debug, Parser)]
#[command(name = "foreman-demo", version, about, long_about = None)]
pub struct Demo {
    /// Where to materialize the fixture. A temporary directory by default.
    ///
    /// Whatever is here will be overwritten, so this is not your project.
    #[arg(long, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    /// Supervise the scripted worker with a real classifier.
    ///
    /// Supervision is the cheap half, so the numbers can be live even when the
    /// worker is not. Needs `TYPESAFE_API_KEY`.
    #[arg(long)]
    pub live_foreman: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let options = Demo::parse();
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

    let mut config = scripted::config();
    let (model, classifier) = if options.live_foreman {
        // A real classifier needs room to answer, and a scripted worker does
        // not wait around for it, so the floor between readings goes up too.
        config.assessment_budget = config
            .assessment_budget
            .max(std::time::Duration::from_secs(10));
        config.min_assessment_interval = config
            .min_assessment_interval
            .max(std::time::Duration::from_secs(2));
        (
            agent::FOREMAN_MODEL,
            Classifier::new(agent::FOREMAN_MODEL, everruns::TypeSafeAI::from_env()?),
        )
    } else {
        // The same supervisor over a service that answers from a table, which
        // is the Framework's own way to run one without a vendor.
        (
            scripted::READINGS_MODEL,
            scripted::Readings::classifier().context("reading the demo readings")?,
        )
    };
    let foreman = Foreman::new(classifier, config.assessment_budget);
    let crew = Crew::sessions(
        scripted::WORKER_MODEL,
        agent::worker(scripted::worker(), &workspace)?,
        agent::verifier(scripted::verifier(), &workspace)?,
    );

    let outcome = run::supervise(
        fixture::JOB,
        model,
        &workspace,
        crew,
        foreman,
        config,
        Some(fixture::TESTS.to_owned()),
    )
    .await;
    run::report(&outcome);

    demo::section("REPOSITORY ON DISK");
    for check in fixture::verify(&workspace) {
        demo::check(check.passed, check.label);
    }
    // The demo's last word is not a reading of the tests but a run of them.
    demo::check(
        fixture::tests_pass(&workspace),
        "`bash tests/run.sh` passes",
    );
    if outcome.status == Status::Finished && !fixture::changed(&workspace) {
        bail!("factory finished without changing the repository");
    }
    run::settle(&outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    use everruns_foreman_agent::factory::Factory;

    #[tokio::test]
    async fn the_demo_finishes_after_verifying_its_own_work() {
        let workspace = tempfile::tempdir().unwrap();
        let root = workspace.path().join("shipkit");
        std::fs::create_dir_all(&root).unwrap();
        fixture::materialize(&root).unwrap();

        let crew = Crew::sessions(
            scripted::WORKER_MODEL,
            agent::worker(scripted::worker(), &root).unwrap(),
            agent::verifier(scripted::verifier(), &root).unwrap(),
        );
        let outcome = Factory::new(
            fixture::JOB,
            &root,
            crew,
            Foreman::new(
                scripted::Readings::classifier().unwrap(),
                scripted::config().assessment_budget,
            ),
            scripted::config(),
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
