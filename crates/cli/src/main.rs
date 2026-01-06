// Everruns CLI
//
// Design Decision: Use clap derive for ergonomic argument parsing.
// Design Decision: Support text/json/yaml output formats for scripting.
// Design Decision: Use reqwest for HTTP client (already in workspace).

mod client;
mod commands;
mod output;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "everruns")]
#[command(about = "Everruns CLI - Manage agents, sessions, and conversations")]
#[command(version)]
pub struct Cli {
    /// API base URL
    #[arg(
        long,
        env = "EVERRUNS_API_URL",
        default_value = "http://localhost:9000"
    )]
    pub api_url: String,

    /// Output format
    #[arg(long, short, default_value = "text", value_parser = ["text", "json", "yaml"])]
    pub output: String,

    /// Suppress non-essential output
    #[arg(long, short)]
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

        /// Agent ID (auto-detected from session if omitted)
        #[arg(long, short)]
        agent: Option<uuid::Uuid>,

        /// Max wait time in seconds
        #[arg(long, default_value = "300")]
        timeout: u64,

        /// Send message and exit immediately without waiting for response
        #[arg(long)]
        no_stream: bool,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let client = client::Client::new(&cli.api_url);
    let output_format = output::OutputFormat::from_str(&cli.output);

    match cli.command {
        Commands::Agents { command } => {
            commands::agents::run(command, &client, output_format, cli.quiet).await
        }
        Commands::Capabilities { status } => {
            commands::capabilities::run(&client, output_format, &status).await
        }
        Commands::Sessions { command } => {
            commands::sessions::run(command, &client, output_format, cli.quiet).await
        }
        Commands::Chat {
            message,
            session,
            agent,
            timeout,
            no_stream,
        } => {
            commands::chat::run(
                &client,
                output_format,
                cli.quiet,
                message,
                session,
                agent,
                timeout,
                no_stream,
            )
            .await
        }
    }
}
