//! One supervised run, rendered to a terminal.
//!
//! The shape is the same whoever is on the floor: build the factory, print the
//! header, run it, print what it decided. Both binaries — the real `foreman`
//! and the demo beside it — go through here, so neither can drift into its own
//! account of what happened.

use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, bail};
use everruns_example_demo::shell as demo;

use crate::factory::{Factory, Outcome, Status};
use crate::foreman::Foreman;
use crate::policy::Config;
use crate::terminal;
use crate::worker::Crew;

/// Run one factory, rendered.
#[allow(clippy::too_many_arguments)]
pub async fn supervise(
    job: &str,
    model: &str,
    workspace: &Path,
    crew: Crew,
    foreman: Foreman,
    config: Config,
    tests: Option<String>,
) -> Outcome {
    let factory = Factory::new(job, workspace, crew, foreman, config)
        .testing(tests.clone())
        .watched_by(Arc::new(terminal::Terminal::new()));

    demo::banner("everruns · foreman");
    demo::field("worker", &factory.crew_label());
    demo::field("foreman", &format!("{model} — nine questions, one request"));
    demo::field("repository", &workspace.display().to_string());
    demo::field("tests", tests.as_deref().unwrap_or("none configured"));
    demo::field("run", factory.run_id());
    demo::field("job", &headline(job));

    factory.run().await
}

/// The exit status is whether the factory decided, not which way.
///
/// Escalation is the supervisor doing its job, and a person is standing right
/// here. Only a run that ran out of clock decided nothing at all.
pub fn settle(outcome: &Outcome) -> Result<()> {
    match outcome.status {
        Status::TimedOut => bail!("factory ran out of time without deciding"),
        _ => Ok(()),
    }
}

pub fn report(outcome: &Outcome) {
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
pub fn headline(job: &str) -> String {
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
        let long = "word ".repeat(40);
        let headline = headline(&long);
        assert!(headline.chars().count() <= 88);
        assert!(!headline.contains('\n'));
        assert!(headline.ends_with('\u{2026}'));
    }
}
