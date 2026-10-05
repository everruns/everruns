// Agent management commands
//
// The shared package codec materializes portable file/folder/ZIP definitions.
// The legacy upload path below retains explicit hidden-path opt-ins and
// --initial-files-dir for existing CLI users.
//
// Trust boundary (initial_files hidden-path policy): the CLI walks the user's
// local filesystem to assemble `initial_files` and uploads the bytes to the
// server. Hidden (dot-prefixed) path components are gated to prevent accidental
// exfiltration of credentials and host secrets (e.g. `.ssh/`, `.env`,
// `.aws/credentials`) while still letting agents ship their packaging assets
// (e.g. `.github/`, `.vscode/`, `.claude/`, `.mcp.json`). The policy is layered:
//   1. `DENIED_DOT_ENTRIES` is a hard-deny floor — never uploaded, even if a
//      user opts in via the manifest. Covers known credential locations.
//   2. `ALLOWED_DOT_ENTRIES` is the built-in safe default — common dev assets
//      that round-trip cleanly between CLI users and the server.
//   3. The agent manifest may declare `initial_files_allow_hidden: [".foo"]`
//      to extend the allowlist for project-specific tools. Entries in the
//      hard-deny floor are still rejected.
// See `knowledge/foundations/cli.md` (Initial Files Hidden Path Policy) and threat
// `TM-FS-009` in `knowledge/security/threat-model.md`.

use super::sessions::is_prefixed_id;
use crate::output::{OutputFormat, print_field, print_table_header, print_table_row};
use anyhow::{Context, Result};
use clap::Subcommand;
use everruns_core::agent_package::{ALLOWED_DOT_ENTRIES, DENIED_DOT_ENTRIES};
use everruns_sdk::{CreateAgentRequest, Everruns};
use serde::Deserialize;
use std::path::Path;

const DEFAULT_AGENT_FILE_NAME: &str = "agent.toml";

mod packages;
use packages::print_package_report;
pub use packages::run_local;

#[derive(Subcommand)]
pub enum AgentsCommand {
    /// Import a portable agent file, folder or ZIP
    Import {
        #[arg(value_name = "PATH", conflicts_with = "content")]
        file: Option<String>,
        /// Inline definition (also supported by Platform Chat and MCP)
        #[arg(long, conflicts_with = "file")]
        content: Option<String>,
        /// Explicit existing agent name to update
        #[arg(long)]
        target: Option<String>,
        #[arg(long)]
        format: Option<String>,
    },
    /// Export an agent by name without IDs or credentials
    Export {
        agent: String,
        #[arg(long, default_value = "markdown", value_parser = ["markdown", "toml", "yaml", "json", "zip", "folder"])]
        format: String,
        #[arg(long)]
        out: Option<std::path::PathBuf>,
    },
    /// Validate a package locally, or also check destination dependencies
    Validate {
        file: String,
        #[arg(long)]
        remote: bool,
    },
    /// Generate a semantic diff against a local package or a remote agent
    Diff {
        file: String,
        #[arg(long, conflicts_with = "target")]
        against: Option<String>,
        #[arg(long, conflicts_with = "against")]
        target: Option<String>,
    },
    /// Create a new agent (upserts if id: is present in frontmatter)
    Create {
        /// TOML/YAML/JSON/Markdown file with agent definition
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
        #[arg(long = "instructions", alias = "system-prompt")]
        system_prompt: Option<String>,

        /// Agent description
        #[arg(long)]
        description: Option<String>,

        /// Default model ID (e.g. mod_xxx)
        #[arg(long)]
        model: Option<String>,

        /// Harness ID or name (e.g. harness_xxx or "generic").
        /// Omit to default to the org's generic harness.
        #[arg(long, short = 'H')]
        harness: Option<String>,

        /// Tags (repeatable)
        #[arg(long, short)]
        tag: Vec<String>,
    },

