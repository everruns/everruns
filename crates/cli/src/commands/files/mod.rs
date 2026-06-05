// File sync CLI commands
//
// One-shot operations (cat/write/rm/mkdir/mv/cp/grep/stat) are backed by the
// SDK's typed `session_files()` client — see `ops.rs`.
//
// The bidirectional `sync`/`push`/`pull` paths still use `RemoteClient` (raw
// reqwest) because they rely on the server's per-entry `content_hash` for
// change detection, which the SDK's `FileInfo`/`SessionFile` models (v0.1.9) do
// not expose. Migrate `RemoteClient` once the SDK surfaces `content_hash`.
//
// Design Decision: All file operations grouped under `everruns files` subcommand.

pub mod ls;
pub mod ops;
pub mod pull;
pub mod push;
pub mod remote;
pub mod state;
pub mod sync_cmd;
pub mod sync_engine;

use crate::output::OutputFormat;
use anyhow::Result;
use clap::Subcommand;
use everruns_sdk::Everruns;

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

    /// Print a session file's content
    Cat {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Remote file path
        path: String,

        /// Write to a local file instead of stdout
        #[arg(long, short)]
        output: Option<String>,
    },

    /// Write a session file from a local file or stdin
    Write {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Remote file path
        path: String,

        /// Local file to upload (defaults to stdin)
        #[arg(long)]
        from: Option<String>,

        /// Mark the file read-only
        #[arg(long)]
        readonly: bool,
    },

    /// Remove a session file or directory
    Rm {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Remote path
        path: String,

        /// Remove directories recursively
        #[arg(long, short)]
        recursive: bool,
    },

    /// Create a directory in the session workspace
    Mkdir {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Remote directory path
        path: String,
    },

    /// Move (rename) a session file
    Mv {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Source path
        src: String,

        /// Destination path
        dest: String,
    },

    /// Copy a session file
    Cp {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Source path
        src: String,

        /// Destination path
        dest: String,
    },

    /// Search session file contents
    Grep {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Search pattern
        pattern: String,

        /// Restrict to a path glob (e.g. "*.rs")
        #[arg(long)]
        path: Option<String>,
    },

    /// Show metadata for a session file or directory
    Stat {
        /// Session ID (e.g. ses_xxx)
        #[arg(long, short)]
        session: String,

        /// Remote path
        path: String,
    },
}

#[allow(clippy::too_many_arguments)]
pub async fn run(
    command: FilesCommand,
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
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
                org_id,
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
                api_url, api_key, org_id, output, quiet, session, local_dir, delete, dry_run,
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
                api_url, api_key, org_id, output, quiet, session, local_dir, delete, dry_run,
            )
            .await
        }
        FilesCommand::Ls {
            session,
            path,
            recursive,
            long,
        } => {
            ls::run(
                api_url, api_key, org_id, output, session, path, recursive, long,
            )
            .await
        }
        FilesCommand::Cat {
            session,
            path,
            output: out_file,
        } => ops::cat(client, &session, &path, out_file.as_deref()).await,
        FilesCommand::Write {
            session,
            path,
            from,
            readonly,
        } => {
            ops::write(
                client,
                output,
                quiet,
                &session,
                &path,
                from.as_deref(),
                readonly,
            )
            .await
        }
        FilesCommand::Rm {
            session,
            path,
            recursive,
        } => ops::rm(client, output, quiet, &session, &path, recursive).await,
        FilesCommand::Mkdir { session, path } => {
            ops::mkdir(client, output, quiet, &session, &path).await
        }
        FilesCommand::Mv { session, src, dest } => {
            ops::mv(client, output, quiet, &session, &src, &dest).await
        }
        FilesCommand::Cp { session, src, dest } => {
            ops::cp(client, output, quiet, &session, &src, &dest).await
        }
        FilesCommand::Grep {
            session,
            pattern,
            path,
        } => ops::grep(client, output, &session, &pattern, path.as_deref()).await,
        FilesCommand::Stat { session, path } => ops::stat(client, output, &session, &path).await,
    }
}
