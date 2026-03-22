// Session management commands

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::Result;
use clap::Subcommand;
use everruns_sdk::{CreateSessionRequest, Everruns};
use std::time::Duration;

#[derive(Subcommand)]
pub enum SessionsCommand {
    /// Create a new session
    Create {
        /// Harness ID (e.g. harness_xxx). Omit to use org default.
        #[arg(long, short = 'H')]
        harness: Option<String>,

        /// Agent ID (optional, e.g. agent_xxx)
        #[arg(long, short)]
        agent: Option<String>,

        /// Session title
        #[arg(long)]
        title: Option<String>,

        /// Model ID override (e.g. mod_xxx)
        #[arg(long)]
        model: Option<String>,
    },

    /// List sessions
    List,

    /// Get session by ID
    Get {
        /// Session ID (e.g. ses_xxx)
        session: String,
    },

    /// Watch session events in real time
    Watch {
        /// Session ID (e.g. ses_xxx)
        session: String,
    },
}

pub async fn run(
    command: SessionsCommand,
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        SessionsCommand::Create {
            harness,
            agent,
            title,
            model,
        } => create(client, output, quiet, harness, agent, title, model).await,
        SessionsCommand::List => list(client, output).await,
        SessionsCommand::Get { session } => get(client, output, session).await,
        SessionsCommand::Watch { session } => watch(client, output, session).await,
    }
}

async fn create(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    harness_id: Option<String>,
    agent_id: Option<String>,
    title: Option<String>,
    model_id: Option<String>,
) -> Result<()> {
    let mut req = CreateSessionRequest::new();
    if let Some(h) = harness_id {
        req = req.harness_id(h);
    }
    if let Some(a) = agent_id {
        req = req.agent_id(a);
    }
    if let Some(t) = title {
        req = req.title(t);
    }
    if let Some(m) = model_id {
        req = req.model_id(m);
    }

    let session = client.sessions().create_with_options(req).await?;

    if output.is_text() {
        if quiet {
            println!("{}", session.id);
        } else {
            println!("Created session: {}", session.id);
            if let Some(agent) = &session.agent_id {
                print_field("Agent", agent);
            }
            let status = format!("{:?}", session.status).to_lowercase();
            print_field("Status", &status);
        }
    } else {
        output.print_value(&session);
    }

    Ok(())
}

async fn list(client: &Everruns, output: OutputFormat) -> Result<()> {
    let response = client.sessions().list().await?;

    if output.is_text() {
        if response.data.is_empty() {
            println!("No sessions found");
            return Ok(());
        }

        print_table_header(&[("ID", 36), ("TITLE", 25), ("STATUS", 10), ("CREATED", 20)]);

        for session in &response.data {
            let title = session.title.as_deref().unwrap_or("-");
            let status = format!("{:?}", session.status).to_lowercase();
            print_table_row(&[
                (&session.id, 36),
                (title, 25),
                (&status, 10),
                (&session.created_at, 20),
            ]);
        }
    } else {
        // Convert to JSON for non-text output
        let data: Vec<serde_json::Value> = response
            .data
            .iter()
            .map(|s| {
                serde_json::json!({
                    "id": s.id,
                    "organization_id": s.organization_id,
                    "agent_id": s.agent_id,
                    "title": s.title,
                    "tags": s.tags,
                    "model_id": s.model_id,
                    "status": format!("{:?}", s.status).to_lowercase(),
                    "created_at": s.created_at,
                    "updated_at": s.updated_at,
                })
            })
            .collect();
        output.print_value(&serde_json::json!({ "data": data }));
    }

    Ok(())
}

async fn get(client: &Everruns, output: OutputFormat, session_id: String) -> Result<()> {
    let session = client
        .sessions()
        .get(&session_id)
        .await
        .map_err(|e| anyhow::anyhow!("Session not found: {} ({})", session_id, e))?;

    if output.is_text() {
        print_field("ID", &session.id);
        if let Some(agent_id) = &session.agent_id {
            print_field("Agent", agent_id);
        }
        let status = format!("{:?}", session.status).to_lowercase();
        print_field("Status", &status);
        if let Some(title) = &session.title {
            print_field("Title", title);
        }
        if !session.tags.is_empty() {
            print_field("Tags", &session.tags.join(", "));
        }
        print_field("Created", &session.created_at);
    } else {
        output.print_value(&session);
    }

    Ok(())
}

