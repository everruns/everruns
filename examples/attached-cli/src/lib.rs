//! The two halves of an attachable terminal host.
//!
//! [`serve`] is the host: it puts its session in a [`ControlRegistry`], starts
//! a [`ControlServer`] and keeps a local prompt whose turns are tracked, so an
//! attached `cancel` can reach them. [`run_attached`] is the second process:
//! one subcommand per control operation, printing what a person would want to
//! see. Both are a few lines over `everruns::control`; the protocol, the
//! token and the permission checks live there.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result};
use clap::Subcommand;
use everruns::Session;
use everruns::control::{
    AttachInfo, ControlClient, ControlClientError, ControlRegistry, ControlServer,
};
use tokio::io::{AsyncBufRead, AsyncBufReadExt};

/// What the attached process can do.
#[derive(Clone, Debug, Subcommand)]
pub enum AttachedCommand {
    /// List the host's sessions and whether a turn is running.
    List,
    /// Send a message. Starts a turn when idle, steers the running one when busy.
    Send {
        /// Session id or label.
        session: String,
        /// Message text.
        #[arg(required = true, num_args = 1..)]
        text: Vec<String>,
        /// Return once the message is accepted instead of printing the reply.
        #[arg(long)]
        no_wait: bool,
    },
    /// Cancel the session's running turn.
    Cancel {
        /// Session id or label.
        session: String,
    },
    /// Print the session's output as it happens. Ctrl-C detaches; the session
    /// keeps running.
    Attach {
        /// Session id or label.
        session: String,
        /// Replay the conversation after this durable sequence first
        /// (0 replays everything).
        #[arg(long)]
        after: Option<i32>,
    },
}

/// Run one attached command against the host serving `dir`.
pub async fn run_attached(
    dir: &Path,
    command: AttachedCommand,
    out: &mut impl Write,
) -> Result<()> {
    let client = ControlClient::new(dir);
    match command {
        AttachedCommand::List => {
            for session in client.list().await.map_err(|e| explain(dir, e))? {
                let state = if session.active_turns.is_empty() {
                    "idle".to_owned()
                } else {
                    format!("running {}", session.active_turns.join(" "))
                };
                writeln!(out, "{}\t{}\t{state}", session.label, session.id)?;
            }
        }
        AttachedCommand::Send {
            session,
            text,
            no_wait,
        } => {
            let text = text.join(" ");
            if no_wait {
                let sent = client
                    .send(&session, text)
                    .await
                    .map_err(|e| explain(dir, e))?;
                writeln!(out, "{} {}", sent.disposition, sent.message_id)?;
            } else {
                let (_, turn) = client
                    .send_and_wait(&session, text)
                    .await
                    .map_err(|e| explain(dir, e))?;
                match turn.error {
                    Some(error) if !turn.success => {
                        writeln!(out, "[{}: {error}]", turn.stop_reason)?
                    }
                    _ => writeln!(out, "{}", turn.response)?,
                }
            }
        }
        AttachedCommand::Cancel { session } => {
            let turns = client.cancel(&session).await.map_err(|e| explain(dir, e))?;
            writeln!(out, "cancelled {}", turns.join(" "))?;
        }
        AttachedCommand::Attach { session, after } => {
            let mut stream = client
                .attach(&session, after)
                .await
                .map_err(|e| explain(dir, e))?;
            loop {
                match stream.next().await {
                    Ok(Some(event)) => {
                        if let Some(text) = event.render() {
                            write!(out, "{text}")?;
                            out.flush()?;
                        }
                    }
                    Ok(None) => break,
                    Err(ControlClientError::Closed(reason)) => {
                        writeln!(out, "[detached: {reason}]")?;
                        break;
                    }
                    Err(error) => return Err(explain(dir, error)),
                }
            }
        }
    }
    Ok(())
}

fn explain(dir: &Path, error: ControlClientError) -> anyhow::Error {
    let info = AttachInfo::in_dir(dir);
    let revoked = || {
        anyhow::anyhow!(
            "the host revoked control access; it must issue a new token before anyone can attach"
        )
    };
    match &error {
        // A live socket without a token file is a revoked server, not a
        // missing one.
        ControlClientError::Io(io)
            if io.kind() == std::io::ErrorKind::NotFound
                && info.socket.exists()
                && !info.token_file.exists() =>
        {
            revoked()
        }
        ControlClientError::Io(io) if io.kind() == std::io::ErrorKind::NotFound => {
            anyhow::anyhow!("no host is serving here (start one with `attached-cli host`): {io}")
        }
        ControlClientError::Rejected { code, .. } if code == "unauthorized" => revoked(),
        _ => anyhow::Error::new(error),
    }
}

/// Serve `session` as `main` on `dir`, and read prompts from `input` until it
/// ends. Every local prompt goes through the registry, so the attached side
/// can cancel it.
pub async fn serve(
    dir: &Path,
    session: Session,
    input: impl AsyncBufRead + Unpin,
    out: &mut impl Write,
) -> Result<()> {
    let registry = ControlRegistry::new();
    let id = registry.register("main", &session);
    let server = ControlServer::start(dir, registry.clone())
        .with_context(|| format!("serving on {}", dir.display()))?;
    writeln!(out, "session main ({id})")?;
    writeln!(out, "control dir {}", server.attach_info().dir.display())?;
    writeln!(
        out,
        "type a prompt, or drive it from another terminal; EOF stops"
    )?;
    out.flush()?;

    let mut lines = input.lines();
    while let Some(line) = lines.next_line().await? {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let sent = registry.send("main", line).await?;
        let turn = sent.wait().await?;
        match turn.error {
            Some(error) if !turn.success => writeln!(out, "[{:?}: {error}]", turn.stop_reason)?,
            _ => writeln!(out, "{}", turn.response)?,
        }
        out.flush()?;
    }
    // Dropping the server removes the socket and token; open attach streams
    // end with `shutdown`.
    server.shutdown();
    Ok(())
}
