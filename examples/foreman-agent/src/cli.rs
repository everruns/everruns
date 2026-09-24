//! The command line, in Foreman's shape.
//!
//! `run` and `demo` are the original's two entry points and mean the same
//! things here. Both supervise a real worker with a real decision service; they
//! differ only in who picked the repository and the job. `demo` picked them, so
//! there is something to run before you have a project in mind.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::worker::{ExternalAgent, TemplateError};

/// A decision service supervising a coding agent it never has to stop.
#[derive(Debug, Parser)]
#[command(name = "foreman", version, about, long_about = None)]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// The two things a factory does.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Supervise a real coding agent working in a repository.
    Run(Run),
    /// The same thing over a bundled fixture, on a job it already knows.
    Demo(Demo),
}

/// `foreman run --repo ./my-project --job "..."`
#[derive(Debug, Parser)]
pub struct Run {
    /// The repository the worker works in. It will be modified.
    #[arg(long, value_name = "PATH")]
    pub repo: PathBuf,

    /// The job, in whatever words describe it.
    #[arg(long, value_name = "TEXT")]
    pub job: String,

    /// Who does the work.
    #[arg(long, value_enum, default_value_t = WorkerChoice::Session)]
    pub worker: WorkerChoice,

    /// The command that runs this repository's tests, e.g. `cargo test` or
    /// `bash tests/run.sh`.
    ///
    /// The supervisor runs it itself, the way it runs `git` itself. Without it
    /// `tests_sufficient` rests on someone reading the tests rather than on one
    /// having passed.
    #[arg(long, value_name = "COMMAND")]
    pub tests: Option<String>,

    /// An external agent's command line, overriding `--worker`.
    ///
    /// `{repo}` and `{mission}` are substituted as whole arguments, so no shell
    /// sees either. Example:
    /// `--worker-command "mycli --cd {repo} --task {mission}"`.
    #[arg(long, value_name = "TEMPLATE")]
    pub worker_command: Option<String>,
}

/// `foreman demo`
///
/// Not a simulation. The worker and the supervisor are the ones `run` uses, on
/// the same credentials; the only thing this subcommand supplies is a small
/// repository with a known flaw and a job describing it, so the run has a fixed
/// starting state instead of a fixed script.
#[derive(Debug, Parser)]
pub struct Demo {
    /// Where to materialize the fixture. A temporary directory by default.
    ///
    /// Whatever is here will be overwritten, so this is not your project.
    #[arg(long, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    /// Who does the work. The same choice `run` offers.
    #[arg(long, value_enum, default_value_t = WorkerChoice::Session)]
    pub worker: WorkerChoice,

    /// An external agent's command line, overriding `--worker`.
    #[arg(long, value_name = "TEMPLATE")]
    pub worker_command: Option<String>,
}

/// Who does the work on a `run`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum WorkerChoice {
    /// An Everruns session on the Bashkit shell.
    Session,
    /// The Codex CLI, as Foreman itself runs it.
    Codex,
    /// yolop, on its one-shot `--print` interface.
    Yolop,
}

/// The external agent an invocation asked for, if it asked for one.
///
/// `--worker-command` wins over `--worker`: an operator naming a command line
/// has said more than an operator picking from a list.
fn external(
    worker: WorkerChoice,
    template: Option<&String>,
) -> Result<Option<ExternalAgent>, TemplateError> {
    if let Some(template) = template {
        return ExternalAgent::from_template(template).map(Some);
    }
    Ok(match worker {
        WorkerChoice::Session => None,
        WorkerChoice::Codex => Some(ExternalAgent::codex()),
        WorkerChoice::Yolop => Some(ExternalAgent::yolop()),
    })
}

impl Run {
    /// The external agent this run asked for, if it asked for one.
    pub fn external(&self) -> Result<Option<ExternalAgent>, TemplateError> {
        external(self.worker, self.worker_command.as_ref())
    }
}

impl Demo {
    /// The external agent this demo asked for, if it asked for one.
    pub fn external(&self) -> Result<Option<ExternalAgent>, TemplateError> {
        external(self.worker, self.worker_command.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::observation::WorkerKind;

    fn run(arguments: &[&str]) -> Run {
        match Cli::try_parse_from(arguments).unwrap().command {
            Command::Run(run) => run,
            other => panic!("expected a run, got {other:?}"),
        }
    }

    #[test]
    fn foremans_own_invocation_parses() {
        // The line from Foreman's README, unchanged.
        let run = run(&[
            "foreman",
            "run",
            "--repo",
            "./my-project",
            "--job",
            "Add rate limiting to the API and make sure it is properly tested.",
        ]);
        assert_eq!(run.repo, PathBuf::from("./my-project"));
        assert!(run.job.starts_with("Add rate limiting"));
        // An Everruns session is the default crew; Foreman's Codex worker is
        // one flag away.
        assert_eq!(run.worker, WorkerChoice::Session);
        assert!(run.external().unwrap().is_none());
    }

    #[test]
    fn a_test_command_is_optional_and_passed_through() {
        assert_eq!(
            run(&["foreman", "run", "--repo", ".", "--job", "j"]).tests,
            None
        );
        assert_eq!(
            run(&[
                "foreman",
                "run",
                "--repo",
                ".",
                "--job",
                "j",
                "--tests",
                "cargo test"
            ])
            .tests
            .as_deref(),
            Some("cargo test")
        );
    }

    #[test]
    fn a_named_worker_selects_its_preset() {
        let codex = run(&[
            "foreman", "run", "--repo", ".", "--job", "j", "--worker", "codex",
        ])
        .external()
        .unwrap()
        .unwrap();
        assert_eq!(codex.label, "codex");

        let yolop = run(&[
            "foreman", "run", "--repo", ".", "--job", "j", "--worker", "yolop",
        ])
        .external()
        .unwrap()
        .unwrap();
        assert_eq!(yolop.label, "yolop");
    }

    #[test]
    fn a_command_template_outranks_a_named_worker() {
        let run = run(&[
            "foreman",
            "run",
            "--repo",
            ".",
            "--job",
            "j",
            "--worker",
            "codex",
            "--worker-command",
            "mycli --cd {repo} --task {mission}",
        ]);
        let agent = run.external().unwrap().unwrap();
        assert_eq!(agent.label, "mycli");
        assert_eq!(
            agent.command(WorkerKind::Coding, std::path::Path::new("/r"), "go"),
            vec!["mycli", "--cd", "/r", "--task", "go"]
        );
    }

    #[test]
    fn a_template_with_nowhere_to_put_the_mission_is_refused() {
        let run = run(&[
            "foreman",
            "run",
            "--repo",
            ".",
            "--job",
            "j",
            "--worker-command",
            "mycli --cd {repo}",
        ]);
        assert!(run.external().is_err());
    }

    #[test]
    fn a_run_without_a_job_is_not_a_run() {
        assert!(Cli::try_parse_from(["foreman", "run", "--repo", "."]).is_err());
    }

    #[test]
    fn a_demo_needs_nothing_and_takes_the_same_workers() {
        let Command::Demo(demo) = Cli::try_parse_from(["foreman", "demo"]).unwrap().command else {
            panic!("expected a demo");
        };
        assert_eq!(demo.worker, WorkerChoice::Session);
        assert!(demo.external().unwrap().is_none());

        let Command::Demo(demo) = Cli::try_parse_from(["foreman", "demo", "--worker", "codex"])
            .unwrap()
            .command
        else {
            panic!("expected a demo");
        };
        assert_eq!(demo.external().unwrap().unwrap().label, "codex");
    }
}
