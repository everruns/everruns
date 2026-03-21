// Agent management commands
//
// Design Decision: When --file is provided, send raw content to the server's
// import API (POST /v1/agents/import) which handles YAML/JSON/Markdown parsing.
// This avoids duplicating parsing logic and removes the serde_yaml dependency.
// CLI flag overrides (--name, --description, etc.) are not supported with --file.

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use everruns_sdk::{CreateAgentRequest, Everruns};
use serde::Deserialize;

#[derive(Subcommand)]
pub enum AgentsCommand {
    /// Create a new agent (upserts if id: is present in frontmatter)
    Create {
        /// YAML/JSON/Markdown file with agent definition (sent to server for parsing)
        #[arg(short, long)]
        file: Option<String>,

        /// Agent name (required if no --file)
        #[arg(long)]
        name: Option<String>,

        /// System prompt (required if no --file)
        #[arg(long)]
        system_prompt: Option<String>,

        /// Agent description
        #[arg(long)]
        description: Option<String>,

        /// Default model ID (e.g. mod_xxx)
        #[arg(long)]
        model: Option<String>,

        /// Tags (repeatable)
        #[arg(long, short)]
        tag: Vec<String>,
    },

    /// Update an existing agent from a file definition
    Update {
        /// Agent ID (e.g. agent_xxx). If omitted, uses id from file frontmatter.
        agent_id: Option<String>,

        /// YAML/JSON/Markdown file with agent definition (sent to server for parsing)
        #[arg(short, long)]
        file: Option<String>,

        /// Agent name
        #[arg(long)]
        name: Option<String>,

        /// System prompt
        #[arg(long)]
        system_prompt: Option<String>,

        /// Agent description
        #[arg(long)]
        description: Option<String>,

        /// Default model ID (e.g. mod_xxx)
        #[arg(long)]
        model: Option<String>,

        /// Tags (repeatable)
        #[arg(long, short)]
        tag: Vec<String>,
    },

    /// List all agents
    List,

    /// Get agent by ID
    Get {
        /// Agent ID (e.g. agt_xxx)
        agent_id: String,
    },

    /// Archive an agent (soft delete)
    Delete {
        /// Agent ID (e.g. agt_xxx)
        agent_id: String,
    },
}

/// Response from the import API
#[derive(Debug, Deserialize)]
struct ImportedAgent {
    id: String,
    name: String,
}

pub async fn run(
    command: AgentsCommand,
    client: &Everruns,
    api_url: &str,
    api_key: &str,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        AgentsCommand::Create {
            file,
            name,
            system_prompt,
            description,
            model,
            tag,
        } => {
            if let Some(path) = file {
                import_from_file(api_url, api_key, &path, output, quiet).await
            } else {
                create_from_flags(
                    client,
                    output,
                    quiet,
                    name,
                    system_prompt,
                    description,
                    model,
                    tag,
                )
                .await
            }
        }
        AgentsCommand::Update {
            agent_id,
            file,
            name,
            system_prompt,
            description,
            model,
            tag,
        } => {
            if let Some(path) = file {
                if agent_id.is_some() || name.is_some() || system_prompt.is_some() {
                    eprintln!("Warning: CLI flag overrides are ignored when --file is used");
                }
                import_from_file(api_url, api_key, &path, output, quiet).await
            } else {
                // Update without file requires agent_id
                let id = agent_id.context("Agent ID is required for update without --file")?;
                update_from_flags(
                    client,
                    output,
                    quiet,
                    &id,
                    name,
                    system_prompt,
                    description,
                    model,
                    tag,
                )
                .await
            }
        }
        AgentsCommand::List => list(client, output).await,
        AgentsCommand::Get { agent_id } => get(client, output, agent_id).await,
        AgentsCommand::Delete { agent_id } => delete(client, output, quiet, agent_id).await,
    }
}

/// Import agent from file via server import API.
/// Server handles YAML/JSON/Markdown parsing.
async fn import_from_file(
    api_url: &str,
    api_key: &str,
    path: &str,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/agents/import", api_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", "text/plain")
        .body(content)
        .send()
        .await
        .context("Failed to send import request")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("Import failed ({}): {}", status, body);
    }

    let was_created = status == reqwest::StatusCode::CREATED;
    let agent: ImportedAgent = resp
        .json()
        .await
        .context("Failed to parse import response")?;

    let verb = if was_created { "Created" } else { "Applied" };

    if output.is_text() {
        if quiet {
            println!("{}", agent.id);
        } else {
            println!("{} agent: {}", verb, agent.id);
            print_field("Name", &agent.name);
        }
    } else {
        // Re-fetch full agent for JSON output
        println!(
            "{{\"id\":\"{}\",\"name\":\"{}\",\"action\":\"{}\"}}",
            agent.id,
            agent.name,
            verb.to_lowercase()
        );
    }

    Ok(())
}

