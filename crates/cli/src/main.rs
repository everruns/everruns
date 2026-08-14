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
    #[arg(long, short, global = true, default_value = "text", value_parser = ["text", "json", "yaml"])]
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
        /// Filter by status
        #[arg(long, default_value = "available", value_parser = ["available", "coming_soon", "all"])]
        status: String,

        #[command(subcommand)]
        command: Option<CapabilitiesCommand>,
    },

    /// Discover plugins
    Plugins {
        #[command(subcommand)]
        command: commands::plugins::PluginsCommands,
    },

    /// Manage skills
    Skills {
        #[command(subcommand)]
        command: commands::skills::SkillsCommands,
    },

    /// Manage knowledge bases
    KnowledgeBases {
        #[command(subcommand)]
        command: commands::knowledge_bases::KnowledgeBasesCommands,
    },

    /// Manage sessions
    Sessions {
        #[command(subcommand)]
        command: commands::sessions::SessionsCommand,
    },

    /// Manage an agent's schedule triggers
    Triggers {
        /// Agent ID that owns the triggers
        #[arg(long)]
        agent: String,
        #[command(subcommand)]
        command: commands::triggers::TriggersCommand,
    },

    /// Manage a session's participants
    Participants {
        /// Session ID that owns the participants
        #[arg(long)]
        session: String,
        #[command(subcommand)]
        command: commands::participants::ParticipantsCommand,
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
    everruns_provider::install_ring_crypto_provider();

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
    let org_id = creds.org_id;

    // Build SDK client
    let client = if let Some(org_id) = org_id.as_deref() {
        Everruns::with_base_url_and_org_id(&api_key, &api_url, org_id)?
    } else {
        Everruns::with_base_url(&api_key, &api_url)?
    };

    match cli.command {
        Commands::Agents { command } => {
            commands::agents::run(
                command,
                &client,
                &api_url,
                &api_key,
                org_id.as_deref(),
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
        Commands::Capabilities { status, command } => {
            let status = match &command {
                Some(CapabilitiesCommand::List { status }) => status.clone(),
                None => status,
            };
            commands::capabilities::run(&client, output_format, &status).await
        }
        Commands::Plugins { command } => {
            commands::plugins::run(
                command,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
            )
            .await
        }
        Commands::Skills { command } => {
            commands::skills::run(
                command,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
            )
            .await
        }
        Commands::KnowledgeBases { command } => {
            commands::knowledge_bases::run(
                command,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
            )
            .await
        }
        Commands::Sessions { command } => {
            commands::sessions::run(
                command,
                &client,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
                cli.quiet,
            )
            .await
        }
        Commands::Triggers { agent, command } => {
            commands::triggers::run(
                command,
                agent,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
                cli.quiet,
            )
            .await
        }
        Commands::Participants { session, command } => {
            commands::participants::run(
                command,
                session,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
                cli.quiet,
            )
            .await
        }
        Commands::Files { command } => {
            commands::files::run(
                command,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
                cli.quiet,
            )
            .await
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
            unreachable!()
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
                locale,
                agent_identity,
                system_prompt,
                tags,
                capabilities,
                hints,
                hints_json,
                network_allow,
                network_block,
                max_iterations,
                secrets,
                budget_limits,
                budget_soft_limits,
            } = command
            {
                assert_eq!(harness, Some("harness_abc".to_string()));
                assert_eq!(agent, Some("agt_abc".to_string()));
                assert_eq!(title, Some("Test Session".to_string()));
                assert_eq!(model, None);
                assert_eq!(locale, None);
                assert_eq!(agent_identity, None);
                assert_eq!(system_prompt, None);
                assert!(tags.is_empty());
                assert!(capabilities.is_empty());
                assert!(hints.is_empty());
                assert_eq!(hints_json, None);
                assert!(network_allow.is_empty());
                assert!(network_block.is_empty());
                assert_eq!(max_iterations, None);
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
    fn test_cli_parse_triggers_create() {
        let cli = Cli::try_parse_from([
            "everruns",
            "triggers",
            "--agent",
            "agent_abc",
            "create",
            "--cron",
            "30 * * * *",
            "--timezone",
            "America/Chicago",
            "--session-mode",
            "session-per-invocation",
            "--message",
            "Prepare report",
        ])
        .unwrap();
        if let Commands::Triggers { agent, command } = cli.command {
            assert_eq!(agent, "agent_abc");
            assert!(matches!(
                command,
                commands::triggers::TriggersCommand::Create {
                    session_mode: commands::triggers::SessionMode::SessionPerInvocation,
                    ..
                }
            ));
        } else {
            panic!("Expected Triggers command");
        }
    }

    #[test]
    fn test_cli_parse_trigger_run_now() {
        let cli = Cli::try_parse_from([
            "everruns",
            "triggers",
            "--agent",
            "agent_abc",
            "run-now",
            "trg_abc",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Triggers {
                command: commands::triggers::TriggersCommand::RunNow { .. },
                ..
            }
        ));
    }

    #[test]
    fn test_cli_parse_participants_add() {
        let cli = Cli::try_parse_from([
            "everruns",
            "participants",
            "--session",
            "session_abc",
            "add",
            "--agent",
            "agent_guest",
        ])
        .unwrap();
        if let Commands::Participants { session, command } = cli.command {
            assert_eq!(session, "session_abc");
            assert!(matches!(
                command,
                commands::participants::ParticipantsCommand::Add { agent }
                    if agent == "agent_guest"
            ));
        } else {
            panic!("Expected Participants command");
        }
    }

    #[test]
    fn test_cli_parse_sessions_create_new_fields() {
        let cli = Cli::try_parse_from([
            "everruns",
            "sessions",
            "create",
            "--agent",
            "agent_abc",
            "--agent-identity",
            "identity_abc",
            "--system-prompt",
            "Be concise",
            "--locale",
            "uk-UA",
            "--tag",
            "debugging",
            "--capability",
            "web_fetch={\"timeout\":10}",
            "--hint",
            "setup_connection=true",
            "--hints-json",
            "{\"rich_media\":true}",
            "--network-allow",
            "api.example.com",
            "--network-block",
            "internal.example.com",
            "--max-iterations",
            "8",
        ])
        .unwrap();
        if let Commands::Sessions { command } = cli.command {
            if let commands::sessions::SessionsCommand::Create {
                agent_identity,
                system_prompt,
                locale,
                tags,
                capabilities,
                hints,
                hints_json,
                network_allow,
                network_block,
                max_iterations,
                ..
            } = command
            {
                assert_eq!(agent_identity, Some("identity_abc".to_string()));
                assert_eq!(system_prompt, Some("Be concise".to_string()));
                assert_eq!(locale, Some("uk-UA".to_string()));
                assert_eq!(tags, vec!["debugging".to_string()]);
                assert_eq!(capabilities, vec!["web_fetch={\"timeout\":10}".to_string()]);
                assert_eq!(hints, vec!["setup_connection=true".to_string()]);
                assert_eq!(hints_json, Some("{\"rich_media\":true}".to_string()));
                assert_eq!(network_allow, vec!["api.example.com".to_string()]);
                assert_eq!(network_block, vec!["internal.example.com".to_string()]);
                assert_eq!(max_iterations, Some(8));
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

        let cli = Cli::try_parse_from(["everruns", "-o", "yaml", "agents", "list"]).unwrap();
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
    fn test_cli_parse_capabilities_bare() {
        let cli = Cli::try_parse_from(["everruns", "capabilities"]).unwrap();
        if let Commands::Capabilities { command, status } = cli.command {
            assert_eq!(status, "available");
            assert!(command.is_none()); // bare defaults to list with available
        } else {
            panic!("Expected Capabilities command");
        }
    }

    #[test]
    fn test_cli_parse_capabilities_bare_status() {
        let cli = Cli::try_parse_from(["everruns", "capabilities", "--status", "all"]).unwrap();
        if let Commands::Capabilities { command, status } = cli.command {
            assert!(command.is_none());
            assert_eq!(status, "all");
        } else {
            panic!("Expected Capabilities command");
        }
    }

    #[test]
    fn test_cli_parse_capabilities_list() {
        let cli = Cli::try_parse_from(["everruns", "capabilities", "list"]).unwrap();
        if let Commands::Capabilities {
            status: _,
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
            status: _,
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
            "--api-key-stdin",
        ])
        .unwrap();
        if let Commands::Connections { command } = cli.command {
            if let commands::connections::ConnectionsCommand::Set {
                provider,
                api_key_stdin,
            } = command
            {
                assert_eq!(provider, "daytona");
                assert!(api_key_stdin);
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

    // Regression tests for fix(cli): avoid API key in connections set args (#1518).
    //
    // The old `--api-key <value>` flag leaked the provider secret into shell
    // history and `ps` output. The fix removed the flag entirely and routed
    // input through `--api-key-stdin` or the interactive password prompt.
    // These tests lock in that contract at the clap-parser boundary.

    #[test]
    fn connections_set_rejects_legacy_api_key_flag() {
        // Passing `--api-key <value>` must now fail parsing so the secret
        // cannot end up in argv / shell history.
        let result = Cli::try_parse_from([
            "everruns",
            "connections",
            "set",
            "daytona",
            "--api-key",
            "leaked_secret_value",
        ]);
        assert!(
            result.is_err(),
            "--api-key argv flag must be rejected to prevent argv/shell-history leaks"
        );
    }

    #[test]
    fn connections_set_defaults_api_key_stdin_to_false_for_interactive_prompt() {
        // Without `--api-key-stdin`, the flag is false and `run` drops into
        // the dialoguer Password prompt — the secret never touches argv.
        let cli = Cli::try_parse_from(["everruns", "connections", "set", "daytona"]).unwrap();
        if let Commands::Connections { command } = cli.command {
            if let commands::connections::ConnectionsCommand::Set {
                provider,
                api_key_stdin,
            } = command
            {
                assert_eq!(provider, "daytona");
                assert!(
                    !api_key_stdin,
                    "omitting --api-key-stdin must default to interactive prompt"
                );
            } else {
                panic!("Expected Set command");
            }
        } else {
            panic!("Expected Connections command");
        }
    }

    #[test]
    fn connections_set_rejects_positional_api_key_after_provider() {
        // A stray positional arg after `provider` must not bind to anything;
        // clap should reject it rather than silently consuming the secret.
        let result = Cli::try_parse_from([
            "everruns",
            "connections",
            "set",
            "daytona",
            "leaked_secret_value",
        ]);
        assert!(
            result.is_err(),
            "extra positional args after provider must be rejected"
        );
    }

    #[test]
    fn connections_set_requires_provider() {
        let result = Cli::try_parse_from(["everruns", "connections", "set"]);
        assert!(result.is_err(), "provider arg is required");
    }
    #[test]
    fn parses_plugin_lifecycle_commands() {
        use crate::commands::plugins::PluginsCommands;

        let cli = Cli::try_parse_from([
            "everruns",
            "plugins",
            "install",
            "marketplace-id",
            "plugin-name",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Plugins {
                command: PluginsCommands::Install { marketplace_id, plugin_name }
            } if marketplace_id == "marketplace-id" && plugin_name == "plugin-name"
        ));

        let cli = Cli::try_parse_from(["everruns", "plugins", "uninstall", "plugin-id"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Plugins { command: PluginsCommands::Uninstall { id } } if id == "plugin-id"
        ));
    }

    #[test]
    fn parses_skill_lifecycle_commands() {
        use crate::commands::skills::SkillsCommands;
        let cli =
            Cli::try_parse_from(["everruns", "skills", "create", "skills/demo/SKILL.md"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Skills { command: SkillsCommands::Create { path } }
                if path == std::path::Path::new("skills/demo/SKILL.md")
        ));

        let cli = Cli::try_parse_from(["everruns", "skills", "delete", "skill-id"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Skills { command: SkillsCommands::Delete { id } } if id == "skill-id"
        ));
    }

    #[test]
    fn parses_knowledge_base_lifecycle_commands() {
        use crate::commands::knowledge_bases::KnowledgeBasesCommands;

        let cli = Cli::try_parse_from([
            "everruns",
            "knowledge-bases",
            "create",
            "Runbooks",
            "--description",
            "Operations guides",
            "--embedding-model-id",
            "model-id",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::KnowledgeBases {
                command: KnowledgeBasesCommands::Create {
                    name,
                    description: Some(description),
                    embedding_model_id: Some(embedding_model_id),
                }
            } if name == "Runbooks" && description == "Operations guides" && embedding_model_id == "model-id"
        ));

        let cli = Cli::try_parse_from(["everruns", "knowledge-bases", "delete", "kb-id"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::KnowledgeBases { command: KnowledgeBasesCommands::Delete { id } } if id == "kb-id"
        ));
    }
}
