// Agent management commands
//
// Design Decision: When --file is provided, send raw content to the server's
// import API (POST /v1/agents/import) which handles YAML/JSON/Markdown parsing.
// --initial-files-dir injects files from a local directory into the payload
// before sending, so the server receives them as initial_files.

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

        /// Directory of files to upload as read-only initial_files
        #[arg(long)]
        initial_files_dir: Option<String>,

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

        /// Directory of files to upload as read-only initial_files
        #[arg(long)]
        initial_files_dir: Option<String>,

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
#[derive(Debug, Deserialize, serde::Serialize)]
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
            initial_files_dir,
            name,
            system_prompt,
            description,
            model,
            tag,
        } => {
            if let Some(path) = file {
                if name.is_some()
                    || system_prompt.is_some()
                    || description.is_some()
                    || model.is_some()
                    || !tag.is_empty()
                {
                    eprintln!("Warning: CLI flag overrides are ignored when --file is used");
                }
                import_from_file(
                    api_url,
                    api_key,
                    &path,
                    initial_files_dir.as_deref(),
                    output,
                    quiet,
                )
                .await
            } else {
                if initial_files_dir.is_some() {
                    anyhow::bail!("--initial-files-dir requires --file");
                }
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
            initial_files_dir,
            name,
            system_prompt,
            description,
            model,
            tag,
        } => {
            if let Some(path) = file {
                if agent_id.is_some()
                    || name.is_some()
                    || system_prompt.is_some()
                    || description.is_some()
                    || model.is_some()
                    || !tag.is_empty()
                {
                    eprintln!("Warning: CLI flag overrides are ignored when --file is used");
                }
                import_from_file(
                    api_url,
                    api_key,
                    &path,
                    initial_files_dir.as_deref(),
                    output,
                    quiet,
                )
                .await
            } else {
                if initial_files_dir.is_some() {
                    anyhow::bail!("--initial-files-dir requires --file");
                }
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
/// When initial_files_dir is provided, files are globbed and injected into the
/// payload as initial_files before sending.
async fn import_from_file(
    api_url: &str,
    api_key: &str,
    path: &str,
    initial_files_dir: Option<&str>,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;

    // If --initial-files-dir is provided, parse the agent file, inject the files,
    // and send as JSON so the server receives initial_files in the payload.
    let (body, content_type) = if let Some(dir) = initial_files_dir {
        let files = glob_initial_files(dir)?;
        let mut agent: serde_json::Value =
            parse_agent_file_as_json(&content).context("Failed to parse agent file")?;
        agent["initial_files"] = serde_json::to_value(&files)?;
        (serde_json::to_string(&agent)?, "application/json")
    } else {
        (content, "text/plain")
    };

    let http = reqwest::Client::new();
    let resp = http
        .post(format!("{}/v1/agents/import", api_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", content_type)
        .body(body)
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
        let json = serde_json::json!({
            "id": agent.id,
            "name": agent.name,
            "action": verb.to_lowercase(),
        });
        output.print_value(&json);
    }

    Ok(())
}

/// Represents a file to be uploaded as an initial file for an agent.
#[derive(Debug, serde::Serialize)]
struct CollectedFile {
    path: String,
    content: String,
    encoding: String,
    is_readonly: bool,
}

/// Recursively collect all non-hidden text files from a directory.
/// Returns them as initial_files entries with paths relative to /workspace.
fn glob_initial_files(dir: &str) -> Result<Vec<CollectedFile>> {
    let base =
        std::fs::canonicalize(dir).with_context(|| format!("Cannot resolve directory: {}", dir))?;
    if !base.is_dir() {
        anyhow::bail!("Not a directory: {}", base.display());
    }

    let walker = ignore::WalkBuilder::new(&base)
        .hidden(true) // respect hidden files filter
        .git_ignore(true) // respect .gitignore
        .build();

    let mut files = Vec::new();
    for entry in walker {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let rel = path
            .strip_prefix(&base)
            .context("File outside base directory")?;

        // Skip hidden files/directories (dot-prefixed relative components).
        // The ignore crate handles this too, but belt-and-suspenders.
        if rel
            .components()
            .any(|c| c.as_os_str().to_string_lossy().starts_with('.'))
        {
            continue;
        }
        let workspace_path = format!("/workspace/{}", rel.display());

        // Read as text; skip binary files
        match std::fs::read_to_string(path) {
            Ok(content) => {
                files.push(CollectedFile {
                    path: workspace_path,
                    content,
                    encoding: "text".to_string(),
                    is_readonly: true,
                });
            }
            Err(_) => {
                eprintln!(
                    "Warning: skipping binary or unreadable file: {}",
                    path.display()
                );
            }
        }
    }

    if files.is_empty() {
        anyhow::bail!("No files found in directory: {}", dir);
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// Parse agent file content (Markdown/YAML/JSON) into a JSON Value.
/// This is minimal parsing to allow injecting initial_files before sending
/// to the server import API.
fn parse_agent_file_as_json(content: &str) -> Result<serde_json::Value> {
    let content = content.trim();

    // Markdown with front matter
    if let Some(after_prefix) = content.strip_prefix("---")
        && let Some(end) = after_prefix.find("---")
    {
        let frontmatter = after_prefix[..end].trim();
        let body = after_prefix[end + 3..].trim();

        let mut obj: serde_json::Value =
            serde_yaml::from_str(frontmatter).context("Failed to parse YAML frontmatter")?;

        // If there's a markdown body and no system_prompt in frontmatter, use it
        if !body.is_empty()
            && (obj.get("system_prompt").is_none()
                || obj["system_prompt"].as_str().is_none_or(|s| s.is_empty()))
        {
            obj["system_prompt"] = serde_json::Value::String(body.to_string());
        }

        return Ok(obj);
    }

    // JSON
    if content.starts_with('{') {
        return serde_json::from_str(content).context("Failed to parse JSON");
    }

    // YAML
    serde_yaml::from_str(content).context("Failed to parse YAML")
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

    #[test]
    fn test_parse_agent_file_json() {
        let content = r#"{"name":"test","system_prompt":"hello"}"#;
        let val = parse_agent_file_as_json(content).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["system_prompt"], "hello");
    }

    #[test]
    fn test_parse_agent_file_yaml() {
        let content = "name: test\nsystem_prompt: hello\n";
        let val = parse_agent_file_as_json(content).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["system_prompt"], "hello");
    }

    #[test]
    fn test_parse_agent_file_markdown() {
        let content = "---\nname: test\n---\nHello world";
        let val = parse_agent_file_as_json(content).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["system_prompt"], "Hello world");
    }

    #[test]
    fn test_parse_agent_file_markdown_with_system_prompt() {
        let content = "---\nname: test\nsystem_prompt: from frontmatter\n---\nBody text";
        let val = parse_agent_file_as_json(content).unwrap();
        assert_eq!(val["name"], "test");
        // Frontmatter system_prompt takes precedence
        assert_eq!(val["system_prompt"], "from frontmatter");
    }

    #[test]
    fn test_glob_initial_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/nested.txt"), "content").unwrap();

        // Hidden files should be skipped
        std::fs::write(dir.path().join(".hidden"), "secret").unwrap();

        let files = glob_initial_files(dir.path().to_str().unwrap()).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path == "/workspace/hello.txt"));
        assert!(files.iter().any(|f| f.path == "/workspace/sub/nested.txt"));
        assert!(files.iter().all(|f| f.is_readonly));
        assert!(files.iter().all(|f| f.encoding == "text"));
    }

    #[test]
    fn test_glob_initial_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = glob_initial_files(dir.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No files found"));
    }

    #[test]
    fn test_glob_initial_files_not_a_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, "content").unwrap();
        let result = glob_initial_files(file_path.to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Not a directory"));
    }
}