/// Create agent from CLI flags using SDK
#[allow(clippy::too_many_arguments)]
async fn create_from_flags(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    name: Option<String>,
    system_prompt: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tags: Vec<String>,
) -> Result<()> {
    let name = name.context("--name is required")?;
    let system_prompt = system_prompt.context("--system-prompt is required")?;

    let mut req = CreateAgentRequest::new(&name, &system_prompt);
    if let Some(desc) = description {
        req = req.description(desc);
    }
    if let Some(model_id) = model {
        req = req.default_model_id(model_id);
    }
    if !tags.is_empty() {
        req = req.tags(tags);
    }

    let agent = client.agents().create_with_options(req).await?;

    if output.is_text() {
        if quiet {
            println!("{}", agent.id);
        } else {
            println!("Created agent: {}", agent.id);
            print_field("Name", &agent.name);
        }
    } else {
        output.print_value(&agent);
    }

    Ok(())
}

/// Update agent from CLI flags using SDK
#[allow(clippy::too_many_arguments)]
async fn update_from_flags(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    agent_id: &str,
    name: Option<String>,
    system_prompt: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tags: Vec<String>,
) -> Result<()> {
    let name = name.context("--name is required for update without --file")?;
    let system_prompt =
        system_prompt.context("--system-prompt is required for update without --file")?;

    let mut req = CreateAgentRequest::new(&name, &system_prompt);
    if let Some(desc) = description {
        req = req.description(desc);
    }
    if let Some(model_id) = model {
        req = req.default_model_id(model_id);
    }
    if !tags.is_empty() {
        req = req.tags(tags);
    }

    let agent = client.agents().apply_with_options(agent_id, req).await?;

    if output.is_text() {
        if quiet {
            println!("{}", agent.id);
        } else {
            println!("Applied agent: {}", agent.id);
            print_field("Name", &agent.name);
        }
    } else {
        output.print_value(&agent);
    }

    Ok(())
}

async fn list(client: &Everruns, output: OutputFormat) -> Result<()> {
    let response = client.agents().list().await?;

    if output.is_text() {
        if response.data.is_empty() {
            println!("No agents found");
            return Ok(());
        }

        print_table_header(&[("ID", 36), ("NAME", 20), ("STATUS", 8)]);

        for agent in &response.data {
            let status = format!("{:?}", agent.status).to_lowercase();
            print_table_row(&[(&agent.id, 36), (&agent.name, 20), (&status, 8)]);
        }
    } else {
        let data: Vec<serde_json::Value> = response
            .data
            .iter()
            .map(|a| {
                serde_json::json!({
                    "id": a.id,
                    "name": a.name,
                    "description": a.description,
                    "system_prompt": a.system_prompt,
                    "default_model_id": a.default_model_id,
                    "tags": a.tags,
                    "status": format!("{:?}", a.status).to_lowercase(),
                    "created_at": a.created_at,
                    "updated_at": a.updated_at,
                })
            })
            .collect();
        output.print_value(&serde_json::json!({ "data": data }));
    }

    Ok(())
}

async fn get(client: &Everruns, output: OutputFormat, agent_id: String) -> Result<()> {
    let agent = client
        .agents()
        .get(&agent_id)
        .await
        .map_err(|e| anyhow::anyhow!("Agent not found: {} ({})", agent_id, e))?;

    if output.is_text() {
        print_field("ID", &agent.id);
        print_field("Name", &agent.name);
        print_field("Status", &format!("{:?}", agent.status).to_lowercase());
        if let Some(desc) = &agent.description {
            print_field("Description", desc);
        }
        if !agent.tags.is_empty() {
            print_field("Tags", &agent.tags.join(", "));
        }
        print_field("Created", &agent.created_at);
    } else {
        output.print_value(&agent);
    }

    Ok(())
}

async fn delete(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    agent_id: String,
) -> Result<()> {
    client
        .agents()
        .delete(&agent_id)
        .await
        .map_err(|e| anyhow::anyhow!("Agent not found: {} ({})", agent_id, e))?;

    if output.is_text() && !quiet {
        println!("Archived agent: {}", agent_id);
    } else if !output.is_text() {
        output.print_value(&serde_json::json!({ "id": agent_id, "status": "archived" }));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_imported_agent_deserialize() {
        let json = r#"{"id":"agent_abc","name":"test","description":null,"system_prompt":"hello","status":"active"}"#;
        let agent: ImportedAgent = serde_json::from_str(json).unwrap();
        assert_eq!(agent.id, "agent_abc");
        assert_eq!(agent.name, "test");
    }
}
