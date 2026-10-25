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
#[cfg(test)]
mod reference;
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

    /// Profile name for credential storage (default: the profile you signed
    /// in to last)
    #[arg(long, global = true, env = "EVERRUNS_PROFILE")]
    pub profile: Option<String>,

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

    /// Show who the server says you are, and your token's access
    Whoami,

    /// Organizations you belong to, and which one is active
    Orgs {
        #[command(subcommand)]
        command: Option<OrgsCommand>,
    },

    /// Agent definitions and their configuration
    Agents {
        #[command(subcommand)]
        command: commands::agents::AgentsCommand,
    },

    /// Your LLM provider API keys
    Connections {
        #[command(subcommand)]
        command: commands::connections::ConnectionsCommand,
    },

    /// Running and archived sessions, their state and participants
    Sessions {
        #[command(subcommand)]
        command: commands::sessions::SessionsCommand,
    },

    /// Sync files between a local folder and a session
    Files {
        #[command(subcommand)]
        command: commands::files::FilesCommand,
    },

    /// Run Everruns on this machine with Docker
    Docker {
        #[command(subcommand)]
        command: commands::docker::DockerCommand,
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
    // Every failure passes through one place that explains the ones a person
    // can act on, such as a read-only token refused a write.
    Box::pin(run()).await.map_err(commands::api::explain)
}

async fn run() -> anyhow::Result<()> {
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
            // Signing in names its profile, `default` unless one is given;
            // it then becomes the current one.
            return Box::pin(commands::login::run(
                cli.api_url.as_deref(),
                *token,
                cli.profile.as_deref().unwrap_or("default"),
            ))
            .await;
        }
        Commands::Logout => {
            return commands::logout::run(&auth::profile_name(cli.profile.as_deref()));
        }
        Commands::Status => {
            return commands::status::run(&auth::profile_name(cli.profile.as_deref()));
        }
        Commands::Orgs { command } => {
            let profile = auth::profile_name(cli.profile.as_deref());
            return match command {
                Some(OrgsCommand::Select) => Box::pin(commands::orgs::run_select(&profile)).await,
                None => Box::pin(commands::orgs::run_list(output_format, &profile)).await,
            };
        }
        Commands::Docker { .. } => {
            let Commands::Docker { command } = cli.command else {
                unreachable!()
            };
            return Box::pin(commands::docker::run(command)).await;
        }
        _ => {}
    }

    // Resolve credentials: CLI flags > env var > credential file
    let creds = auth::resolve_credentials(
        cli.api_key.as_deref(),
        cli.api_url.as_deref(),
        cli.profile.as_deref(),
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
        Commands::Whoami => Box::pin(commands::whoami::run(&client, output_format)).await,
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
        Commands::Login { .. }
        | Commands::Logout
        | Commands::Status
        | Commands::Orgs { .. }
        | Commands::Docker { .. } => {
            unreachable!()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

#[cfg(test)]
mod reference_pages {
    use super::*;
    use clap::CommandFactory;

    /// The generated CLI reference in `docs/reference/cli/` matches this binary.
    ///
    /// The pages are what a web search for an `everruns` command lands on, so
    /// a stale page sends a reader (person or agent) to a flag that moved.
    #[test]
    fn the_cli_reference_pages_are_current() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");
        let dir = std::path::Path::new(root).join(reference::DIR);
        let pages = reference::pages(&contract::augment(Cli::command()));

        if std::env::var("UPDATE_CLI_REFERENCE").is_ok() {
            if dir.exists() {
                std::fs::remove_dir_all(&dir).expect("clear reference dir");
            }
            std::fs::create_dir_all(&dir).expect("create reference dir");
            for (name, text) in &pages {
                std::fs::write(dir.join(name), text).expect("write reference page");
            }
            return;
        }

        let mut stale = Vec::new();
        for (name, text) in &pages {
            match std::fs::read_to_string(dir.join(name)) {
                Ok(on_disk) if on_disk == *text => {}
                _ => stale.push(name.clone()),
            }
        }
        let on_disk: Vec<String> = std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(Result::ok)
                    .map(|entry| entry.file_name().to_string_lossy().to_string())
                    .filter(|name| !pages.contains_key(name))
                    .collect()
            })
            .unwrap_or_default();
        stale.extend(on_disk);
        assert!(
            stale.is_empty(),
            "docs/reference/cli is stale ({stale:?}). Run \
             `UPDATE_CLI_REFERENCE=1 cargo test -p everruns-cli the_cli_reference`."
        );
    }
}
