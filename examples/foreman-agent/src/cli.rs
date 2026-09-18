//! The command line, in Foreman's shape.
//!
//! `run` and `demo` are the original's two entry points and mean the same
//! things here: `run` supervises real work in a repository you name, `demo`
//! walks the same runtime over a disposable fixture with nothing to pay for.

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

/// The two things a factory does.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Supervise a real coding agent working in a repository.
    Run(Run),
    /// Walk the same runtime over a disposable fixture, with no credentials.
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

    /// An external agent's command line, overriding `--worker`.
    ///
    /// `{repo}` and `{mission}` are substituted as whole arguments, so no shell
    /// sees either. Example:
    /// `--worker-command "mycli --cd {repo} --task {mission}"`.
    #[arg(long, value_name = "TEMPLATE")]
    pub worker_command: Option<String>,
}

/// `foreman demo`
#[derive(Debug, Parser)]
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
        let cli = Cli::try_parse_from(arguments).unwrap();
        match cli.command {
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
    fn demo_needs_nothing_and_run_needs_a_job() {
        let cli = Cli::try_parse_from(["foreman", "demo"]).unwrap();
        assert!(matches!(cli.command, Command::Demo(_)));
        // A run without a job is not a run.
        assert!(Cli::try_parse_from(["foreman", "run", "--repo", "."]).is_err());
    }
}
