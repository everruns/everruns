#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Run from the repository checkout; see README.md for credentials and scenarios.
use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use everruns::{ContainmentMode, Engine, SandboxLauncher, SandboxOptions};
use everruns_example_demo::shell as demo;

mod agent;
mod fixture;

const DEFAULT_TASK: &str = "The test suite is failing. Find out why and fix it.";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Before anything else, including the async runtime: a re-exec of this
    // binary as the containment worker restricts itself and then execs bash, so
    // it must not start a runtime or touch the terminal first.
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some(agent::SANDBOX_WORKER_ARGUMENT) {
        everruns::containment_worker(arguments)?;
    }
    run()
}

#[tokio::main]
async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let options = Options::parse(std::env::args_os().skip(1))?;
    let (_temporary, workspace) = match options.workspace {
        Some(path) => (None, path),
        None => {
            let directory = tempfile::tempdir()?;
            let path = directory.path().join("chunker");
            (Some(directory), path)
        }
    };
    fs::create_dir_all(&workspace)?;
    fixture::materialize(&workspace)?;

    demo::banner("everruns · host shell agent");
    demo::field("model", agent::MODEL);
    demo::field(
        "capability",
        "host_shell — real processes, kernel-contained",
    );
    demo::field("containment", ContainmentMode::WorkspaceWrite.as_str());
    demo::field(
        "network",
        everruns::network_access(ContainmentMode::WorkspaceWrite),
    );
    demo::field("workspace", &workspace.display().to_string());
    report_boundary(&workspace).await;

    // OPENAI_API_KEY, declared by the OpenAI driver itself.
    let agent = agent::build(everruns::OpenAI::from_env()?, &workspace)?;
    let engine = Engine::new();
    let session = engine.create(agent);
    demo::run(&session, &options.task).await?;

    verify(&workspace)?;
    println!("\nCompleted: the suite passes, checked from the host");
    Ok(())
}

/// Show the boundary before the agent runs, by trying to cross it.
///
/// The same provider the capability uses, so this is the real policy rather than
/// a description of one. It is also the honest place to say when a machine
/// cannot enforce it: on such a kernel the provider errors, and the run should
/// be read as uncontained.
async fn report_boundary(workspace: &Path) {
    /// A path outside every writable root. `/tmp` is writable by design, so a
    /// temp directory would prove nothing.
    const OUTSIDE: &str = "/etc/everruns-containment-probe";

    let sandbox = everruns::containment_provider(
        SandboxOptions::new(ContainmentMode::WorkspaceWrite).launcher(SandboxLauncher::ReexecSelf(
            vec![agent::SANDBOX_WORKER_ARGUMENT.to_string()],
        )),
    );
    demo::section("THE BOUNDARY, BEFORE THE AGENT TOUCHES IT");
    for (label, script, expected) in [
        (
            "writes inside the workspace succeed",
            "touch ./probe && rm ./probe",
            true,
        ),
        (
            "writes outside it are refused by the kernel",
            &format!("touch {OUTSIDE}") as &str,
            false,
        ),
        (
            "outbound sockets are refused by the kernel",
            "exec 3<>/dev/tcp/1.1.1.1/80",
            false,
        ),
    ] {
        let allowed = match sandbox.command(workspace, script) {
            Ok(mut command) => {
                everruns::configure_contained_stdio(&mut command);
                match command.output().await {
                    Ok(output) => output.status.success(),
                    Err(error) => {
                        demo::body(&format!("{label}: could not run ({error})"), demo::DIM);
                        continue;
                    }
                }
            }
            Err(error) => {
                // Fail closed: the provider refused rather than running the
                // command uncontained, and the demo says so rather than
                // reporting a boundary that is not there.
                demo::body(
                    &format!("containment unavailable on this machine: {error:#}"),
                    demo::DIM,
                );
                return;
            }
        };
        demo::check(allowed == expected, label);
    }
    // Only reachable if the boundary did not hold; leaving the file behind
    // would be worse than saying nothing.
    let _ = fs::remove_file(OUTSIDE);
}

fn verify(workspace: &Path) -> Result<(), Box<dyn std::error::Error>> {
    demo::section("CHECKED FROM THE HOST, NOT FROM THE AGENT");
    let checks = fixture::verify(workspace);
    for check in &checks {
        demo::check(check.passed, &check.label);
        if !check.passed && !check.detail.is_empty() {
            demo::body(&check.detail, demo::DIM);
        }
    }

    demo::section("src/lib.rs AFTER THE RUN");
    demo::body(
        &fs::read_to_string(workspace.join("src/lib.rs"))?,
        demo::DIM,
    );

    let failures = checks.iter().filter(|check| !check.passed).count();
    if failures > 0 {
        return Err(format!("{failures} check(s) failed").into());
    }
    Ok(())
}

struct Options {
    task: String,
    workspace: Option<PathBuf>,
}

impl Options {
    fn parse(arguments: impl IntoIterator<Item = OsString>) -> io::Result<Self> {
        let mut interactive = false;
        let mut workspace = None;
        for argument in arguments {
            if argument == "--interactive" {
                interactive = true;
            } else if workspace.is_none() {
                workspace = Some(PathBuf::from(argument));
            } else {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "usage: host-shell-agent [--interactive] [workspace]",
                ));
            }
        }
        let task = if interactive {
            read_task()?
        } else {
            DEFAULT_TASK.to_owned()
        };
        Ok(Self { task, workspace })
    }
}

fn read_task() -> io::Result<String> {
    use std::io::Write;
    print!("Task: ");
    io::stdout().flush()?;

    let mut task = String::new();
    io::stdin().read_line(&mut task)?;
    validate_task(&task)
}

fn validate_task(task: &str) -> io::Result<String> {
    let task = task.trim();
    if task.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "task cannot be empty",
        ));
    }
    Ok(task.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_workspace_argument_is_optional_and_defaults_to_the_bundled_task() {
        let options = Options::parse(["/tmp/chunker".into()]).unwrap();
        assert_eq!(options.workspace, Some(PathBuf::from("/tmp/chunker")));
        assert_eq!(options.task, DEFAULT_TASK);

        assert!(Options::parse([]).unwrap().workspace.is_none());
        assert!(Options::parse(["/one".into(), "/two".into()]).is_err());
    }

    #[test]
    fn an_interactive_task_is_trimmed_and_required() {
        assert_eq!(
            validate_task("  Fix the suite.\n").unwrap(),
            "Fix the suite."
        );
        assert_eq!(
            validate_task(" \n").unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }
}
