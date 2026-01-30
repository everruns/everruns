// Everruns CLI
//
// Design Decision: Use clap derive for ergonomic argument parsing.
// Design Decision: Support text/json/yaml output formats for scripting.
// Design Decision: Use everruns-sdk for API client.

mod commands;
mod output;

use clap::{Parser, Subcommand};
use everruns_sdk::Everruns;

#[derive(Parser)]
#[command(name = "everruns")]
#[command(about = "Everruns CLI - Manage agents, sessions, and conversations")]
#[command(version)]
pub struct Cli {
    /// API key (defaults to EVERRUNS_API_KEY env var)
    #[arg(long, env = "EVERRUNS_API_KEY")]
    pub api_key: Option<String>,

    /// API base URL
    #[arg(long, env = "EVERRUNS_API_URL")]
    pub api_url: Option<String>,

    /// Output format
    #[arg(long, short, global = true, default_value = "text", value_parser = ["text", "json", "yaml"])]
    pub output: String,

    /// Suppress non-essential output
    #[arg(long, short, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Manage agents
    Agents {
        #[command(subcommand)]
        command: commands::agents::AgentsCommand,
    },

    /// List available capabilities
    Capabilities {
        /// Filter by status
        #[arg(long, default_value = "available", value_parser = ["available", "coming_soon", "all"])]
        status: String,
    },

    /// Manage sessions
    Sessions {
        #[command(subcommand)]
        command: commands::sessions::SessionsCommand,
    },

    /// Send a message and stream the response
    Chat {
        /// Message text to send
        message: String,

        /// Session ID
        #[arg(long, short)]
        session: uuid::Uuid,

        /// Max wait time in seconds
        #[arg(long, default_value = "300")]
        timeout: u64,

        /// Send message and exit immediately without waiting for response
        #[arg(long)]
        no_stream: bool,
    },
}

const DEFAULT_API_URL: &str = "https://app.everruns.com/api";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let output_format = output::OutputFormat::from_str(&cli.output);

    // Resolve API key and URL
    let api_key = cli
        .api_key
        .or_else(|| std::env::var("EVERRUNS_API_KEY").ok())
        .ok_or_else(|| anyhow::anyhow!("EVERRUNS_API_KEY environment variable not set"))?;
    let api_url = cli
        .api_url
        .or_else(|| std::env::var("EVERRUNS_API_URL").ok())
        .unwrap_or_else(|| DEFAULT_API_URL.to_string());

    // Build SDK client
    let client = Everruns::with_base_url(&api_key, &api_url)?;

    match cli.command {
        Commands::Agents { command } => {
            commands::agents::run(command, &client, output_format, cli.quiet).await
        }
        Commands::Capabilities { status } => {
            // Capabilities endpoint not yet in SDK, use direct HTTP
            commands::capabilities::run(&api_url, &api_key, output_format, &status).await
        }
        Commands::Sessions { command } => {
            commands::sessions::run(command, &client, output_format, cli.quiet).await
        }
        Commands::Chat {
            message,
            session,
            timeout,
            no_stream,
        } => {
            commands::chat::run(&client, output_format, cli.quiet, message, session, timeout, no_stream)
                .await
        }
    }
}
