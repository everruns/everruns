//! Detach a GitHub checks watch, finish the foreground turn, then wake the
//! agent with the result. The task registry and wake channel are deliberately
//! application-owned.
//!
//! With simulated GitHub checks:
//!
//! ```text
//! cargo run -p everruns --features openai --example github_monitor -- --simulate
//! ```
//!
//! With an authenticated GitHub CLI:
//!
//! ```text
//! cargo run -p everruns --features openai --example github_monitor -- --live OWNER/REPO PR_NUMBER
//! ```

use std::process::Stdio;
use std::time::Duration;

use everruns::{Agent, Engine, FunctionTool, OpenAI, ToolResponse};
use serde_json::json;
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Clone)]
enum MonitorMode {
    Simulate,
    Live {
        repository: String,
        pull_request: u64,
    },
}

impl MonitorMode {
    fn target(&self) -> String {
        match self {
            Self::Simulate => "simulated pull request".into(),
            Self::Live {
                repository,
                pull_request,
            } => format!("{repository}#{pull_request}"),
        }
    }
}

struct MonitorFinished {
    task_id: String,
    succeeded: bool,
    output: String,
}

async fn run_monitor(mode: MonitorMode, task_id: String) -> MonitorFinished {
    match mode {
        MonitorMode::Simulate => {
            tokio::time::sleep(Duration::from_millis(50)).await;
            MonitorFinished {
                task_id,
                succeeded: true,
                output: "All simulated GitHub checks passed.".into(),
            }
        }
        MonitorMode::Live {
            repository,
            pull_request,
        } => {
            let mut command = Command::new("gh");
            command
                .args(["pr", "checks"])
                .arg(pull_request.to_string())
                .args(["--repo", &repository, "--watch"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .kill_on_drop(true);

            let result = tokio::time::timeout(Duration::from_secs(30 * 60), command.status()).await;
            match result {
                Ok(Ok(status)) => MonitorFinished {
                    task_id,
                    succeeded: status.success(),
                    output: format!(
                        "GitHub checks for {repository}#{pull_request} exited with {status}."
                    ),
                },
                Ok(Err(error)) => MonitorFinished {
                    task_id,
                    succeeded: false,
                    output: format!("Could not run the GitHub CLI: {error}"),
                },
                Err(_) => MonitorFinished {
                    task_id,
                    succeeded: false,
                    output: "GitHub checks did not finish within 30 minutes.".into(),
                },
            }
        }
    }
}

fn parse_mode() -> Result<MonitorMode, String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [] => Ok(MonitorMode::Simulate),
        [flag] if flag == "--simulate" => Ok(MonitorMode::Simulate),
        [flag, repository, pull_request] if flag == "--live" => {
            let pull_request = pull_request
                .parse::<u64>()
                .ok()
                .filter(|number| *number > 0)
                .ok_or_else(|| "PR_NUMBER must be a positive integer".to_string())?;
            if !repository.contains('/') || repository.starts_with('/') || repository.ends_with('/')
            {
                return Err("OWNER/REPO must contain a non-empty owner and repository".into());
            }
            Ok(MonitorMode::Live {
                repository: repository.clone(),
                pull_request,
            })
        }
        _ => Err("usage: github_monitor [--simulate | --live OWNER/REPO PR_NUMBER]".into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = match parse_mode() {
        Ok(mode) => mode,
        Err(message) => {
            eprintln!("{message}");
            return Ok(());
        }
    };

    let (finished_tx, mut finished_rx) = mpsc::unbounded_channel();
    let mode_for_tool = mode.clone();
    let monitor = FunctionTool::new(
        "monitor_github_checks",
        "Start a host-owned GitHub checks watch and return immediately.",
        json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        }),
        move |_arguments| {
            let finished = finished_tx.clone();
            let mode = mode_for_tool.clone();
            async move {
                let task_id = "github_monitor_1".to_string();
                let task_id_for_run = task_id.clone();
                tokio::spawn(async move {
                    let completion = run_monitor(mode, task_id_for_run).await;
                    let _ = finished.send(completion);
                });
                Ok::<_, String>(ToolResponse::text(format!(
                    "Started {task_id}. End this turn; the host will wake you when the checks finish."
                )))
            }
        },
    );

    let agent = Agent::builder()
        .name("github-monitor")
        .instructions(
            "Start one detached GitHub checks watch, end the turn, and review its completion wake.",
        )
        .provider(OpenAI::from_env()?)
        .model("gpt-5.6-terra")
        .tool(monitor)
        .build()?;

    println!("monitoring {}", mode.target());
    let session = Engine::new().create(agent.clone());
    let started = session
        .send_and_wait("Monitor this pull request until its checks finish.")
        .await?;
    println!("agent: {}", started.response);

    let Some(completion) = finished_rx.recv().await else {
        return Err("GitHub monitor ended without reporting completion".into());
    };
    let status = if completion.succeeded {
        "succeeded"
    } else {
        "failed"
    };
    let wake = format!(
        "[automatic] Background task {} {status}.\n\n{}",
        completion.task_id, completion.output
    );
    let reviewed = session.send_and_wait(wake).await?;
    println!("agent: {}", reviewed.response);

    Ok(())
}
