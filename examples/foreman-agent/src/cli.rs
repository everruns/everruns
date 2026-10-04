//! Foreman-compatible command line.

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
    /// Supervise a job over a bundled shipping-rate repository.
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
    #[arg(long, value_name = "COMMAND")]
    pub tests: Option<String>,

    /// Docker image containing the project toolchain and offline dependencies.
    #[arg(long, default_value = "rust:1.96-bookworm")]
    pub test_image: String,

    /// An external agent's command line, overriding `--worker`.
    #[arg(long, value_name = "TEMPLATE")]
    pub worker_command: Option<String>,
}

/// `foreman demo`
#[derive(Debug, Parser)]
pub struct Demo {
    /// Enter the job before starting the coding worker.
    #[arg(long)]
    pub interactive: bool,

    /// Where to materialize the fixture. A temporary directory by default.
    #[arg(long, value_name = "PATH")]
    pub repo: Option<PathBuf>,

    /// Docker image for the shell fixture tests.
    #[arg(long, default_value = "rust:1.96-bookworm")]
    pub test_image: String,

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
#[path = "../tests/unit/cli.rs"]
mod tests;

/// Normalize before using the path as both a mount and an external CLI argument.
pub fn repository(path: &std::path::Path) -> std::io::Result<PathBuf> {
    let path = path.canonicalize()?;
    if !path.is_dir() {
        return Err(std::io::Error::other("repository must be a directory"));
    }
    Ok(path)
}
