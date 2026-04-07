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
use std::path::Path;

#[derive(Subcommand)]
pub enum AgentsCommand {
    /// Create a new agent (upserts if id: is present in frontmatter)
    Create {
        /// YAML/JSON/Markdown file with agent definition (sent to server for parsing)
        #[arg(short, long)]
        file: Option<String>,

        /// Directory of files to upload as initial_files (read-only by default)
        #[arg(long)]
        initial_files_dir: Option<String>,

        /// Make initial files writable (default: read-only)
        #[arg(long)]
        writable: bool,

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

        /// Directory of files to upload as initial_files (read-only by default)
        #[arg(long)]
        initial_files_dir: Option<String>,

        /// Make initial files writable (default: read-only)
        #[arg(long)]
        writable: bool,

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
            writable,
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
                    writable,
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
            writable,
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
                    writable,
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
    writable: bool,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;

    // Resolve the agent file's parent directory for expanding initial_files globs.
    let file_dir = std::fs::canonicalize(path)
        .with_context(|| format!("Cannot resolve file path: {}", path))?
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // If --initial-files-dir is provided, parse the agent file, inject the files,
    // and send as JSON so the server receives initial_files in the payload.
    // Also parse when initial_files contains glob patterns (strings) in frontmatter.
    let (body, content_type) = if let Some(dir) = initial_files_dir {
        let files = glob_initial_files(dir, writable)?;
        let mut agent: serde_json::Value =
            parse_agent_file_as_json(&content).context("Failed to parse agent file")?;
        agent["initial_files"] = serde_json::to_value(&files)?;
        (serde_json::to_string(&agent)?, "application/json")
    } else if has_initial_files_globs(&content) {
        let mut agent: serde_json::Value =
            parse_agent_file_as_json(&content).context("Failed to parse agent file")?;
        let files = expand_initial_files_globs(&agent, &file_dir, writable)?;
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
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CollectedFile {
    path: String,
    content: String,
    encoding: String,
    is_readonly: bool,
}

/// Dot-prefixed path components explicitly allowed in initial-files collection.
/// Everything else starting with `.` is skipped to prevent accidental upload of
/// secrets (`.env`, `.ssh/`, `.npmrc`, etc.).
const ALLOWED_DOT_ENTRIES: &[&str] = &[".agents"];

/// Recursively collect text files from a directory, including allowed
/// dotfile directories (e.g. `.agents/`). Returns them as initial_files
/// entries with paths relative to /workspace.
fn glob_initial_files(dir: &str, writable: bool) -> Result<Vec<CollectedFile>> {
    let base =
        std::fs::canonicalize(dir).with_context(|| format!("Cannot resolve directory: {}", dir))?;
    if !base.is_dir() {
        anyhow::bail!("Not a directory: {}", base.display());
    }

    // hidden(true) prunes most dotfiles/dirs during traversal for performance.
    // We do a second walk of ALLOWED_DOT_ENTRIES below to include them.
    let walker = ignore::WalkBuilder::new(&base)
        .hidden(true) // skip dotfiles by default (perf: avoids traversing .git/)
        .git_ignore(true) // respect .gitignore (repo-local)
        .git_global(false) // ignore global gitignore for predictable behavior
        .git_exclude(false) // ignore repo exclude files for predictable behavior
        .build();

    // Also walk allowed dot-directories that hidden(true) would skip.
    let dot_walkers: Vec<_> = ALLOWED_DOT_ENTRIES
        .iter()
        .filter_map(|name| {
            let dot_path = base.join(name);
            if dot_path.is_dir() {
                Some(
                    ignore::WalkBuilder::new(&dot_path)
                        .hidden(false) // include nested dotfiles within allowed dirs
                        .git_ignore(true)
                        .git_global(false)
                        .git_exclude(false)
                        .build(),
                )
            } else {
                None
            }
        })
        .collect();

    let mut files = Vec::new();
    let all_entries = walker.chain(dot_walkers.into_iter().flatten());
    for entry in all_entries {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        // Canonicalize each file to prevent symlink escapes outside the base dir
        let canonical = std::fs::canonicalize(path)
            .with_context(|| format!("Cannot resolve file: {}", path.display()))?;
        if !canonical.starts_with(&base) {
            eprintln!(
                "Warning: skipping symlink outside base directory: {}",
                path.display()
            );
            continue;
        }

        let rel = canonical
            .strip_prefix(&base)
            .context("File outside base directory")?;

        // Normalize path separators to POSIX-style for workspace paths
        let rel_normalized = rel.to_string_lossy().replace('\\', "/");
        let workspace_path = format!("/workspace/{}", rel_normalized);

        // Read as text; skip binary files
        match std::fs::read_to_string(path) {
            Ok(content) => {
                files.push(CollectedFile {
                    path: workspace_path,
                    content,
                    encoding: "text".to_string(),
                    is_readonly: !writable,
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

/// Quick check whether parsed initial_files contains glob patterns (strings)
/// rather than fully-specified InitialFile objects.
fn has_initial_files_globs(content: &str) -> bool {
    // Fast-path: parse just enough to detect string entries in initial_files.
    let Ok(agent) = parse_agent_file_as_json(content) else {
        return false;
    };
    let Some(arr) = agent.get("initial_files").and_then(|v| v.as_array()) else {
        return false;
    };
    arr.iter().any(|v| v.is_string())
}

/// Expand initial_files glob patterns from agent frontmatter into CollectedFile entries.
/// String entries are treated as relative paths/globs resolved against `base_dir`.
/// Object entries (already-expanded InitialFile) are passed through as-is.
fn expand_initial_files_globs(
    agent: &serde_json::Value,
    base_dir: &Path,
    writable: bool,
) -> Result<Vec<CollectedFile>> {
    let Some(arr) = agent.get("initial_files").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let mut all_files: Vec<CollectedFile> = Vec::new();

    for entry in arr {
        if let Some(pattern) = entry.as_str() {
            // String entry: treat as a path/glob relative to agent file directory.
            let resolved = base_dir.join(pattern);
            let resolved = std::fs::canonicalize(&resolved).with_context(|| {
                format!(
                    "Cannot resolve initial_files path: {} (relative to {})",
                    pattern,
                    base_dir.display()
                )
            })?;

            if resolved.is_dir() {
                // Directory: collect all files within it
                let mut files = glob_initial_files(resolved.to_str().unwrap(), writable)?;
                all_files.append(&mut files);
            } else if resolved.is_file() {
                // Single file
                collect_single_file(&resolved, base_dir, writable, &mut all_files)?;
            } else {
                anyhow::bail!(
                    "initial_files pattern resolved to non-existent path: {}",
                    resolved.display()
                );
            }
        } else if entry.is_object() {
            // Already a full InitialFile object — pass through
            let file: CollectedFile = serde_json::from_value(entry.clone())
                .context("Invalid initial_files entry object")?;
            all_files.push(file);
        } else {
            anyhow::bail!("initial_files entries must be strings (paths/globs) or objects");
        }
    }

    // Deduplicate by path (first occurrence wins)
    let mut seen = std::collections::HashSet::new();
    all_files.retain(|f| seen.insert(f.path.clone()));

    if all_files.is_empty() {
        anyhow::bail!("initial_files patterns matched no files");
    }

    all_files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(all_files)
}

/// Collect a single file into the CollectedFile list.
fn collect_single_file(
    file_path: &Path,
    base_dir: &Path,
    writable: bool,
    files: &mut Vec<CollectedFile>,
) -> Result<()> {
    let canonical = std::fs::canonicalize(file_path)
        .with_context(|| format!("Cannot resolve file: {}", file_path.display()))?;
    let base_canonical = std::fs::canonicalize(base_dir)?;

    if !canonical.starts_with(&base_canonical) {
        eprintln!(
            "Warning: skipping file outside base directory: {}",
            file_path.display()
        );
        return Ok(());
    }

    let rel = canonical.strip_prefix(&base_canonical)?;
    let rel_normalized = rel.to_string_lossy().replace('\\', "/");
    let workspace_path = format!("/workspace/{}", rel_normalized);

    match std::fs::read_to_string(file_path) {
        Ok(content) => {
            files.push(CollectedFile {
                path: workspace_path,
                content,
                encoding: "text".to_string(),
                is_readonly: !writable,
            });
        }
        Err(_) => {
            eprintln!(
                "Warning: skipping binary or unreadable file: {}",
                file_path.display()
            );
        }
    }
    Ok(())
}

/// Parse agent file content (Markdown/YAML/JSON) into a JSON Value.
/// This is minimal parsing to allow injecting initial_files before sending
/// to the server import API.
fn parse_agent_file_as_json(content: &str) -> Result<serde_json::Value> {
    let content = content.trim();

    // Markdown with front matter: require `---` delimiters on their own lines
    {
        let mut lines = content.lines();
        if let Some(first) = lines.next()
            && first.trim() == "---"
        {
            let mut frontmatter_lines = Vec::new();
            let mut found_end = false;

            for line in &mut lines {
                if line.trim() == "---" {
                    found_end = true;
                    break;
                }
                frontmatter_lines.push(line);
            }

            if found_end {
                let frontmatter = frontmatter_lines.join("\n");
                let body: String = lines.collect::<Vec<_>>().join("\n");
                let body = body.trim();

                let mut obj: serde_json::Value = serde_yaml::from_str(&frontmatter)
                    .context("Failed to parse YAML frontmatter")?;

                if !obj.is_object() {
                    anyhow::bail!(
                        "Agent file frontmatter must be a YAML mapping, not a scalar or array"
                    );
                }

                // If there's a markdown body and no system_prompt in frontmatter, use it
                if !body.is_empty()
                    && (obj.get("system_prompt").is_none()
                        || obj["system_prompt"].as_str().is_none_or(|s| s.is_empty()))
                {
                    obj["system_prompt"] = serde_json::Value::String(body.to_string());
                }

                return Ok(obj);
            }
        }
    }

    // JSON
    if content.starts_with('{') {
        return serde_json::from_str(content).context("Failed to parse JSON");
    }

    // YAML
    let val: serde_json::Value = serde_yaml::from_str(content).context("Failed to parse YAML")?;
    if !val.is_object() {
        anyhow::bail!("Agent file must be a YAML/JSON object");
    }
    Ok(val)
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

        // Hidden files/dirs should be skipped (security: prevents .env, .ssh leaks)
        std::fs::write(dir.path().join(".env"), "SECRET=key").unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        std::fs::write(dir.path().join(".git/config"), "gitdata").unwrap();

        let files = glob_initial_files(dir.path().to_str().unwrap(), false).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path == "/workspace/hello.txt"));
        assert!(files.iter().any(|f| f.path == "/workspace/sub/nested.txt"));
        assert!(files.iter().all(|f| f.is_readonly));
        assert!(files.iter().all(|f| f.encoding == "text"));
    }

    #[test]
    fn test_glob_initial_files_writable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();

        let files = glob_initial_files(dir.path().to_str().unwrap(), true).unwrap();
        assert_eq!(files.len(), 1);
        assert!(files.iter().all(|f| !f.is_readonly));
    }

    #[test]
    fn test_glob_initial_files_includes_dot_agents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.md"), "# Agent").unwrap();
        std::fs::create_dir_all(dir.path().join(".agents/skills/search")).unwrap();
        std::fs::write(
            dir.path().join(".agents/skills/search/SKILL.md"),
            "# Search skill",
        )
        .unwrap();

        let files = glob_initial_files(dir.path().to_str().unwrap(), false).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path == "/workspace/agent.md"));
        assert!(
            files
                .iter()
                .any(|f| f.path == "/workspace/.agents/skills/search/SKILL.md")
        );
    }

    #[test]
    fn test_glob_initial_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = glob_initial_files(dir.path().to_str().unwrap(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No files found"));
    }

    #[test]
    fn test_glob_initial_files_not_a_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, "content").unwrap();
        let result = glob_initial_files(file_path.to_str().unwrap(), false);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Not a directory"));
    }

    #[test]
    fn test_glob_initial_files_skips_symlinks_outside_base() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.txt"), "secret").unwrap();
        // Create a symlink pointing outside the base directory
        std::os::unix::fs::symlink(
            outside.path().join("secret.txt"),
            dir.path().join("link.txt"),
        )
        .unwrap();
        // Should error because only file is the symlink (skipped)
        let result = glob_initial_files(dir.path().to_str().unwrap(), false);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_agent_file_non_object_errors() {
        let content = "just a string";
        let result = parse_agent_file_as_json(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_has_initial_files_globs_with_strings() {
        let content = "---\nname: test\ninitial_files:\n  - .\n  - .agents/*\n---\nPrompt";
        assert!(has_initial_files_globs(content));
    }

    #[test]
    fn test_has_initial_files_globs_without_strings() {
        // No initial_files at all
        let content = "---\nname: test\n---\nPrompt";
        assert!(!has_initial_files_globs(content));

        // initial_files with objects (already expanded)
        let content = r#"{"name":"test","initial_files":[{"path":"/workspace/f.txt","content":"x","encoding":"text","is_readonly":false}]}"#;
        assert!(!has_initial_files_globs(content));
    }

    #[test]
    fn test_expand_initial_files_globs_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/nested.txt"), "content").unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": ["."]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path == "/workspace/hello.txt"));
        assert!(files.iter().any(|f| f.path == "/workspace/sub/nested.txt"));
        assert!(files.iter().all(|f| f.is_readonly));
    }

