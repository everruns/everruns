// Session management commands

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::Result;
use clap::Subcommand;
use everruns_sdk::{CreateSessionRequest, Everruns};

#[derive(Subcommand)]
pub enum SessionsCommand {
    /// Create a new session
    Create {
        /// Agent ID (e.g. agt_xxx)
        #[arg(long, short)]
        agent: String,

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
}

pub async fn run(
    command: SessionsCommand,
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        SessionsCommand::Create {
            agent,
            title,
            model,
        } => create(client, output, quiet, agent, title, model).await,
        SessionsCommand::List => list(client, output).await,
        SessionsCommand::Get { session } => get(client, output, session).await,
    }
}

async fn create(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    agent_id: String,
    title: Option<String>,
    model_id: Option<String>,
) -> Result<()> {
    let mut req = CreateSessionRequest::new(&agent_id);
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
            print_field("Agent", &session.agent_id);
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
        print_field("Agent", &session.agent_id);
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
