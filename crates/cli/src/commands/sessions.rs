// Session management commands

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::Result;
use clap::Subcommand;
use everruns_sdk::Everruns;

#[derive(Subcommand)]
pub enum SessionsCommand {
    /// Create a new session
    Create {
        /// Harness ID (e.g. harness_xxx)
        #[arg(long, short = 'H')]
        harness: String,

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
}

pub async fn run(
    command: SessionsCommand,
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        SessionsCommand::Create {
            harness,
            agent,
            title,
            model,
        } => {
            create(
                api_url, api_key, output, quiet, harness, agent, title, model,
            )
            .await
        }
        SessionsCommand::List => list(client, output).await,
        SessionsCommand::Get { session } => get(client, output, session).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn create(
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
    harness_id: String,
    agent_id: Option<String>,
    title: Option<String>,
    model_id: Option<String>,
) -> Result<()> {
    // SDK doesn't support harness_id yet, use reqwest directly
    let mut body = serde_json::json!({
        "harness_id": harness_id,
    });
    if let Some(a) = agent_id {
        body["agent_id"] = serde_json::Value::String(a);
    }
    if let Some(t) = title {
        body["title"] = serde_json::Value::String(t);
    }
    if let Some(m) = model_id {
        body["model_id"] = serde_json::Value::String(m);
    }

    let http = reqwest::Client::new();
    let url = format!("{}/v1/sessions", api_url.trim_end_matches('/'));
    let resp = http
        .post(&url)
        .header("Authorization", api_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        anyhow::bail!("Failed to create session: {} {}", status, text);
    }

    let session: serde_json::Value = resp.json().await?;

    if output.is_text() {
        let id = session["id"].as_str().unwrap_or("unknown");
        if quiet {
            println!("{}", id);
        } else {
            println!("Created session: {}", id);
            if let Some(agent) = session["agent_id"].as_str() {
                print_field("Agent", agent);
            }
            let status = session["status"].as_str().unwrap_or("unknown");
            print_field("Status", status);
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
