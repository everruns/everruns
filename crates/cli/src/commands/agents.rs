// Agent management commands

use crate::client::{Client, ClientError};
use crate::output::{print_field, print_table_header, print_table_row, OutputFormat};
use anyhow::{Context, Result};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

        /// Default model ID
        #[arg(long)]
        model: Option<Uuid>,

        /// Tags (repeatable)
        #[arg(long, short)]
        tag: Vec<String>,

        /// Capability IDs (repeatable)
        #[arg(long, short)]
        capability: Vec<String>,
    },

    /// List all agents
    List,

    /// Get agent by ID
    Get {
        /// Agent ID
        agent_id: Uuid,
    },

    /// Archive an agent (soft delete)
    Delete {
        /// Agent ID
        agent_id: Uuid,
    },
}

/// Agent definition from YAML/JSON file
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AgentFile {
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<Uuid>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

/// Request to create an agent
#[derive(Debug, Serialize)]
struct CreateAgentRequest {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    system_prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_model_id: Option<Uuid>,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    capabilities: Vec<String>,
}

/// Agent response from API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Agent {
    pub id: Uuid,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub system_prompt: String,
    #[serde(default)]
    pub default_model_id: Option<Uuid>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub status: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct ListResponse<T> {
    data: Vec<T>,
}

/// Parse markdown file with YAML front matter.
/// Format:
/// ```markdown
/// ---
/// name: "agent-name"
/// capabilities:
///   - current_time
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
    client: &Client,
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
            capability,
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
                capability,
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
    client: &Client,
    output: OutputFormat,
    quiet: bool,
    file: Option<String>,
    name: Option<String>,
    system_prompt: Option<String>,
    description: Option<String>,
    model: Option<Uuid>,
    tags: Vec<String>,
    capabilities: Vec<String>,
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
    let final_capabilities = if capabilities.is_empty() {
        file_config.capabilities
    } else {
        capabilities
    };

    let request = CreateAgentRequest {
        name: final_name,
        description: final_description,
        system_prompt: final_system_prompt,
        default_model_id: final_model,
        tags: final_tags,
        capabilities: final_capabilities,
    };

    let agent: Agent = client.post("/v1/agents", &request).await?;

    if output.is_text() {
        if quiet {
            println!("{}", agent.id);
        } else {
            println!("Created agent: {}", agent.id);
            print_field("Name", &agent.name);
            if !agent.capabilities.is_empty() {
                print_field("Capabilities", &agent.capabilities.join(", "));
            }
        }
    } else {
        output.print_value(&agent);
    }

    Ok(())
}

async fn list(client: &Client, output: OutputFormat) -> Result<()> {
    let response: ListResponse<Agent> = client.get("/v1/agents").await?;

    if output.is_text() {
        if response.data.is_empty() {
            println!("No agents found");
            return Ok(());
        }

        print_table_header(&[
            ("ID", 36),
            ("NAME", 20),
            ("STATUS", 8),
            ("CAPABILITIES", 30),
        ]);

        for agent in &response.data {
            let caps = if agent.capabilities.is_empty() {
                "-".to_string()
            } else {
                agent.capabilities.join(", ")
            };
            print_table_row(&[
                (&agent.id.to_string(), 36),
                (&agent.name, 20),
                (&agent.status, 8),
                (&caps, 30),
            ]);
        }
    } else {
        output.print_value(&response);
    }

    Ok(())
}

async fn get(client: &Client, output: OutputFormat, agent_id: Uuid) -> Result<()> {
    let agent: Agent = client
        .get(&format!("/v1/agents/{}", agent_id))
        .await
        .map_err(|e| match e {
            ClientError::NotFound => anyhow::anyhow!("Agent not found: {}", agent_id),
            e => e.into(),
        })?;

    if output.is_text() {
        print_field("ID", &agent.id.to_string());
        print_field("Name", &agent.name);
        print_field("Status", &agent.status);
        if let Some(desc) = &agent.description {
            print_field("Description", desc);
        }
        if !agent.capabilities.is_empty() {
            print_field("Capabilities", &agent.capabilities.join(", "));
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

async fn delete(client: &Client, output: OutputFormat, quiet: bool, agent_id: Uuid) -> Result<()> {
    client
        .delete(&format!("/v1/agents/{}", agent_id))
        .await
        .map_err(|e| match e {
            ClientError::NotFound => anyhow::anyhow!("Agent not found: {}", agent_id),
            e => e.into(),
        })?;

    if output.is_text() && !quiet {
        println!("Archived agent: {}", agent_id);
    } else if !output.is_text() {
        output.print_value(&serde_json::json!({ "id": agent_id, "status": "archived" }));
    }

    Ok(())
}
