#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Everruns CLI
//
// Design Decision: Use clap derive for ergonomic argument parsing.
// Design Decision: Support text/json output formats for scripting.
// Design Decision: No SDK. Platform commands go through the shared contract
// mapper and POST /v1/commands; the rest is plain HTTP (see contract.rs).
// Design Decision: Credential file (platform config dir/everruns/credentials.json) with env var override.

mod auth;
mod browser;
mod commands;
mod contract;
mod events;
mod output;
mod user_dirs;

use clap::{CommandFactory, FromArgMatches, Parser, Subcommand};

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    everruns_contracts::install_default_crypto_provider();

    // The CLI's own tree, plus every contract command it does not hand-write.
    // Parsing happens once, against the merged tree, so a mounted command gets
    // the same global flags and the same help as a hand-written one.
    let root = contract::augment(Cli::command());
    let matches = root.clone().get_matches();

    let output_format = output::OutputFormat::from_str(
        matches
            .get_one::<String>("output")
            .map(String::as_str)
            .unwrap_or("text"),
    );

    // A contract command is decided before credentials are resolved, so a
    // hand-written command that needs none (`login`, `status`) is not made to
    // produce one.
    if let Some((contract, leaf)) = contract::selected(&matches) {
        let creds = auth::resolve_credentials(
            matches.get_one::<String>("api_key").map(String::as_str),
            matches.get_one::<String>("api_url").map(String::as_str),
            matches.get_one::<String>("profile").map(String::as_str),
        )?;
        return Box::pin(contract::dispatch(
            contract,
            leaf,
            &creds.api_url,
            &creds.api_key,
            creds.org_id.as_deref(),
            output_format,
        ))
        .await;
    }

    let cli = Cli::from_arg_matches(&matches)?;

    if let Commands::Agents { command } = &cli.command
        && let Some(result) = commands::agents::run_local(command, output_format)
    {
        return result;
    }

    // Commands that don't need authentication
    match &cli.command {
        Commands::Login { token } => {
            return Box::pin(commands::login::run(
                cli.api_url.as_deref(),
                *token,
                &cli.profile,
            ))
            .await;
        }
        Commands::Logout => {
            return commands::logout::run(&cli.profile);
        }
        Commands::Status => {
            return commands::status::run(&cli.profile);
        }
        Commands::Orgs { command } => {
            return match command {
                Some(OrgsCommand::Select) => {
                    Box::pin(commands::orgs::run_select(&cli.profile)).await
                }
                None => Box::pin(commands::orgs::run_list(output_format, &cli.profile)).await,
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

    let client = commands::api::ApiClient::new(&api_url, &api_key, org_id.as_deref());

    // Each arm's future is boxed rather than awaited inline. Without it every
    // command's state machine is inlined into main's, which the optimizer then
    // duplicates across codegen units — main::{{closure}} and its drop glue cost
    // ~200 KiB of the release binary. One allocation per process is free here:
    // the CLI runs exactly one command and exits.
    match cli.command {
        Commands::Agents { command } => {
            Box::pin(commands::agents::run(
                command,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
                cli.quiet,
            ))
            .await
        }
        Commands::Connections { command } => {
            Box::pin(commands::connections::run(
                command,
                &api_url,
                &api_key,
                output_format,
                cli.quiet,
            ))
            .await
        }
        Commands::Sessions { command } => {
            Box::pin(commands::sessions::run(
                command,
                client,
                output_format,
                cli.quiet,
            ))
            .await
        }
        Commands::Files { command } => {
            Box::pin(commands::files::run(
                command,
                &api_url,
                &api_key,
                org_id.as_deref(),
                output_format,
                cli.quiet,
            ))
            .await
        }
        Commands::Chat {
            message,
            session,
            timeout,
            no_stream,
        } => {
            Box::pin(commands::chat::run(
                client,
                output_format,
                cli.quiet,
                message,
                session,
                timeout,
                no_stream,
            ))
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
                virtual_user,
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
                reason,
            } = command
            {
                assert_eq!(harness, Some("harness_abc".to_string()));
                assert_eq!(agent, Some("agt_abc".to_string()));
                assert_eq!(title, Some("Test Session".to_string()));
                assert_eq!(model, None);
                assert_eq!(locale, None);
                assert_eq!(virtual_user, None);
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
                assert_eq!(reason, None);
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
    fn test_cli_parse_sessions_create_new_fields() {
        let cli = Cli::try_parse_from([
            "everruns",
            "sessions",
            "create",
            "--agent",
            "agent_abc",
            "--virtual-user",
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
                virtual_user,
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
                assert_eq!(virtual_user, Some("identity_abc".to_string()));
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
        let cli = Cli::try_parse_from(["everruns", "-o", "json", "status"]).unwrap();
        assert_eq!(cli.output, "json");

        let cli = Cli::try_parse_from(["everruns", "-o", "yaml", "status"]).unwrap();
        assert_eq!(cli.output, "yaml");
    }

    #[test]
    fn test_cli_parse_quiet_flag() {
        let cli = Cli::try_parse_from(["everruns", "-q", "status"]).unwrap();
        assert!(cli.quiet);

        let cli = Cli::try_parse_from(["everruns", "--quiet", "status"]).unwrap();
        assert!(cli.quiet);
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

    /// Regression: export's file flag used to share the global `--output`
    /// id, so the format default ("text") became the file path.
    #[test]
    fn sessions_export_writes_stdout_unless_out_is_given() {
        let cli = Cli::try_parse_from(["everruns", "sessions", "export", "ses_1"]).unwrap();
        let Commands::Sessions {
            command: commands::sessions::SessionsCommand::Export { out, .. },
        } = cli.command
        else {
            panic!("expected sessions export");
        };
        assert_eq!(out, None);

        let cli = Cli::try_parse_from([
            "everruns", "-o", "json", "sessions", "export", "ses_1", "--out", "a.jsonl",
        ])
        .unwrap();
        assert_eq!(cli.output, "json");
        let Commands::Sessions {
            command: commands::sessions::SessionsCommand::Export { out, .. },
        } = cli.command
        else {
            panic!("expected sessions export");
        };
        assert_eq!(out.as_deref(), Some("a.jsonl"));
    }

    #[test]
    fn test_cli_invalid_output_format() {
        let result = Cli::try_parse_from(["everruns", "-o", "invalid", "status"]);
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
}

#[cfg(test)]
mod contract_golden {
    use super::*;
    use clap::CommandFactory;

    /// The command line this CLI ships, as a diffable rendering: the commands
    /// written by hand here, and the contract commands mounted beside them.
    ///
    /// Humans and their scripts already type these words, so what they depend
    /// on is pinned rather than described. It guards in both directions now: a
    /// hand-written flag that moves shows up as a changed line, and so does a
    /// command added or re-spelled in the control plane, which reaches this
    /// binary through the contract without anyone editing this crate.
    const GOLDEN: &str = include_str!("../contract.golden");

    #[test]
    fn the_shipped_command_line_has_not_changed() {
        let rendered = everruns_cli_contract::render::tree(&contract::augment(Cli::command()));

        if std::env::var("UPDATE_CLI_CONTRACT").is_ok() {
            std::fs::write(
                concat!(env!("CARGO_MANIFEST_DIR"), "/contract.golden"),
                format!("{rendered}\n"),
            )
            .expect("write contract golden");
            return;
        }

        if let Some(report) = everruns_cli_contract::render::diff(GOLDEN, &rendered) {
            panic!(
                "the CLI's command line changed:\n\n{report}\nEvery line here is \
                 something a person or a script already types. If the change is \
                 deliberate, run `UPDATE_CLI_CONTRACT=1 cargo test -p everruns-cli \
                 the_shipped_command_line` and say in the commit message what moved \
                 and why it is safe."
            );
        }
    }
}
