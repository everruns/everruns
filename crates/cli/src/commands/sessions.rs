// Session management commands

use crate::client::{Client, ClientError};
use crate::output::{print_field, print_table_header, print_table_row, OutputFormat};
use anyhow::Result;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Subcommand)]
pub enum SessionsCommand {
    /// Create a new session
    Create {
        /// Agent ID
        #[arg(long, short)]
        agent: Uuid,

        /// Session title
        #[arg(long)]
        title: Option<String>,

        /// Model ID override
        #[arg(long)]
        model: Option<Uuid>,

        /// Tags (repeatable)
        #[arg(long, short)]
        tag: Vec<String>,
    },

    /// List sessions for an agent
    List {
        /// Agent ID
        #[arg(long, short)]
        agent: Uuid,
    },

    /// Get session by ID
    Get {
        /// Agent ID
        #[arg(long, short)]
        agent: Uuid,

        /// Session ID
        #[arg(long, short)]
        session: Uuid,
    },
}

/// Request to create a session
#[derive(Debug, Serialize)]
struct CreateSessionRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model_id: Option<Uuid>,
}

/// Session response from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub agent_id: Uuid,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub model_id: Option<Uuid>,
    pub status: String,
    pub created_at: String,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ListResponse<T> {
    data: Vec<T>,
}

pub async fn run(
    command: SessionsCommand,
    client: &Client,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        SessionsCommand::Create {
            agent,
            title,
            model,
            tag,
        } => create(client, output, quiet, agent, title, model, tag).await,
        SessionsCommand::List { agent } => list(client, output, agent).await,
        SessionsCommand::Get { agent, session } => get(client, output, agent, session).await,
    }
}

async fn create(
    client: &Client,
    output: OutputFormat,
    quiet: bool,
    agent_id: Uuid,
    title: Option<String>,
    model_id: Option<Uuid>,
    tags: Vec<String>,
) -> Result<()> {
    let request = CreateSessionRequest {
        title,
        tags,
        model_id,
    };

    let session: Session = client
        .post(&format!("/v1/agents/{}/sessions", agent_id), &request)
        .await?;

    if output.is_text() {
        if quiet {
            println!("{}", session.id);
        } else {
            println!("Created session: {}", session.id);
            print_field("Agent", &session.agent_id.to_string());
            print_field("Status", &session.status);
        }
    } else {
        output.print_value(&session);
    }

    Ok(())
}

async fn list(client: &Client, output: OutputFormat, agent_id: Uuid) -> Result<()> {
    let response: ListResponse<Session> = client
        .get(&format!("/v1/agents/{}/sessions", agent_id))
        .await?;

    if output.is_text() {
        if response.data.is_empty() {
            println!("No sessions found");
            return Ok(());
        }

        print_table_header(&[("ID", 36), ("TITLE", 25), ("STATUS", 10), ("CREATED", 20)]);

        for session in &response.data {
            let title = session.title.as_deref().unwrap_or("-");
            print_table_row(&[
                (&session.id.to_string(), 36),
                (title, 25),
                (&session.status, 10),
                (&session.created_at, 20),
            ]);
        }
    } else {
        output.print_value(&response);
    }

    Ok(())
}

async fn get(
    client: &Client,
    output: OutputFormat,
    agent_id: Uuid,
    session_id: Uuid,
) -> Result<()> {
    let session: Session = client
        .get(&format!("/v1/agents/{}/sessions/{}", agent_id, session_id))
        .await
        .map_err(|e| match e {
            ClientError::NotFound => anyhow::anyhow!("Session not found: {}", session_id),
            e => e.into(),
        })?;

    if output.is_text() {
        print_field("ID", &session.id.to_string());
        print_field("Agent", &session.agent_id.to_string());
        print_field("Status", &session.status);
        if let Some(title) = &session.title {
            print_field("Title", title);
        }
        if !session.tags.is_empty() {
            print_field("Tags", &session.tags.join(", "));
        }
        print_field("Created", &session.created_at);
        if let Some(started) = &session.started_at {
            print_field("Started", started);
        }
        if let Some(finished) = &session.finished_at {
            print_field("Finished", finished);
        }
    } else {
        output.print_value(&session);
    }

    Ok(())
}
