//! `ercode` — a minimal terminal coding agent built only on the `everruns`
//! public library (EVE-835).
//!
//! Run one prompt and exit, or drop into a REPL that keeps one [`Session`]'s
//! history across turns. Offline by default (deterministic simulator); pass a
//! real model with `--model` once `OPENAI_API_KEY` is set.

use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::Parser;
use everruns::Model;
use everruns::{OpenAI, Session, SessionEventKind};
use everruns_coding_cli::{build_agent, set_workspace};
use tokio::io::{AsyncBufReadExt, BufReader};

#[derive(Parser, Debug)]
#[command(
    name = "ercode",
    version,
    about = "Everruns coding CLI — a minimal coding agent built on the everruns library"
)]
struct Cli {
    /// Workspace root the agent's file tools operate inside (default: current dir).
    #[arg(short = 'C', long = "cwd")]
    cwd: Option<PathBuf>,

    /// Model id to use with OpenAI (requires OPENAI_API_KEY). Ignored with --offline.
    #[arg(long, default_value = "gpt-5-mini")]
    model: String,

    /// Run fully offline with the deterministic simulator (no credentials, no network).
    #[arg(long)]
    offline: bool,

    /// One-shot prompt. Omit to start an interactive REPL.
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let root = match cli.cwd {
        Some(dir) => dir,
        None => std::env::current_dir().context("resolve current directory")?,
    };
    set_workspace(root);

    let agent = if cli.offline {
        build_agent(Model::simulated(
            "Offline simulator: set up a real model with --model to use the tools.",
        ))
    } else {
        let model = OpenAI::from_env(&cli.model)
            .context("configure OpenAI (set OPENAI_API_KEY, or pass --offline)")?;
        build_agent(model)
    }
    .context("build the coding agent")?;

    let mut session = agent.session();

    if cli.prompt.is_empty() {
        repl(&mut session).await
    } else {
        run_prompt(&mut session, &cli.prompt.join(" ")).await
    }
}

/// Run one prompt: render the tool activity the turn emits, then its response.
async fn run_prompt(session: &mut Session, prompt: &str) -> Result<()> {
    let mut events = session.events();
    let turn = session.run(prompt).await.context("run turn")?;

    // Events emitted during the turn are buffered on the stream; drain them.
    while let Some(event) = events.try_recv() {
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

/// Interactive REPL: each line is a turn; history accumulates in the session.
async fn repl(session: &mut Session) -> Result<()> {
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