    #[test]
    fn test_expand_initial_files_globs_subdirectory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("root.txt"), "root").unwrap();
        std::fs::create_dir_all(dir.path().join(".agents/skills")).unwrap();
        std::fs::write(dir.path().join(".agents/skills/SKILL.md"), "# Skill").unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".agents"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        // Should only include .agents/ files, not root.txt
        assert_eq!(files.len(), 1);
        assert!(files.iter().any(|f| f.path.contains("SKILL.md")));
    }

    #[test]
    fn test_expand_initial_files_globs_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/readme.md"), "# Hi").unwrap();

        // "." collects everything; "sub" also collects sub/ files.
        // glob_initial_files uses the given dir as base for /workspace/ paths,
        // so "." yields /workspace/sub/readme.md and "sub" yields /workspace/readme.md.
        // Only truly duplicate /workspace/ paths are deduped.
        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".", "."]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        // Each path should appear exactly once despite "." being listed twice
        let hello_count = files
            .iter()
            .filter(|f| f.path.contains("hello.txt"))
            .count();
        assert_eq!(hello_count, 1);
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn test_expand_initial_files_globs_single_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("README.md"), "# Readme").unwrap();
        std::fs::write(dir.path().join("other.txt"), "other").unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": ["README.md"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/workspace/README.md");
        assert_eq!(files[0].content, "# Readme");
    }

    #[test]
    fn test_expand_initial_files_globs_writable() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": ["."]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), true).unwrap();
        assert!(files.iter().all(|f| !f.is_readonly));
    }

    #[test]
    fn test_expand_initial_files_globs_nonexistent_errors() {
        let dir = tempfile::tempdir().unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": ["nonexistent"]
        });

        let result = expand_initial_files_globs(&agent, dir.path(), false);
        assert!(result.is_err());
    }
}
