// File sync CLI commands
//
// TODO(sdk): Replace RemoteClient (raw reqwest) with SDK session_files() methods.
// SDK v0.1.5 ships session filesystem support (https://github.com/everruns/sdk/issues/60 resolved).
// Migration is tracked separately — involves adapting to SDK's FileInfo/SessionFile types.
//
// Design Decision: All file operations grouped under `everruns files` subcommand.

pub mod ls;
pub mod pull;
pub mod push;
pub mod remote;
pub mod state;
pub mod sync_cmd;
pub mod sync_engine;

use crate::output::OutputFormat;
use anyhow::Result;
use clap::Subcommand;

#[derive(Subcommand)]
pub enum FilesCommand {
    /// Bidirectional live sync between local directory and session workspace
    Sync {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Local directory to sync (default: current directory)
        #[arg(default_value = ".")]
        local_dir: String,

        /// Remote poll interval in seconds
        #[arg(long, default_value = "3")]
        interval: u64,

        /// Conflict strategy
        #[arg(long, default_value = "last-write-wins", value_parser = ["last-write-wins", "local-wins", "remote-wins"])]
        conflict: String,

        /// Additional exclude patterns (repeatable)
        #[arg(long)]
        exclude: Vec<String>,

        /// Don't read .gitignore
        #[arg(long)]
        no_gitignore: bool,

        /// Show what would sync without making changes
        #[arg(long)]
        dry_run: bool,

        /// Delete files on one side when deleted on the other
        #[arg(long)]
        delete: bool,

        /// Show every file operation
        #[arg(long, short)]
        verbose: bool,
    },

    /// One-shot push local files to session workspace
    Push {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Local directory to push from (default: current directory)
        #[arg(default_value = ".")]
        local_dir: String,

        /// Delete remote files not present locally
        #[arg(long)]
        delete: bool,

        /// Show what would be pushed
        #[arg(long)]
        dry_run: bool,
    },

    /// One-shot pull session workspace files to local directory
    Pull {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Local directory to pull into (default: current directory)
        #[arg(default_value = ".")]
        local_dir: String,

        /// Delete local files not present remotely
        #[arg(long)]
        delete: bool,

        /// Show what would be pulled
        #[arg(long)]
        dry_run: bool,
    },

    /// List files in session workspace
    Ls {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Remote path to list (default: root)
        #[arg(default_value = "/")]
        path: String,

        /// List recursively
        #[arg(long, short)]
        recursive: bool,

        /// Show size and dates
        #[arg(long, short)]
        long: bool,
    },
}

pub async fn run(
    command: FilesCommand,
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        FilesCommand::Sync {
            session,
            local_dir,
            interval,
            conflict,
            exclude,
            no_gitignore,
            dry_run,
            delete,
            verbose,
        } => {
            sync_cmd::run(
                api_url,
                api_key,
                quiet,
                session,
                local_dir,
                interval,
                conflict,
                exclude,
                no_gitignore,
                dry_run,
                delete,
                verbose,
            )
            .await
        }
        FilesCommand::Push {
            session,
            local_dir,
            delete,
            dry_run,
        } => {
            push::run(
                api_url, api_key, output, quiet, session, local_dir, delete, dry_run,
            )
            .await
        }
        FilesCommand::Pull {
            session,
            local_dir,
            delete,
            dry_run,
        } => {
            pull::run(
                api_url, api_key, output, quiet, session, local_dir, delete, dry_run,
            )
            .await
        }
        FilesCommand::Ls {
            session,
            path,
            recursive,
            long,
        } => ls::run(api_url, api_key, output, session, path, recursive, long).await,
    }
}
