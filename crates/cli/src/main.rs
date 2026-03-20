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

    /// File sync and management
    Files {
        #[command(subcommand)]
        command: commands::files::FilesCommand,
    },

    /// Send a message and stream the response
    Chat {
        /// Message text to send
        message: String,

        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

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
            commands::sessions::run(
                command,
                &client,
                &api_url,
                &api_key,
                output_format,
                cli.quiet,
            )
            .await
        }
        Commands::Files { command } => {
            commands::files::run(command, &api_url, &api_key, output_format, cli.quiet).await
        }
        Commands::Chat {
            message,
            session,
            timeout,
            no_stream,
        } => {
            commands::chat::run(
                &client,
                output_format,
                cli.quiet,
                message,
                session,
                timeout,
                no_stream,
            )
            .await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn test_cli_parse_agents_list() {
        let cli = Cli::try_parse_from(["everruns", "agents", "list"]).unwrap();
        assert!(matches!(cli.command, Commands::Agents { .. }));
        assert_eq!(cli.output, "text");
        assert!(!cli.quiet);
    }

    #[test]
    fn test_cli_parse_agents_get() {
        let cli = Cli::try_parse_from(["everruns", "agents", "get", "agt_123"]).unwrap();
        if let Commands::Agents { command } = cli.command {
            if let commands::agents::AgentsCommand::Get { agent_id } = command {
                assert_eq!(agent_id, "agt_123");
            } else {
                panic!("Expected Get command");
            }
        } else {
            panic!("Expected Agents command");
        }
    }

    #[test]
    fn test_cli_parse_sessions_create() {
        let cli = Cli::try_parse_from([
            "everruns",
            "sessions",
            "create",
            "--harness",
            "harness_abc",
            "--agent",
            "agt_abc",
            "--title",
            "Test Session",
        ])
        .unwrap();
        if let Commands::Sessions { command } = cli.command {
            if let commands::sessions::SessionsCommand::Create {
                harness,
                agent,
                title,
                model,
            } = command
            {
                assert_eq!(harness, "harness_abc");
                assert_eq!(agent, Some("agt_abc".to_string()));
                assert_eq!(title, Some("Test Session".to_string()));
                assert_eq!(model, None);
            } else {
                panic!("Expected Create command");
            }
        } else {
            panic!("Expected Sessions command");
        }
    }

    #[test]
    fn test_cli_parse_chat() {
        let cli = Cli::try_parse_from(["everruns", "chat", "--session", "ses_xyz", "Hello world"])
            .unwrap();
        if let Commands::Chat {
            message,
            session,
            timeout,
            no_stream,
        } = cli.command
        {
            assert_eq!(message, "Hello world");
            assert_eq!(session, "ses_xyz");
            assert_eq!(timeout, 300); // default
            assert!(!no_stream);
        } else {
            panic!("Expected Chat command");
        }
    }

    #[test]
    fn test_cli_parse_chat_with_options() {
        let cli = Cli::try_parse_from([
            "everruns",
            "chat",
            "--session",
            "ses_xyz",
            "--timeout",
            "60",
            "--no-stream",
            "Test message",
        ])
        .unwrap();
        if let Commands::Chat {
            message,
            session,
            timeout,
            no_stream,
        } = cli.command
        {
            assert_eq!(message, "Test message");
            assert_eq!(session, "ses_xyz");
            assert_eq!(timeout, 60);
            assert!(no_stream);
        } else {
            panic!("Expected Chat command");
        }
    }

    #[test]
    fn test_cli_parse_output_format() {
        let cli = Cli::try_parse_from(["everruns", "-o", "json", "agents", "list"]).unwrap();
        assert_eq!(cli.output, "json");

        let cli = Cli::try_parse_from(["everruns", "--output", "yaml", "agents", "list"]).unwrap();
        assert_eq!(cli.output, "yaml");
    }

    #[test]
    fn test_cli_parse_quiet_flag() {
        let cli = Cli::try_parse_from(["everruns", "-q", "agents", "list"]).unwrap();
        assert!(cli.quiet);

        let cli = Cli::try_parse_from(["everruns", "--quiet", "agents", "list"]).unwrap();
        assert!(cli.quiet);
    }

    #[test]
    fn test_cli_parse_capabilities() {
        let cli = Cli::try_parse_from(["everruns", "capabilities"]).unwrap();
        if let Commands::Capabilities { status } = cli.command {
            assert_eq!(status, "available"); // default
        } else {
            panic!("Expected Capabilities command");
        }

        let cli = Cli::try_parse_from(["everruns", "capabilities", "--status", "all"]).unwrap();
        if let Commands::Capabilities { status } = cli.command {
            assert_eq!(status, "all");
        } else {
            panic!("Expected Capabilities command");
        }
    }

    #[test]
    fn test_cli_parse_files_sync() {
        let cli = Cli::try_parse_from([
            "everruns",
            "files",
            "sync",
            "--session",
            "ses_abc",
            "--interval",
            "5",
            "--conflict",
            "local-wins",
            "--verbose",
            "/tmp/mydir",
        ])
        .unwrap();
        if let Commands::Files { command } = cli.command {
            if let commands::files::FilesCommand::Sync {
                session,
                local_dir,
                interval,
                conflict,
                verbose,
                ..
            } = command
            {
                assert_eq!(session, "ses_abc");
                assert_eq!(local_dir, "/tmp/mydir");
                assert_eq!(interval, 5);
                assert_eq!(conflict, "local-wins");
                assert!(verbose);
            } else {
                panic!("Expected Sync command");
            }
        } else {
            panic!("Expected Files command");
        }
    }

    #[test]
    fn test_cli_parse_files_push() {
        let cli = Cli::try_parse_from([
            "everruns",
            "files",
            "push",
            "--session",
            "ses_xyz",
            "--dry-run",
        ])
        .unwrap();
        if let Commands::Files { command } = cli.command {
            if let commands::files::FilesCommand::Push {
                session, dry_run, ..
            } = command
            {
                assert_eq!(session, "ses_xyz");
                assert!(dry_run);
            } else {
                panic!("Expected Push command");
            }
        } else {
            panic!("Expected Files command");
        }
    }

    #[test]
    fn test_cli_parse_files_pull() {
        let cli = Cli::try_parse_from([
            "everruns",
            "files",
            "pull",
            "--session",
            "ses_xyz",
            "--delete",
        ])
        .unwrap();
        if let Commands::Files { command } = cli.command {
            if let commands::files::FilesCommand::Pull {
                session, delete, ..
            } = command
            {
                assert_eq!(session, "ses_xyz");
                assert!(delete);
            } else {
                panic!("Expected Pull command");
            }
        } else {
            panic!("Expected Files command");
        }
    }

    #[test]
    fn test_cli_parse_files_ls() {
        let cli = Cli::try_parse_from([
            "everruns",
            "files",
            "ls",
            "--session",
            "ses_xyz",
            "-r",
            "-l",
            "/src",
        ])
        .unwrap();
        if let Commands::Files { command } = cli.command {
            if let commands::files::FilesCommand::Ls {
                session,
                path,
                recursive,
                long,
            } = command
            {
                assert_eq!(session, "ses_xyz");
                assert_eq!(path, "/src");
                assert!(recursive);
                assert!(long);
            } else {
                panic!("Expected Ls command");
            }
        } else {
            panic!("Expected Files command");
        }
    }

    #[test]
    fn test_cli_invalid_output_format() {
        let result = Cli::try_parse_from(["everruns", "-o", "invalid", "agents", "list"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_missing_required_args() {
        // Chat requires --session
        let result = Cli::try_parse_from(["everruns", "chat", "Hello"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_cli_help_available() {
        // Verify help can be generated without panic
        let _ = Cli::command().render_help();
    }
}
