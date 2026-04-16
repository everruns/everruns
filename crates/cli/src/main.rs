// Everruns CLI
//
// Design Decision: Use clap derive for ergonomic argument parsing.
// Design Decision: Support text/json output formats for scripting.
// Design Decision: Use everruns-sdk for API client.
// Design Decision: Credential file (platform config dir/everruns/credentials.json) with env var override.

mod auth;
mod commands;
mod output;

use clap::{Parser, Subcommand};
use everruns_sdk::Everruns;

#[derive(Parser)]
#[command(name = "everruns")]
#[command(about = "Everruns CLI - Manage agents, sessions, and conversations")]
#[command(version)]
pub struct Cli {
    /// API key (defaults to EVERRUNS_API_KEY env var, then credential file)
    #[arg(long, env = "EVERRUNS_API_KEY")]
    pub api_key: Option<String>,

    /// API base URL
    #[arg(long, env = "EVERRUNS_API_URL")]
    pub api_url: Option<String>,

    /// Output format
    #[arg(long, short, global = true, default_value = "text", value_parser = ["text", "json"])]
    pub output: String,

    /// Suppress non-essential output
    #[arg(long, short, global = true)]
    pub quiet: bool,

    /// Profile name for credential storage
    #[arg(long, global = true, default_value = "default")]
    pub profile: String,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Interactive login (localhost OAuth callback)
    Login {
        /// Paste API key directly (headless/SSH fallback)
        #[arg(long)]
        token: bool,
    },

    /// Remove stored credentials
    Logout,

    /// Show current user and org
    Status,

    /// Manage organizations
    Orgs {
        #[command(subcommand)]
        command: Option<OrgsCommand>,
    },

    /// Manage agents
    Agents {
        #[command(subcommand)]
        command: commands::agents::AgentsCommand,
    },

    /// Manage provider connections (API keys)
    Connections {
        #[command(subcommand)]
        command: commands::connections::ConnectionsCommand,
    },

    /// Manage capabilities
    Capabilities {
        #[command(subcommand)]
        command: Option<CapabilitiesCommand>,
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

        /// Max wait time in seconds (default: unlimited)
        #[arg(long)]
        timeout: Option<u64>,

        /// Send message and exit immediately without waiting for response
        #[arg(long)]
        no_stream: bool,
    },
}

#[derive(Subcommand)]
pub enum OrgsCommand {
    /// Interactive organization picker
    Select,
}

