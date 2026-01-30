// Agent management commands

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use everruns_sdk::{CreateAgentRequest, Everruns};
use serde::{Deserialize, Serialize};

#[derive(Subcommand)]
pub enum AgentsCommand {
    /// Create a new agent
    Create {
        /// YAML/JSON/Markdown file with agent definition
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

/// Agent definition from YAML/JSON file
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentFile {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

/// Parse markdown file with YAML front matter.
/// Format:
/// ```markdown
/// ---
/// name: "agent-name"
/// ---
/// System prompt goes here as the body.
/// ```
fn parse_markdown_frontmatter(content: &str) -> Result<AgentFile> {
    // Check for front matter delimiter
    if !content.starts_with("---") {
        anyhow::bail!("Markdown file must start with YAML front matter (---)");
    }

    // Find the closing delimiter
    let rest = &content[3..];
    let end_pos = rest
        .find("\n---")
        .context("Missing closing front matter delimiter (---)")?;

    let front_matter = &rest[..end_pos].trim();
    let body = rest[end_pos + 4..].trim(); // Skip "\n---"

    // Parse front matter as YAML
    let mut config: AgentFile =
        serde_yaml::from_str(front_matter).context("Failed to parse front matter as YAML")?;

    // Body becomes system_prompt if not empty
    if !body.is_empty() {
        config.system_prompt = Some(body.to_string());
    }

    Ok(config)
}

pub async fn run(
    command: AgentsCommand,
    client: &Everruns,
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
            create(
                client,
                output,
                quiet,
                file,
                name,
                system_prompt,
                description,
                model,
                tag,
            )
            .await
        }
        AgentsCommand::List => list(client, output).await,
        AgentsCommand::Get { agent_id } => get(client, output, agent_id).await,
        AgentsCommand::Delete { agent_id } => delete(client, output, quiet, agent_id).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn create(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    file: Option<String>,
    name: Option<String>,
    system_prompt: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tags: Vec<String>,
) -> Result<()> {
    // Load from file if provided
    let file_config = if let Some(path) = file {
        let content = std::fs::read_to_string(&path)
            .with_context(|| format!("Failed to read file: {}", path))?;

        // Detect format by extension
        let config: AgentFile = if path.ends_with(".md") {
            // Markdown with YAML front matter
            parse_markdown_frontmatter(&content)
                .with_context(|| format!("Failed to parse markdown: {}", path))?
        } else if path.ends_with(".yaml") || path.ends_with(".yml") {
            serde_yaml::from_str(&content)
                .with_context(|| format!("Failed to parse YAML: {}", path))?
        } else if path.ends_with(".json") {
            serde_json::from_str(&content)
                .with_context(|| format!("Failed to parse JSON: {}", path))?
        } else {
            // Try markdown first (if starts with ---), then YAML, then JSON
            if content.starts_with("---") {
                parse_markdown_frontmatter(&content)
                    .or_else(|_| serde_yaml::from_str(&content))
                    .or_else(|_| serde_json::from_str(&content))
                    .with_context(|| {
                        format!(
                            "Failed to parse file (tried markdown, YAML, JSON): {}",
                            path
                        )
                    })?
            } else {
                serde_yaml::from_str(&content)
                    .or_else(|_| serde_json::from_str(&content))
                    .with_context(|| {
                        format!("Failed to parse file (tried YAML and JSON): {}", path)
                    })?
            }
        };
        config
    } else {
        AgentFile::default()
    };

    // CLI args override file values
    let final_name = name
        .or(file_config.name)
        .context("--name is required (or provide in file)")?;
    let final_system_prompt = system_prompt
        .or(file_config.system_prompt)
        .context("--system-prompt is required (or provide in file)")?;
    let final_description = description.or(file_config.description);
    let final_model = model.or(file_config.default_model_id);
    let final_tags = if tags.is_empty() {
        file_config.tags
    } else {
        tags
    };

    // Build the request
    let mut req = CreateAgentRequest::new(&final_name, &final_system_prompt);
    if let Some(desc) = final_description {
        req = req.description(desc);
    }
    if let Some(model_id) = final_model {
        req = req.default_model_id(model_id);
    }
    if !final_tags.is_empty() {
        req = req.tags(final_tags);
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
        // Convert to JSON for non-text output
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

async fn delete(client: &Everruns, output: OutputFormat, quiet: bool, agent_id: String) -> Result<()> {
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