async fn watch(client: &Everruns, output: OutputFormat, session_id: String) -> Result<()> {
    // Verify session exists
    let session = client
        .sessions()
        .get(&session_id)
        .await
        .map_err(|e| anyhow::anyhow!("Session not found: {} ({})", session_id, e))?;

    if output.is_text() {
        let title = session.title.as_deref().unwrap_or("(untitled)");
        eprintln!("Watching session {} [{}]", session_id, title);
        eprintln!("Press Ctrl+C to stop\n");
    }

    let poll_interval = Duration::from_millis(500);
    let mut last_event_id: Option<String> = None;

    loop {
        let response = client.events().list(&session_id).await?;

        let events: Vec<_> = if let Some(ref last_id) = last_event_id {
            response
                .data
                .into_iter()
                .skip_while(|e| &e.id != last_id)
                .skip(1)
                .collect()
        } else {
            response.data
        };

        for event in events {
            last_event_id = Some(event.id.clone());

            if output.is_text() {
                format_event_text(&event.event_type, &event.data, &event.ts);
            } else {
                let event_json = serde_json::json!({
                    "id": event.id,
                    "type": event.event_type,
                    "ts": event.ts,
                    "session_id": event.session_id,
                    "data": event.data,
                });
                output.print_value(&event_json);
            }
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Format a single event for human-readable text output.
fn format_event_text(event_type: &str, data: &serde_json::Value, ts: &str) {
    match event_type {
        "turn.started" => {
            eprintln!("[{ts}] Turn started");
        }
        "turn.completed" => {
            eprintln!("[{ts}] Turn completed");
        }
        "turn.failed" => {
            let error = data
                .get("error")
                .and_then(|e| e.as_str())
                .unwrap_or("unknown");
            eprintln!("[{ts}] Turn failed: {error}");
        }
        "turn.cancelled" => {
            eprintln!("[{ts}] Turn cancelled");
        }
        "tool.started" => {
            let name = data
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            eprintln!("[{ts}] Tool started: {name}");
        }
        "tool.completed" => {
            let name = data
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            let truncated = data
                .get("result")
                .and_then(|r| r.as_str())
                .map(|r| truncate_str(r, 200))
                .unwrap_or_default();
            if truncated.is_empty() {
                eprintln!("[{ts}] Tool completed: {name}");
            } else {
                eprintln!("[{ts}] Tool completed: {name} -> {truncated}");
            }
        }
        "tool.call_requested" => {
            let name = data
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("unknown");
            eprintln!("[{ts}] Tool call requested: {name}");
        }
        "output.message.completed" => {
            let content = data
                .get("content")
                .or_else(|| data.get("message").and_then(|m| m.get("content")));
            if let Some(parts) = content.and_then(|c| c.as_array()) {
                for part in parts {
                    if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                        println!("{text}");
                    }
                }
            }
        }
        "output.message.delta" => {
            // Delta text fragments — print inline without newline
            if let Some(text) = data
                .get("delta")
                .and_then(|d| d.get("text"))
                .and_then(|t| t.as_str())
            {
                print!("{text}");
                // Flush so partial lines appear immediately
                use std::io::Write;
                let _ = std::io::stdout().flush();
            }
        }
        "reason.started" | "reason.completed" | "act.started" | "act.completed" => {
            let phase = event_type.replace('.', " ");
            let phase = capitalize_first(&phase);
            eprintln!("[{ts}] {phase}");
        }
        "input.message" => {
            let text = data
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(|c| c.as_str())
                .or_else(|| data.get("content").and_then(|c| c.as_str()));
            if let Some(text) = text {
                let preview = truncate_str(text, 120);
                eprintln!("[{ts}] Input: {preview}");
            } else {
                eprintln!("[{ts}] Input message");
            }
        }
        "session.started" | "session.activated" | "session.idled" => {
            let label = event_type.replace('.', " ");
            let label = capitalize_first(&label);
            eprintln!("[{ts}] {label}");
        }
        "subagent.spawned" => {
            eprintln!("[{ts}] Subagent spawned");
        }
        "subagent.completed" => {
            eprintln!("[{ts}] Subagent completed");
        }
        "subagent.failed" => {
            eprintln!("[{ts}] Subagent failed");
        }
        "subagent.cancelled" => {
            eprintln!("[{ts}] Subagent cancelled");
        }
        _ => {
            eprintln!("[{ts}] {event_type}");
        }
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}...", &s[..max])
    }
}

fn capitalize_first(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => c.to_uppercase().collect::<String>() + chars.as_str(),
    }
}