#[derive(Subcommand)]
pub enum CapabilitiesCommand {
    /// List available capabilities
    List {
        /// Filter by status
        #[arg(long, default_value = "available", value_parser = ["available", "coming_soon", "all"])]
        status: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Install rustls crypto provider before any TLS usage.
    // See everruns_core::telemetry::install_crypto_provider for the canonical helper.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let cli = Cli::parse();
    let output_format = output::OutputFormat::from_str(&cli.output);

    // Commands that don't need authentication
    match &cli.command {
        Commands::Login { token } => {
            return commands::login::run(cli.api_url.as_deref(), *token, &cli.profile).await;
        }
        Commands::Logout => {
            return commands::logout::run(&cli.profile);
        }
        Commands::Status => {
            return commands::status::run(&cli.profile);
        }
        Commands::Orgs { command } => {
            return match command {
                Some(OrgsCommand::Select) => commands::orgs::run_select(&cli.profile).await,
                None => commands::orgs::run_list(output_format, &cli.profile).await,
            };
        }
        _ => {}
    }

    // Resolve credentials: CLI flags > env var > credential file
    let creds = auth::resolve_credentials(
        cli.api_key.as_deref(),
        cli.api_url.as_deref(),
        Some(&cli.profile),
    )?;
    let api_key = creds.api_key;
    let api_url = creds.api_url;

    // Build SDK client
    let client = Everruns::with_base_url(&api_key, &api_url)?;

    match cli.command {
        Commands::Agents { command } => {
            commands::agents::run(
                command,
                &client,
                &api_url,
                &api_key,
                output_format,
                cli.quiet,
            )
            .await
        }
        Commands::Connections { command } => {
            commands::connections::run(
                command,
                &client,
                &api_url,
                &api_key,
                output_format,
                cli.quiet,
            )
            .await
        }
        Commands::Capabilities { command } => {
            let status = match &command {
                Some(CapabilitiesCommand::List { status }) => status.clone(),
                None => "available".to_string(),
            };
            commands::capabilities::run(&client, output_format, &status).await
        }
        Commands::Sessions { command } => {
            commands::sessions::run(command, &client, output_format, cli.quiet).await
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
        // Already handled above
        Commands::Login { .. } | Commands::Logout | Commands::Status | Commands::Orgs { .. } => {
            unreachable!();
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
                secrets,
                budget_limits,
                budget_soft_limits,
            } = command
            {
                assert_eq!(harness, Some("harness_abc".to_string()));
                assert_eq!(agent, Some("agt_abc".to_string()));
                assert_eq!(title, Some("Test Session".to_string()));
                assert_eq!(model, None);
                assert!(secrets.is_empty());
                assert!(budget_limits.is_empty());
                assert!(budget_soft_limits.is_empty());
            } else {
                panic!("Expected Create command");
            }
        } else {
            panic!("Expected Sessions command");
        }
    }

    #[test]
    fn test_cli_parse_sessions_create_no_harness() {
        let cli = Cli::try_parse_from(["everruns", "sessions", "create"]).unwrap();
        if let Commands::Sessions { command } = cli.command {
            if let commands::sessions::SessionsCommand::Create {
                harness, secrets, ..
            } = command
            {
                assert_eq!(harness, None); // org default
                assert!(secrets.is_empty());
            } else {
                panic!("Expected Create command");
            }
        } else {
            panic!("Expected Sessions command");
        }
    }

    #[test]
    fn test_cli_parse_sessions_create_with_secrets() {
        let cli = Cli::try_parse_from([
            "everruns",
            "sessions",
            "create",
            "--agent",
            "agt_abc",
            "--secret",
            "KEY1=value1",
            "--secret",
            "KEY2=value2",
        ])
        .unwrap();
        if let Commands::Sessions { command } = cli.command {
            if let commands::sessions::SessionsCommand::Create { secrets, .. } = command {
                assert_eq!(secrets.len(), 2);
                assert_eq!(secrets[0], "KEY1=value1");
                assert_eq!(secrets[1], "KEY2=value2");
            } else {
                panic!("Expected Create command");
            }
        } else {
            panic!("Expected Sessions command");
        }
    }

    #[test]
    fn test_cli_parse_sessions_watch() {
        let cli = Cli::try_parse_from(["everruns", "sessions", "watch", "ses_abc"]).unwrap();
        if let Commands::Sessions { command } = cli.command {
            if let commands::sessions::SessionsCommand::Watch { session } = command {
                assert_eq!(session, "ses_abc");
            } else {
                panic!("Expected Watch command");
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
            assert_eq!(timeout, None); // default: no timeout
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
            assert_eq!(timeout, Some(60));
            assert!(no_stream);
        } else {
            panic!("Expected Chat command");
        }
    }

    #[test]
    fn test_cli_parse_output_format() {
        let cli = Cli::try_parse_from(["everruns", "-o", "json", "agents", "list"]).unwrap();
        assert_eq!(cli.output, "json");
    }

    #[test]
    fn test_cli_parse_quiet_flag() {
        let cli = Cli::try_parse_from(["everruns", "-q", "agents", "list"]).unwrap();
        assert!(cli.quiet);

        let cli = Cli::try_parse_from(["everruns", "--quiet", "agents", "list"]).unwrap();
        assert!(cli.quiet);
    }

    #[test]
    fn test_cli_parse_capabilities_bare() {
        let cli = Cli::try_parse_from(["everruns", "capabilities"]).unwrap();
        if let Commands::Capabilities { command } = cli.command {
            assert!(command.is_none()); // bare defaults to list with available
        } else {
            panic!("Expected Capabilities command");
        }
    }

    #[test]
    fn test_cli_parse_capabilities_list() {
        let cli = Cli::try_parse_from(["everruns", "capabilities", "list"]).unwrap();
        if let Commands::Capabilities {
            command: Some(CapabilitiesCommand::List { status }),
        } = cli.command
        {
            assert_eq!(status, "available"); // default
        } else {
            panic!("Expected Capabilities List command");
        }
    }

    #[test]
    fn test_cli_parse_capabilities_list_status() {
        let cli =
            Cli::try_parse_from(["everruns", "capabilities", "list", "--status", "all"]).unwrap();
        if let Commands::Capabilities {
            command: Some(CapabilitiesCommand::List { status }),
        } = cli.command
        {
            assert_eq!(status, "all");
        } else {
            panic!("Expected Capabilities List command");
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

    #[test]
    fn test_cli_parse_login() {
        let cli = Cli::try_parse_from(["everruns", "login"]).unwrap();
        if let Commands::Login { token } = cli.command {
            assert!(!token);
        } else {
            panic!("Expected Login command");
        }
    }

    #[test]
    fn test_cli_parse_login_token() {
        let cli = Cli::try_parse_from(["everruns", "login", "--token"]).unwrap();
        if let Commands::Login { token } = cli.command {
            assert!(token);
        } else {
            panic!("Expected Login command");
        }
    }

    #[test]
    fn test_cli_parse_logout() {
        let cli = Cli::try_parse_from(["everruns", "logout"]).unwrap();
        assert!(matches!(cli.command, Commands::Logout));
    }

    #[test]
    fn test_cli_parse_status() {
        let cli = Cli::try_parse_from(["everruns", "status"]).unwrap();
        assert!(matches!(cli.command, Commands::Status));
    }

    #[test]
    fn test_cli_parse_orgs() {
        let cli = Cli::try_parse_from(["everruns", "orgs"]).unwrap();
        assert!(matches!(cli.command, Commands::Orgs { command: None }));
    }

    #[test]
    fn test_cli_parse_orgs_select() {
        let cli = Cli::try_parse_from(["everruns", "orgs", "select"]).unwrap();
        if let Commands::Orgs {
            command: Some(OrgsCommand::Select),
        } = cli.command
        {
            // ok
        } else {
            panic!("Expected Orgs Select command");
        }
    }

    #[test]
    fn test_cli_parse_profile() {
        let cli = Cli::try_parse_from(["everruns", "--profile", "staging", "status"]).unwrap();
        assert_eq!(cli.profile, "staging");
    }

    #[test]
    fn test_cli_parse_connections_set() {
        let cli = Cli::try_parse_from([
            "everruns",
            "connections",
            "set",
            "daytona",
            "--api-key",
            "test_key_123",
        ])
        .unwrap();
        if let Commands::Connections { command } = cli.command {
            if let commands::connections::ConnectionsCommand::Set { provider, api_key } = command {
                assert_eq!(provider, "daytona");
                assert_eq!(api_key, "test_key_123");
            } else {
                panic!("Expected Set command");
            }
        } else {
            panic!("Expected Connections command");
        }
    }

    #[test]
    fn test_cli_parse_connections_list() {
        let cli = Cli::try_parse_from(["everruns", "connections", "list"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Connections {
                command: commands::connections::ConnectionsCommand::List
            }
        ));
    }

    #[test]
    fn test_cli_parse_connections_remove() {
        let cli = Cli::try_parse_from(["everruns", "connections", "remove", "daytona"]).unwrap();
        if let Commands::Connections { command } = cli.command {
            if let commands::connections::ConnectionsCommand::Remove { provider } = command {
                assert_eq!(provider, "daytona");
            } else {
                panic!("Expected Remove command");
            }
        } else {
            panic!("Expected Connections command");
        }
    }
}
