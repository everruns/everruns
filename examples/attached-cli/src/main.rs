//! `attached-cli host` runs an agent and serves it; every other subcommand is
//! the attached side, run from another terminal as the same user.
//!
//! ```text
//! OPENAI_API_KEY=... cargo run -p everruns-attached-cli -- host
//! cargo run -p everruns-attached-cli -- list
//! cargo run -p everruns-attached-cli -- attach main
//! cargo run -p everruns-attached-cli -- send main "count to five slowly"
//! cargo run -p everruns-attached-cli -- cancel main
//! ```

use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use everruns::control::default_dir;
use everruns::{Agent, Engine, OpenAI};
use everruns_attached_cli::{AttachedCommand, run_attached, serve};

const MODEL: &str = "gpt-6.1-sol";

#[derive(Parser)]
#[command(
    name = "attached-cli",
    about = "Drive a running Everruns session from a second terminal"
)]
struct Cli {
    /// Control directory. Defaults to a private per-user directory.
    #[arg(long, global = true)]
    dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the agent and serve it to attached processes.
    Host {
        /// OpenAI model id.
        #[arg(long, default_value = MODEL)]
        model: String,
    },
    #[command(flatten)]
    Attached(AttachedCommand),
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let dir = cli
        .dir
        .unwrap_or_else(|| default_dir("everruns-attached-cli"));
    let mut stdout = std::io::stdout();
    match cli.command {
        Command::Host { model } => {
            let agent = Agent::builder()
                .instructions("You are a helpful assistant in a terminal. Keep answers short.")
                .provider(OpenAI::from_env().context("OPENAI_API_KEY is required")?)
                .model(model)
                .build()?;
            let session = Engine::new().create(agent);
            let stdin = tokio::io::BufReader::new(tokio::io::stdin());
            // Returning drops the server, which removes the socket and token
            // file; a signal that killed the process outright would leave
            // them for the next start to clean up.
            tokio::select! {
                result = serve(&dir, session, stdin, &mut stdout) => result,
                () = stop_signal() => Ok(()),
            }
        }
        Command::Attached(command) => {
            tokio::select! {
                result = run_attached(&dir, command, &mut stdout) => result,
                // Detaching never cancels anything on the host.
                () = stop_signal() => Ok(()),
            }
        }
    }
}

/// Ctrl-C or SIGTERM.
async fn stop_signal() {
    let mut term = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
        Ok(term) => term,
        Err(_) => {
            let _ = tokio::signal::ctrl_c().await;
            return;
        }
    };
    tokio::select! {
        _ = tokio::signal::ctrl_c() => {}
        _ = term.recv() => {}
    }
}
