// Agent management commands

use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use everruns_sdk::{AgentCapabilityConfig, CreateAgentRequest, Everruns, InitialFile};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Subcommand)]
pub enum AgentsCommand {
    /// Create a new agent (upserts if id: is present in frontmatter)
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

        /// Directory to glob for initial_files (uploaded as session starter files)
        #[arg(long)]
        initial_files_dir: Option<String>,
    },

    /// Update an existing agent from a file definition
    Update {
        /// Agent ID (e.g. agent_xxx). If omitted, uses id from file frontmatter.
        agent_id: Option<String>,

        /// YAML/JSON/Markdown file with agent definition
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

        /// Directory to glob for initial_files (uploaded as session starter files)
        #[arg(long)]
        initial_files_dir: Option<String>,
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
    /// Agent ID for upsert. When present, uses apply (create-or-update).
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub system_prompt: Option<String>,
    pub default_model_id: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Capabilities - supports both string IDs and objects with ref/config
    #[serde(default)]
    pub capabilities: Vec<AgentFileCapability>,
    #[serde(default)]
    pub initial_files: Vec<InitialFile>,
}

/// Capability entry in agent file - supports both string and object formats
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AgentFileCapability {
    /// Legacy format: just capability ID string
    Simple(String),
    /// New format: object with ref and config
    WithConfig {
        #[serde(rename = "ref")]
        capability_ref: String,
        #[serde(default)]
        config: serde_json::Value,
    },
}

impl AgentFileCapability {
    fn to_sdk_config(&self) -> AgentCapabilityConfig {
        match self {
            AgentFileCapability::Simple(id) => AgentCapabilityConfig::new(id.clone()),
            AgentFileCapability::WithConfig {
                capability_ref,
                config,
            } => AgentCapabilityConfig::new(capability_ref.clone()).config(config.clone()),
        }
    }
}

/// Glob a directory and collect all files as InitialFile entries.
/// Walks the directory recursively; file paths are stored relative to `dir`.
fn glob_initial_files(dir: &str) -> Result<Vec<InitialFile>> {
    let base = Path::new(dir)
        .canonicalize()
        .with_context(|| format!("Cannot resolve initial-files-dir: {}", dir))?;

    let mut files = Vec::new();
    collect_files(&base, &base, &mut files)?;
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

fn collect_files(root: &Path, current: &Path, out: &mut Vec<InitialFile>) -> Result<()> {
    for entry in std::fs::read_dir(current)
        .with_context(|| format!("Failed to read directory: {}", current.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories
            if path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with('.'))
                .unwrap_or(false)
            {
                continue;
            }
            collect_files(root, &path, out)?;
        } else {
            let rel = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            // Skip hidden files and binary-looking extensions
            if rel.starts_with('.') {
                continue;
            }
            let content = std::fs::read_to_string(&path)
                .with_context(|| format!("Failed to read file: {}", path.display()))?;
            let file_path = format!("/{}", rel);
            out.push(InitialFile::new(file_path, content).is_readonly(true));
        }
    }
    Ok(())
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
            initial_files_dir,
        } => {
            create_or_update(
                client,
                output,
                quiet,
                file,
                name,
                system_prompt,
                description,
                model,
                tag,
                initial_files_dir,
                None, // no explicit agent_id override
            )
            .await
        }
        AgentsCommand::Update {
            agent_id,
            file,
            name,
            system_prompt,
            description,
            model,
            tag,
            initial_files_dir,
        } => {
            create_or_update(
                client,
                output,
                quiet,
                file,
                name,
                system_prompt,
                description,
                model,
                tag,
                initial_files_dir,
                agent_id,
            )
            .await
        }
        AgentsCommand::List => list(client, output).await,
        AgentsCommand::Get { agent_id } => get(client, output, agent_id).await,
        AgentsCommand::Delete { agent_id } => delete(client, output, quiet, agent_id).await,
    }
}