    /// Update an existing agent from a file definition
    Update {
        /// Agent ID (e.g. agent_xxx). If omitted, uses id from file frontmatter.
        agent_id: Option<String>,

        /// TOML/YAML/JSON/Markdown file with agent definition
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
        #[arg(long = "instructions", alias = "system-prompt")]
        system_prompt: Option<String>,

        /// Agent description
        #[arg(long)]
        description: Option<String>,

        /// Default model ID (e.g. mod_xxx)
        #[arg(long)]
        model: Option<String>,

        /// Harness ID or name (e.g. harness_xxx or "generic")
        #[arg(long, short = 'H')]
        harness: Option<String>,

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
    org_id: Option<&str>,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    match command {
        command @ (AgentsCommand::Import { .. }
        | AgentsCommand::Export { .. }
        | AgentsCommand::Validate { .. }
        | AgentsCommand::Diff { .. }) => {
            packages::run(command, api_url, api_key, org_id, output).await
        }
        AgentsCommand::Create {
            file,
            initial_files_dir,
            writable,
            name,
            system_prompt,
            description,
            model,
            harness,
            tag,
        } => {
            let use_default_file = name.is_none()
                && system_prompt.is_none()
                && description.is_none()
                && model.is_none()
                && harness.is_none()
                && tag.is_empty();
            let file = resolve_agent_file(file, use_default_file);
            if let Some(path) = file {
                if name.is_some()
                    || system_prompt.is_some()
                    || description.is_some()
                    || model.is_some()
                    || harness.is_some()
                    || !tag.is_empty()
                {
                    eprintln!("Warning: CLI flag overrides are ignored when --file is used");
                }
                import_from_file(
                    api_url,
                    api_key,
                    org_id,
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
                    harness,
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
            harness,
            tag,
        } => {
            let use_default_file = agent_id.is_none()
                && name.is_none()
                && system_prompt.is_none()
                && description.is_none()
                && model.is_none()
                && harness.is_none()
                && tag.is_empty();
            let file = resolve_agent_file(file, use_default_file);
            if let Some(path) = file {
                if agent_id.is_some()
                    || name.is_some()
                    || system_prompt.is_some()
                    || description.is_some()
                    || model.is_some()
                    || harness.is_some()
                    || !tag.is_empty()
                {
                    eprintln!("Warning: CLI flag overrides are ignored when --file is used");
                }
                import_from_file(
                    api_url,
                    api_key,
                    org_id,
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
                    harness,
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
/// The CLI normalizes TOML into JSON before calling the server import API.
/// When initial_files_dir is provided, files are globbed and injected into the
/// payload as initial_files before sending.
#[allow(clippy::too_many_arguments)]
async fn import_from_file(
    api_url: &str,
    api_key: &str,
    org_id: Option<&str>,
    path: &str,
    initial_files_dir: Option<&str>,
    writable: bool,
    output: OutputFormat,
    quiet: bool,
) -> Result<()> {
    let legacy_hidden_policy = std::fs::read_to_string(path)
        .ok()
        .is_some_and(|s| s.contains("initial_files_allow_hidden"));
    if initial_files_dir.is_none() && !legacy_hidden_policy {
        let package = everruns_core::agent_package::AgentPackage::load(path)?;
        let mut payload: serde_json::Value =
            serde_json::from_str(&package.to_string(everruns_core::agent_package::Format::Json)?)?;
        // Explicit legacy update IDs remain accepted; new exports contain none.
        if let Some(id) = package.legacy_id {
            payload
                .as_object_mut()
                .context("manifest")?
                .remove("schema_version");
            payload["id"] = serde_json::json!(id);
        }
        if let Some(id) = package.legacy_model_id {
            payload
                .as_object_mut()
                .context("manifest")?
                .remove("schema_version");
            payload["default_model_id"] = serde_json::json!(id);
        }
        if let Some(id) = package.legacy_harness_id {
            payload
                .as_object_mut()
                .context("manifest")?
                .remove("schema_version");
            payload["harness_id"] = serde_json::json!(id);
        }
        if writable && let Some(files) = payload["files"].as_array_mut() {
            for file in files {
                file["is_readonly"] = serde_json::json!(false);
            }
        }
        let response = super::api::ApiClient::new(api_url, api_key, org_id)
            .post("/v1/agents/import", Some(&payload))
            .await?;
        print_package_report(output, &response);
        return Ok(());
    }
    let file_path = Path::new(path);
    let content =
        std::fs::read_to_string(path).with_context(|| format!("Failed to read file: {}", path))?;

    // Resolve the agent file's parent directory for expanding initial_files globs.
    let file_dir = std::fs::canonicalize(path)
        .with_context(|| format!("Cannot resolve file path: {}", path))?
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Parse once when possible so TOML conversion and initial_files inspection
    // don't duplicate work before we build the request body.
    let parsed_agent = parse_agent_file_as_json(file_path, &content).ok();
    let has_glob_initial_files = parsed_agent.as_ref().is_some_and(initial_files_has_globs);
    let should_send_json =
        initial_files_dir.is_some() || has_glob_initial_files || is_toml_agent_file(file_path);

    let (body, content_type) = if should_send_json {
        let mut agent = if let Some(agent) = parsed_agent {
            agent
        } else {
            parse_agent_file_as_json(file_path, &content).context("Failed to parse agent file")?
        };

        if let Some(dir) = initial_files_dir {
            let allow_hidden = extract_allow_hidden_extras(&agent);
            let files = glob_initial_files(dir, writable, &allow_hidden)?;
            agent["initial_files"] = serde_json::to_value(&files)?;
        } else if has_glob_initial_files {
            let files = expand_initial_files_globs(&agent, &file_dir, writable)?;
            agent["initial_files"] = serde_json::to_value(&files)?;
        }

        // The opt-in field is only consumed locally to gate the upload.
        // Strip it before sending so the server import payload stays clean.
        if let Some(obj) = agent.as_object_mut() {
            obj.remove("initial_files_allow_hidden");
        }

        (serde_json::to_string(&agent)?, "application/json")
    } else {
        (content, "text/plain")
    };

    let http = reqwest::Client::new();
    let mut req = http
        .post(format!("{}/v1/agents/import", api_url))
        .header("Authorization", format!("Bearer {}", api_key))
        .header("Content-Type", content_type);
    let env_org = std::env::var("EVERRUNS_ORG_ID").ok();
    if let Some(org) = org_id.or(env_org.as_deref()) {
        req = req.header("X-Org-Id", org);
    }
    let resp = req
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

fn resolve_agent_file(file: Option<String>, use_default: bool) -> Option<String> {
    file.or_else(|| {
        if use_default && Path::new(DEFAULT_AGENT_FILE_NAME).is_file() {
            Some(DEFAULT_AGENT_FILE_NAME.to_string())
        } else {
            None
        }
    })
}

/// Represents a file to be uploaded as an initial file for an agent.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct CollectedFile {
    path: String,
    content: String,
    encoding: String,
    is_readonly: bool,
}

/// Extract user-declared hidden-path opt-ins from the agent manifest's
/// `initial_files_allow_hidden` field. Each entry must be a single normal path
/// component (basename) — entries containing `/` or `\\`, equal to `.` or
/// `..`, missing the leading dot, or matching a hard-denied basename in
/// `DENIED_DOT_ENTRIES` are filtered out.
fn extract_allow_hidden_extras(agent: &serde_json::Value) -> Vec<String> {
    let Some(arr) = agent
        .get("initial_files_allow_hidden")
        .and_then(|v| v.as_array())
    else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .filter(|s| is_valid_basename_extra(s))
        .filter(|s| !DENIED_DOT_ENTRIES.contains(&s.as_str()))
        .collect()
}

/// True when `s` is a single hidden basename (starts with `.`, contains no
/// path separator, and is not the relative-path placeholders `.` / `..`).
fn is_valid_basename_extra(s: &str) -> bool {
    if !s.starts_with('.') || s == "." || s == ".." {
        return false;
    }
    !s.contains('/') && !s.contains('\\')
}

/// Precomputed allow/deny lookup for the hidden-path policy. Built once per
/// import so repeated calls during directory walks avoid allocating a fresh
/// `Vec` on every component check.
struct HiddenPathPolicy {
    allowed: std::collections::HashSet<String>,
}

impl HiddenPathPolicy {
    fn new(extras: &[String]) -> Self {
        let mut allowed: std::collections::HashSet<String> = ALLOWED_DOT_ENTRIES
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        for extra in extras {
            if is_valid_basename_extra(extra) && !DENIED_DOT_ENTRIES.contains(&extra.as_str()) {
                allowed.insert(extra.clone());
            }
        }
        Self { allowed }
    }

    /// Iterator over the names of allow-walkable dot directories that the
    /// extras-aware walker should descend into in addition to the default
    /// non-hidden tree.
    fn allow_walk_names(&self) -> impl Iterator<Item = &String> {
        self.allowed.iter()
    }

    /// True when a single path component is hidden and either (a) hard-denied,
    /// or (b) not in the effective allowlist.
    fn component_is_disallowed(&self, component: &str) -> bool {
        if !component.starts_with('.') {
            return false;
        }
        if DENIED_DOT_ENTRIES.contains(&component) {
            return true;
        }
        !self.allowed.contains(component)
    }

    /// True when any path component is disallowed under the policy. Walks
    /// every `Normal` component so denied entries nested inside an allowlisted
    /// root (e.g. `.github/.env`) are still rejected.
    fn path_has_disallowed_component(&self, path: &Path) -> bool {
        path.components().any(|component| {
            let std::path::Component::Normal(name) = component else {
                return false;
            };
            self.component_is_disallowed(&name.to_string_lossy())
        })
    }
}

/// Recursively collect text files from a directory, including allowed
/// dotfile directories (e.g. `.agents/`). Returns them as initial_files
/// entries with paths relative to /workspace.
fn glob_initial_files(
    dir: &str,
    writable: bool,
    allow_hidden_extras: &[String],
) -> Result<Vec<CollectedFile>> {
    let base =
        std::fs::canonicalize(dir).with_context(|| format!("Cannot resolve directory: {}", dir))?;
    if !base.is_dir() {
        anyhow::bail!("Not a directory: {}", base.display());
    }

    let policy = HiddenPathPolicy::new(allow_hidden_extras);

    // hidden(true) prunes most dotfiles/dirs during traversal for performance.
    // We do a second walk of policy.allow_walk_names() below to include them.
    let walker = ignore::WalkBuilder::new(&base)
        .hidden(true) // skip dotfiles by default (perf: avoids traversing .git/)
        .git_ignore(true) // respect .gitignore (repo-local)
        .git_global(false) // ignore global gitignore for predictable behavior
        .git_exclude(false) // ignore repo exclude files for predictable behavior
        .build();

    // Also walk allowed dot-directories that hidden(true) would skip.
    // Each candidate file is filtered through the policy below so that nested
    // hard-denied components (e.g. `.github/.env`) are still rejected even
    // under an allowlisted root.
    let dot_walkers: Vec<_> = policy
        .allow_walk_names()
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

        // Defense in depth: enforce the deny floor on every component, even
        // for files surfaced via the allowlisted-root walkers.
        if policy.path_has_disallowed_component(rel) {
            eprintln!(
                "Warning: skipping hidden path: {} (allowed: {:?})",
                canonical.display(),
                ALLOWED_DOT_ENTRIES
            );
            continue;
        }

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
fn initial_files_has_globs(agent: &serde_json::Value) -> bool {
    let Some(arr) = agent.get("initial_files").and_then(|v| v.as_array()) else {
        return false;
    };
    arr.iter().any(|v| v.is_string())
}

fn is_toml_agent_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("toml"))
}

/// Strip glob metacharacters from a pattern to find the directory prefix.
/// e.g. ".agents/*" → ".agents", "src/**/*.rs" → "src", "." → "."
fn glob_base_dir(pattern: &str) -> &str {
    // Find the first glob metacharacter
    if let Some(pos) = pattern.find(['*', '?', '[']) {
        // Walk back to the last path separator before the metacharacter
        let prefix = &pattern[..pos];
        prefix.trim_end_matches('/').trim_end_matches('\\')
    } else {
        pattern
    }
}

/// Expand initial_files glob patterns from agent frontmatter into CollectedFile entries.
/// String entries are treated as relative paths resolved against `base_dir`.
/// All workspace paths are computed relative to `base_dir` so subdirectory
/// prefixes are preserved (e.g. ".agents/config.json" → "/workspace/.agents/config.json").
/// Object entries (already-expanded InitialFile) are passed through as-is.
fn expand_initial_files_globs(
    agent: &serde_json::Value,
    base_dir: &Path,
    writable: bool,
) -> Result<Vec<CollectedFile>> {
    let Some(arr) = agent.get("initial_files").and_then(|v| v.as_array()) else {
        return Ok(vec![]);
    };

    let allow_hidden_extras = extract_allow_hidden_extras(agent);
    let policy = HiddenPathPolicy::new(&allow_hidden_extras);
    let base_canonical = std::fs::canonicalize(base_dir)?;
    let mut all_files: Vec<CollectedFile> = Vec::new();

    for entry in arr {
        if let Some(pattern) = entry.as_str() {
            // Strip glob metacharacters to find the actual directory/file path.
            // e.g. ".agents/*" → ".agents", "." → "."
            let clean_path = glob_base_dir(pattern);
            let resolved = base_dir.join(clean_path);
            let resolved = std::fs::canonicalize(&resolved).with_context(|| {
                format!(
                    "Cannot resolve initial_files path: {} (relative to {})",
                    pattern,
                    base_dir.display()
                )
            })?;

            if resolved.is_dir() {
                // Reject hidden directory roots unless explicitly allowlisted.
                let rel = resolved.strip_prefix(&base_canonical)?;
                if policy.path_has_disallowed_component(rel) {
                    eprintln!(
                        "Warning: skipping hidden directory: {} (allowed: {:?}; user opt-ins: {:?})",
                        resolved.display(),
                        ALLOWED_DOT_ENTRIES,
                        allow_hidden_extras,
                    );
                    continue;
                }
                // Directory: collect all files, computing workspace paths relative to base_dir
                collect_dir_files(
                    &resolved,
                    &base_canonical,
                    writable,
                    &policy,
                    &mut all_files,
                )?;
            } else if resolved.is_file() {
                // Single file — reject disallowed hidden files
                collect_single_file(
                    &resolved,
                    &base_canonical,
                    writable,
                    &policy,
                    &mut all_files,
                )?;
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

/// Collect all text files from a directory into the CollectedFile list.
/// Workspace paths are computed relative to `workspace_base` (the agent file's
/// parent directory), preserving subdirectory prefixes.
fn collect_dir_files(
    dir: &Path,
    workspace_base: &Path,
    writable: bool,
    policy: &HiddenPathPolicy,
    files: &mut Vec<CollectedFile>,
) -> Result<()> {
    // Main walker skips hidden files for security (.env, .ssh, etc.)
    let walker = ignore::WalkBuilder::new(dir)
        .hidden(true)
        .git_ignore(true)
        .git_global(false)
        .git_exclude(false)
        .build();

    // Also walk allowed dot-directories that hidden(true) would skip.
    // Each candidate file is filtered by the policy below so nested hard-denied
    // components (e.g. `.github/.env`) are still rejected.
    let dot_walkers: Vec<_> = policy
        .allow_walk_names()
        .filter_map(|name| {
            let dot_path = dir.join(name);
            if dot_path.is_dir() {
                Some(
                    ignore::WalkBuilder::new(&dot_path)
                        .hidden(false)
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

    let all_entries = walker.chain(dot_walkers.into_iter().flatten());
    for entry in all_entries {
        let entry = entry.context("Failed to read directory entry")?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let canonical = std::fs::canonicalize(path)
            .with_context(|| format!("Cannot resolve file: {}", path.display()))?;
        if !canonical.starts_with(workspace_base) {
            eprintln!(
                "Warning: skipping symlink outside base directory: {}",
                path.display()
            );
            continue;
        }

        // Compute workspace path relative to workspace_base (agent file dir)
        let rel = canonical
            .strip_prefix(workspace_base)
            .context("File outside base directory")?;

        // Defense in depth: re-check every component against the deny floor,
        // since the allowlisted-root walkers descend with `hidden(false)`.
        if policy.path_has_disallowed_component(rel) {
            eprintln!(
                "Warning: skipping hidden path: {} (allowed: {:?})",
                canonical.display(),
                ALLOWED_DOT_ENTRIES
            );
            continue;
        }

        let rel_normalized = rel.to_string_lossy().replace('\\', "/");
        let workspace_path = format!("/workspace/{}", rel_normalized);

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

    Ok(())
}

/// Collect a single file into the CollectedFile list.
/// Rejects hidden files not in the effective allowlist (see
/// `HiddenPathPolicy`). Hard-denied entries from `DENIED_DOT_ENTRIES` are
/// always rejected, even if the user opts in.
fn collect_single_file(
    file_path: &Path,
    workspace_base: &Path,
    writable: bool,
    policy: &HiddenPathPolicy,
    files: &mut Vec<CollectedFile>,
) -> Result<()> {
    let canonical = std::fs::canonicalize(file_path)
        .with_context(|| format!("Cannot resolve file: {}", file_path.display()))?;

    if !canonical.starts_with(workspace_base) {
        eprintln!(
            "Warning: skipping file outside base directory: {}",
            file_path.display()
        );
        return Ok(());
    }

    // Check for disallowed hidden path components (e.g. .env, .ssh/config)
    let rel = canonical.strip_prefix(workspace_base)?;
    if policy.path_has_disallowed_component(rel) {
        eprintln!(
            "Warning: skipping hidden file: {} (allowed: {:?})",
            file_path.display(),
            ALLOWED_DOT_ENTRIES,
        );
        return Ok(());
    }

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

/// Parse agent file content (Markdown/TOML/YAML/JSON) into a JSON Value.
/// This is minimal parsing to allow injecting initial_files before sending
/// to the server import API.
fn parse_agent_file_as_json(path: &Path, content: &str) -> Result<serde_json::Value> {
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

    if is_toml_agent_file(path) {
        let val: toml::Value = toml::from_str(content).context("Failed to parse TOML")?;
        let val = serde_json::to_value(val).context("Failed to convert TOML to JSON")?;
        if !val.is_object() {
            anyhow::bail!("Agent file must be a TOML object");
        }
        return Ok(val);
    }

    // YAML
    let val: serde_json::Value = serde_yaml::from_str(content).context("Failed to parse YAML")?;
    if !val.is_object() {
        anyhow::bail!("Agent file must be a YAML/JSON object");
    }
    Ok(val)
}

/// Apply a `--harness` value to the request, detecting a strict harness id
/// (`harness_<32-hex>`) vs. an addressable name (e.g. `generic`). Mirrors the
/// `sessions create --harness` detection so the two commands behave the same.
fn apply_harness(req: CreateAgentRequest, harness: Option<String>) -> CreateAgentRequest {
    match harness {
        Some(h) if is_prefixed_id(&h, "harness") => req.harness_id(h),
        Some(h) => req.harness_name(h),
        None => req,
    }
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
    harness: Option<String>,
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
    req = apply_harness(req, harness);
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
    harness: Option<String>,
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
    req = apply_harness(req, harness);
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
mod tests;
