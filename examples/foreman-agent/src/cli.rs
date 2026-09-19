//! The command line, in Foreman's shape.
//!
//! `run` is the original's entry point and means the same thing here: supervise
//! real work in a repository you name. It stays a subcommand rather than
//! collapsing into the bare binary so that the invocation from Foreman's own
//! README keeps working verbatim.

use std::path::PathBuf;

use clap::{Parser, Subcommand, ValueEnum};

use crate::worker::{ExternalAgent, TemplateError};

/// A classifier supervising a coding agent it never has to stop.
#[derive(Debug, Parser)]
#[command(name = "foreman", version, about, long_about = None)]
pub struct Cli {
    /// What to do.
    #[command(subcommand)]
    pub command: Command,
}

/// What a factory does.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Supervise a real coding agent working in a repository.
    Run(Run),
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

impl Run {
    /// The external agent this run asked for, if it asked for one.
    ///
    /// `--worker-command` wins over `--worker`: an operator naming a command
    /// line has said more than an operator picking from a list.
    pub fn external(&self) -> Result<Option<ExternalAgent>, TemplateError> {
        if let Some(template) = &self.worker_command {
            return ExternalAgent::from_template(template).map(Some);
        }
        Ok(match self.worker {
            WorkerChoice::Session => None,
            WorkerChoice::Codex => Some(ExternalAgent::codex()),
            WorkerChoice::Yolop => Some(ExternalAgent::yolop()),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::observation::WorkerKind;

    fn run(arguments: &[&str]) -> Run {
        let Command::Run(run) = Cli::try_parse_from(arguments).unwrap().command;
        run
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
}
