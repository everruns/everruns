//! `ercode` — a terminal coding agent on public Framework workspace APIs.

use std::io::Write;

use anyhow::{Context, Result};
use clap::Parser;
use everruns::{Model, OpenAI, Session, SessionEventKind, WorkspaceHeadAccess};
use everruns_coding_cli::{Cli, CodingWorkspace, SessionMode, agent_builder};
use tokio::io::{AsyncBufReadExt, BufReader};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let state_dir = cli.state_dir()?;
    let session_mode = cli.session_mode();
    let workspace = match &session_mode {
        SessionMode::NewHead { .. } => CodingWorkspace::open(cli.repository()?, &state_dir).await?,
        SessionMode::SharedHead { .. } | SessionMode::Resume { .. } => {
            CodingWorkspace::from_state(&state_dir)?
        }
    };

    let builder = workspace.configure_agent(agent_builder(), cli.workspace_policy());
    let agent = if cli.offline() {
        builder
            .model(Model::simulated(
                "Offline simulator: configure a real model to execute coding tool calls.",
            ))
            .build()
    } else {
        let provider = OpenAI::from_env()
            .context("configure OpenAI (set OPENAI_API_KEY, or pass --offline)")?;
        builder.provider(provider).model(cli.model()).build()
    }
    .context("build the coding agent")?;

    let session = match session_mode {
        SessionMode::NewHead { name, base, shared } => {
            let head = workspace.create_head(name, base.as_deref(), shared).await?;
            workspace.start_session(&agent, head).await?
        }
        SessionMode::SharedHead { recorded_session } => {
            workspace
                .start_shared_session(&agent, recorded_session)
                .await?
        }
        SessionMode::Resume { session_id } => workspace.resume_session(&agent, session_id).await?,
    };

    let head = session
        .workspace_head()
        .context("session did not bind a workspace head")?;
    let access = match head.access() {
        WorkspaceHeadAccess::Isolated => "isolated",
        WorkspaceHeadAccess::Shared => "shared",
    };
    eprintln!(
        "session={} workspace={} head={} name={:?} access={} base={}",
        session.session_id(),
        head.workspace_id(),
        head.id(),
        head.name(),
        access,
        head.base().unwrap_or("HEAD")
    );
    eprintln!("state={}", state_dir.display());

    if cli.prompt().is_empty() {
        repl(&session).await
    } else {
        run_prompt(&session, &cli.prompt().join(" ")).await
    }
}

/// Run one prompt: render the tool activity the turn emits, then its response.
async fn run_prompt(session: &Session, prompt: &str) -> Result<()> {
    let mut events = session.events();
    let turn = session.run(prompt).await.context("run turn")?;

    while let Some(event) = events.try_recv().context("read turn events")? {
        match event.kind {
            SessionEventKind::ToolStarted { tool_name, .. } => eprintln!("  → {tool_name}"),
            SessionEventKind::ToolCompleted {
                tool_name, success, ..
            } => {
                eprintln!("  {} {tool_name}", if success { "✓" } else { "✗" });
            }
            _ => {}
        }
    }

    if !turn.success
        && let Some(err) = &turn.error
    {
        eprintln!("turn did not complete: {err}");
    }
    println!("{}", turn.response);
    Ok(())
}

/// Interactive REPL: each line is a turn on the same session and exact head.
async fn repl(session: &Session) -> Result<()> {
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    eprintln!("ercode — type a prompt, or Ctrl-D to exit.");
    loop {
        eprint!("› ");
        std::io::stderr().flush().ok();
        match lines.next_line().await.context("read stdin")? {
            None => break,
            Some(line) if line.trim().is_empty() => continue,
            Some(line) => run_prompt(session, &line).await?,
        }
    }
    Ok(())
}