#[allow(clippy::too_many_arguments)]
async fn create_or_update(
    client: &Everruns,
    output: OutputFormat,
    quiet: bool,
    file: Option<String>,
    name: Option<String>,
    system_prompt: Option<String>,
    description: Option<String>,
    model: Option<String>,
    tags: Vec<String>,
    initial_files_dir: Option<String>,
    agent_id_override: Option<String>,
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

    // Resolve agent ID: CLI arg > file frontmatter > none (pure create)
    let resolved_id = agent_id_override.or(file_config.id);

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

    // Resolve capabilities from file
    let final_capabilities: Vec<AgentCapabilityConfig> = file_config
        .capabilities
        .iter()
        .map(|c| c.to_sdk_config())
        .collect();

    // Resolve initial_files: --initial-files-dir overrides file-defined initial_files
    let final_initial_files = if let Some(dir) = initial_files_dir {
        glob_initial_files(&dir)?
    } else {
        file_config.initial_files
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
    if !final_capabilities.is_empty() {
        req = req.capabilities(final_capabilities);
    }
    if !final_initial_files.is_empty() {
        req = req.initial_files(final_initial_files);
    }

    let agent = if let Some(id) = &resolved_id {
        client.agents().apply_with_options(id, req).await?
    } else {
        client.agents().create_with_options(req).await?
    };

    let verb = if resolved_id.is_some() {
        "Applied"
    } else {
        "Created"
    };

    if output.is_text() {
        if quiet {
            println!("{}", agent.id);
        } else {
            println!("{} agent: {}", verb, agent.id);
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
    fn test_parse_markdown_frontmatter_basic() {
        let content = r#"---
name: "test-agent"
description: "A test agent"
---
You are a helpful assistant."#;

        let result = parse_markdown_frontmatter(content).unwrap();
        assert_eq!(result.name, Some("test-agent".to_string()));
        assert_eq!(result.description, Some("A test agent".to_string()));
        assert_eq!(
            result.system_prompt,
            Some("You are a helpful assistant.".to_string())
        );
    }

    #[test]
    fn test_parse_markdown_frontmatter_with_tags() {
        let content = r#"---
name: "agent"
tags:
  - coding
  - rust
---
Help with code."#;

        let result = parse_markdown_frontmatter(content).unwrap();
        assert_eq!(result.name, Some("agent".to_string()));
        assert_eq!(result.tags, vec!["coding", "rust"]);
        assert_eq!(result.system_prompt, Some("Help with code.".to_string()));
    }

    #[test]
    fn test_parse_markdown_frontmatter_empty_body() {
        let content = r#"---
name: "agent"
system_prompt: "From front matter"
---
"#;

        let result = parse_markdown_frontmatter(content).unwrap();
        assert_eq!(result.name, Some("agent".to_string()));
        // Empty body doesn't override front matter system_prompt
        assert_eq!(result.system_prompt, Some("From front matter".to_string()));
    }

    #[test]
    fn test_parse_markdown_frontmatter_no_start_delimiter() {
        let content = "name: test\n---\nBody";
        let result = parse_markdown_frontmatter(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_markdown_frontmatter_no_end_delimiter() {
        let content = "---\nname: test\nNo end delimiter";
        let result = parse_markdown_frontmatter(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_agent_file_yaml_parse() {
        let yaml = r#"
name: "yaml-agent"
description: "Parsed from YAML"
default_model_id: "mod_123"
tags:
  - helper
"#;
        let result: AgentFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(result.name, Some("yaml-agent".to_string()));
        assert_eq!(result.description, Some("Parsed from YAML".to_string()));
        assert_eq!(result.default_model_id, Some("mod_123".to_string()));
        assert_eq!(result.tags, vec!["helper"]);
    }

    #[test]
    fn test_agent_file_json_parse() {
        let json = r#"{
            "name": "json-agent",
            "system_prompt": "You help.",
            "tags": ["fast", "smart"]
        }"#;
        let result: AgentFile = serde_json::from_str(json).unwrap();
        assert_eq!(result.name, Some("json-agent".to_string()));
        assert_eq!(result.system_prompt, Some("You help.".to_string()));
        assert_eq!(result.tags, vec!["fast", "smart"]);
    }

    #[test]
    fn test_agent_file_defaults() {
        let yaml = "name: minimal";
        let result: AgentFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(result.name, Some("minimal".to_string()));
        assert_eq!(result.id, None);
        assert_eq!(result.description, None);
        assert_eq!(result.system_prompt, None);
        assert_eq!(result.default_model_id, None);
        assert!(result.tags.is_empty());
        assert!(result.capabilities.is_empty());
        assert!(result.initial_files.is_empty());
    }

    #[test]
    fn test_agent_file_with_id() {
        let yaml = r#"
id: "agent_abc123"
name: "upsert-agent"
system_prompt: "You help."
"#;
        let result: AgentFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(result.id, Some("agent_abc123".to_string()));
        assert_eq!(result.name, Some("upsert-agent".to_string()));
    }

    #[test]
    fn test_parse_markdown_frontmatter_with_id() {
        let content = r#"---
id: "agent_abc123"
name: "test-agent"
---
You are a helpful assistant."#;

        let result = parse_markdown_frontmatter(content).unwrap();
        assert_eq!(result.id, Some("agent_abc123".to_string()));
        assert_eq!(result.name, Some("test-agent".to_string()));
        assert_eq!(
            result.system_prompt,
            Some("You are a helpful assistant.".to_string())
        );
    }

    #[test]
    fn test_agent_file_capabilities_simple() {
        let yaml = r#"
name: "cap-agent"
capabilities:
  - current_time
  - web_fetch
"#;
        let result: AgentFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(result.capabilities.len(), 2);
        let sdk_caps: Vec<_> = result
            .capabilities
            .iter()
            .map(|c| c.to_sdk_config())
            .collect();
        assert_eq!(sdk_caps[0].capability_ref, "current_time");
        assert_eq!(sdk_caps[1].capability_ref, "web_fetch");
    }

    #[test]
    fn test_agent_file_capabilities_with_config() {
        let yaml = r#"
name: "cap-agent"
capabilities:
  - ref: current_time
    config: {}
  - ref: web_fetch
    config:
      max_size: 1024
"#;
        let result: AgentFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(result.capabilities.len(), 2);
        let sdk_caps: Vec<_> = result
            .capabilities
            .iter()
            .map(|c| c.to_sdk_config())
            .collect();
        assert_eq!(sdk_caps[0].capability_ref, "current_time");
        assert_eq!(sdk_caps[1].capability_ref, "web_fetch");
        assert!(sdk_caps[1].config.is_some());
    }

    #[test]
    fn test_agent_file_capabilities_mixed() {
        let yaml = r#"
name: "cap-agent"
capabilities:
  - current_time
  - ref: web_fetch
    config: {}
"#;
        let result: AgentFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(result.capabilities.len(), 2);
    }

    #[test]
    fn test_agent_file_initial_files_inline() {
        let yaml = r##"
name: "files-agent"
initial_files:
  - path: "/README.md"
    content: "# Hello"
  - path: "/data.txt"
    content: "some data"
    is_readonly: true
"##;
        let result: AgentFile = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(result.initial_files.len(), 2);
        assert_eq!(result.initial_files[0].path, "/README.md");
        assert_eq!(result.initial_files[0].content, "# Hello");
    }

    #[test]
    fn test_parse_markdown_frontmatter_with_capabilities() {
        let content = r#"---
name: "cap-agent"
capabilities:
  - current_time
  - ref: web_fetch
    config: {}
---
You are helpful."#;

        let result = parse_markdown_frontmatter(content).unwrap();
        assert_eq!(result.capabilities.len(), 2);
    }

    #[test]
    fn test_glob_initial_files() {
        let tmp = std::env::temp_dir().join("everruns_test_glob");
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("sub")).unwrap();
        std::fs::write(tmp.join("README.md"), "# Hello").unwrap();
        std::fs::write(tmp.join("sub/data.txt"), "data").unwrap();
        std::fs::write(tmp.join(".hidden"), "skip").unwrap();

        let files = glob_initial_files(tmp.to_str().unwrap()).unwrap();
        assert_eq!(files.len(), 2);
        // Sorted by path
        assert_eq!(files[0].path, "/README.md");
        assert_eq!(files[0].content, "# Hello");
        assert_eq!(files[1].path, "/sub/data.txt");
        assert_eq!(files[1].content, "data");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
