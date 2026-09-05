// Agent management commands
//
// Design Decision: Most file formats are forwarded to the server import API
// unchanged, but TOML is normalized client-side into JSON because the server
// import endpoint only parses Markdown/YAML/JSON today. The CLI also parses any
// agent file locally when it needs to inject initial_files.
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
use everruns_sdk::{CreateAgentRequest, Everruns};
use serde::Deserialize;
use std::path::Path;

const DEFAULT_AGENT_FILE_NAME: &str = "agent.toml";

#[derive(Subcommand)]
pub enum AgentsCommand {
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
        #[arg(long)]
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
        #[arg(long)]
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

/// Dot-prefixed path components allowed by default in initial-files collection.
/// Covers the common dev-ecosystem assets shipped alongside agent packages.
/// Anything not in this list (and not in the user-declared
/// `initial_files_allow_hidden` opt-in) is skipped to prevent accidental upload
/// of secrets (`.env`, `.ssh/`, `.npmrc`, etc.).
const ALLOWED_DOT_ENTRIES: &[&str] = &[
    ".agents",
    ".github",
    ".vscode",
    ".claude",
    ".cursor",
    ".mcp.json",
    ".gitignore",
    ".gitattributes",
    ".editorconfig",
    ".prettierrc",
    ".prettierrc.json",
    ".prettierrc.yaml",
    ".prettierrc.yml",
    ".prettierrc.js",
    ".prettierrc.cjs",
    ".prettierrc.mjs",
    ".eslintrc",
    ".eslintrc.json",
    ".eslintrc.yaml",
    ".eslintrc.yml",
    ".eslintrc.js",
    ".eslintrc.cjs",
    ".eslintignore",
    ".nvmrc",
    ".node-version",
    ".python-version",
    ".tool-versions",
    ".dockerignore",
    ".rubocop.yml",
];

/// Hard-deny floor for hidden path components. Even if a user opts in via the
/// `initial_files_allow_hidden` manifest field, anything matching one of these
/// entries (exactly, by basename) is rejected. Protects against accidental
/// exfiltration of credentials, SSH/GPG keys, and shell history.
///
/// Keep this list strict and well-known. Adding speculative entries here is
/// safer than adding them to `ALLOWED_DOT_ENTRIES`.
const DENIED_DOT_ENTRIES: &[&str] = &[
    ".env",
    ".env.local",
    ".env.development",
    ".env.production",
    ".env.test",
    ".envrc",
    ".ssh",
    ".gnupg",
    ".aws",
    ".azure",
    ".gcloud",
    ".kube",
    ".docker",
    ".npmrc",
    ".yarnrc",
    ".pypirc",
    ".netrc",
    ".cargo",
    ".git",
    ".hg",
    ".svn",
    ".bash_history",
    ".zsh_history",
    ".python_history",
    ".node_repl_history",
];

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
    fn test_apply_harness_detects_strict_id() {
        let id = "harness_00000000000000000000000000000001";
        let req = apply_harness(CreateAgentRequest::new("a", "p"), Some(id.to_string()));
        assert_eq!(req.harness_id.as_deref(), Some(id));
        assert!(req.harness_name.is_none());
    }

    #[test]
    fn test_apply_harness_treats_bare_name_as_name() {
        let req = apply_harness(
            CreateAgentRequest::new("a", "p"),
            Some("generic".to_string()),
        );
        assert_eq!(req.harness_name.as_deref(), Some("generic"));
        assert!(req.harness_id.is_none());
    }

    #[test]
    fn test_apply_harness_keeps_prefix_names_as_name() {
        // "harness_generic" is not a strict harness id (not 32 hex), so it is a name.
        let req = apply_harness(
            CreateAgentRequest::new("a", "p"),
            Some("harness_generic".to_string()),
        );
        assert_eq!(req.harness_name.as_deref(), Some("harness_generic"));
        assert!(req.harness_id.is_none());
    }

    #[test]
    fn test_apply_harness_none_sets_nothing() {
        let req = apply_harness(CreateAgentRequest::new("a", "p"), None);
        assert!(req.harness_id.is_none());
        assert!(req.harness_name.is_none());
    }

    #[test]
    fn test_parse_agent_file_json() {
        let content = r#"{"name":"test","system_prompt":"hello"}"#;
        let val = parse_agent_file_as_json(Path::new("agent.json"), content).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["system_prompt"], "hello");
    }

    #[test]
    fn test_parse_agent_file_yaml() {
        let content = "name: test\nsystem_prompt: hello\n";
        let val = parse_agent_file_as_json(Path::new("agent.yaml"), content).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["system_prompt"], "hello");
    }

    #[test]
    fn test_parse_agent_file_toml() {
        let content = "name = \"test\"\nsystem_prompt = \"hello\"\n";
        let val = parse_agent_file_as_json(Path::new("agent.toml"), content).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["system_prompt"], "hello");
    }

    #[test]
    fn test_parse_agent_file_markdown() {
        let content = "---\nname: test\n---\nHello world";
        let val = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
        assert_eq!(val["name"], "test");
        assert_eq!(val["system_prompt"], "Hello world");
    }

    #[test]
    fn test_parse_agent_file_markdown_with_system_prompt() {
        let content = "---\nname: test\nsystem_prompt: from frontmatter\n---\nBody text";
        let val = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
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

        let files = glob_initial_files(dir.path().to_str().unwrap(), false, &[]).unwrap();
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

        let files = glob_initial_files(dir.path().to_str().unwrap(), true, &[]).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/workspace/hello.txt");
        assert!(!files[0].is_readonly);
    }

    #[test]
    fn test_glob_initial_files_includes_dot_agents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("agent.md"), "# Agent").unwrap();
        std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
        std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();

        let files = glob_initial_files(dir.path().to_str().unwrap(), false, &[]).unwrap();
        assert_eq!(files.len(), 2);
        assert!(files.iter().any(|f| f.path == "/workspace/agent.md"));
        assert!(
            files
                .iter()
                .any(|f| f.path == "/workspace/.agents/config.json")
        );
    }

    #[test]
    fn test_glob_initial_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = glob_initial_files(dir.path().to_str().unwrap(), false, &[]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("No files found"));
    }

    #[test]
    fn test_glob_initial_files_not_a_dir() {
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("file.txt");
        std::fs::write(&file_path, "content").unwrap();
        let result = glob_initial_files(file_path.to_str().unwrap(), false, &[]);
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
        let result = glob_initial_files(dir.path().to_str().unwrap(), false, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_agent_file_non_object_errors() {
        let content = "just a string";
        let result = parse_agent_file_as_json(Path::new("agent.yaml"), content);
        assert!(result.is_err());
    }

    #[test]
    fn test_has_initial_files_globs_with_strings() {
        let content = "---\nname: test\ninitial_files:\n  - .\n  - .agents/*\n---\nPrompt";
        let agent = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
        assert!(initial_files_has_globs(&agent));
    }

    #[test]
    fn test_has_initial_files_globs_without_strings() {
        // No initial_files at all
        let content = "---\nname: test\n---\nPrompt";
        let agent = parse_agent_file_as_json(Path::new("agent.md"), content).unwrap();
        assert!(!initial_files_has_globs(&agent));

        // initial_files with objects (already expanded)
        let content = r#"{"name":"test","initial_files":[{"path":"/workspace/f.txt","content":"x","encoding":"text","is_readonly":false}]}"#;
        let agent = parse_agent_file_as_json(Path::new("agent.json"), content).unwrap();
        assert!(!initial_files_has_globs(&agent));
    }

    #[test]
    fn test_has_initial_files_globs_toml() {
        let content = "name = \"test\"\ninitial_files = [\".\", \".agents/*\"]\n";
        let agent = parse_agent_file_as_json(Path::new("agent.toml"), content).unwrap();
        assert!(initial_files_has_globs(&agent));
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
        std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
        std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/nested.txt"), "nested").unwrap();

        // .agents subdirectory preserves prefix in workspace path
        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".agents"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/workspace/.agents/config.json");

        // Regular subdirectory also preserves prefix
        let agent2 = serde_json::json!({
            "name": "test",
            "initial_files": ["sub"]
        });

        let files2 = expand_initial_files_globs(&agent2, dir.path(), false).unwrap();
        assert_eq!(files2.len(), 1);
        assert_eq!(files2[0].path, "/workspace/sub/nested.txt");
    }

    #[test]
    fn test_expand_initial_files_globs_with_wildcard() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("root.txt"), "root").unwrap();
        std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
        std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();

        // ".agents/*" should work — the * is stripped, .agents/ is walked
        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".agents/*"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/workspace/.agents/config.json");
    }

    #[test]
    fn test_expand_initial_files_globs_deduplicates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        std::fs::write(dir.path().join("sub/readme.md"), "# Hi").unwrap();

        // Both "." entries collect the same files — dedup by workspace path
        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".", "."]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        let hello_count = files
            .iter()
            .filter(|f| f.path.contains("hello.txt"))
            .count();
        assert_eq!(hello_count, 1);
        assert_eq!(files.len(), 2);

        // "." and "sub" overlap on sub/readme.md — dedup keeps first occurrence
        let agent2 = serde_json::json!({
            "name": "test",
            "initial_files": [".", "sub"]
        });

        let files2 = expand_initial_files_globs(&agent2, dir.path(), false).unwrap();
        assert_eq!(files2.len(), 2);
        assert!(files2.iter().any(|f| f.path == "/workspace/hello.txt"));
        assert!(files2.iter().any(|f| f.path == "/workspace/sub/readme.md"));
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

    #[test]
    fn test_expand_initial_files_rejects_hidden_single_file() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=key").unwrap();
        std::fs::write(dir.path().join("ok.txt"), "safe").unwrap();

        // Explicitly listing .env should be rejected (hidden, not in allowlist)
        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".env", "ok.txt"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/workspace/ok.txt");
    }

    #[test]
    fn test_expand_initial_files_rejects_hidden_directory() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
        std::fs::write(dir.path().join(".ssh/config"), "Host *").unwrap();
        std::fs::write(dir.path().join("ok.txt"), "safe").unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".ssh", "ok.txt"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/workspace/ok.txt");
    }

    #[test]
    fn test_glob_base_dir() {
        assert_eq!(glob_base_dir("."), ".");
        assert_eq!(glob_base_dir(".agents"), ".agents");
        assert_eq!(glob_base_dir(".agents/*"), ".agents");
        assert_eq!(glob_base_dir("src/**/*.rs"), "src");
        assert_eq!(glob_base_dir("*.txt"), "");
        assert_eq!(glob_base_dir("dir/sub/*.md"), "dir/sub");
    }

    #[test]
    fn test_expand_initial_files_includes_default_dev_dot_dirs() {
        // Common dev-ecosystem dot directories ship by default without
        // requiring `initial_files_allow_hidden`. This is only about file
        // collection policy; the CLI does not interpret tool-specific files.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".agents")).unwrap();
        std::fs::write(dir.path().join(".agents/config.json"), "{}").unwrap();
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        std::fs::write(dir.path().join(".github/workflows/ci.yml"), "name: ci").unwrap();
        std::fs::create_dir_all(dir.path().join(".vscode")).unwrap();
        std::fs::write(
            dir.path().join(".vscode/settings.json"),
            "{\"editor.tabSize\":2}",
        )
        .unwrap();
        std::fs::create_dir_all(dir.path().join(".claude")).unwrap();
        std::fs::write(
            dir.path().join(".claude/settings.json"),
            "{\"permissions\":{}}",
        )
        .unwrap();

        for dot_path in [".agents", ".github", ".vscode", ".claude"] {
            let agent = serde_json::json!({
                "name": "test",
                "initial_files": [dot_path]
            });
            let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
            assert!(
                !files.is_empty(),
                "expected files when shipping {dot_path}, got none"
            );
            assert!(
                files.iter().all(|f| f.path.starts_with("/workspace/")),
                "all collected files must be under /workspace"
            );
        }
    }

    #[test]
    fn test_expand_initial_files_user_opt_in_extras() {
        // User-declared `initial_files_allow_hidden` extends the allowlist.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".mytool")).unwrap();
        std::fs::write(dir.path().join(".mytool/config"), "k=v").unwrap();

        // Without opt-in: hidden directory is skipped.
        let baseline = serde_json::json!({
            "name": "test",
            "initial_files": [".mytool", "."]
        });
        std::fs::write(dir.path().join("ok.txt"), "ok").unwrap();
        let baseline_files = expand_initial_files_globs(&baseline, dir.path(), false).unwrap();
        assert!(
            baseline_files.iter().all(|f| !f.path.contains("/.mytool/")),
            "baseline must not include .mytool"
        );

        // With opt-in: .mytool is collected.
        let opt_in = serde_json::json!({
            "name": "test",
            "initial_files": [".mytool"],
            "initial_files_allow_hidden": [".mytool"]
        });
        let opt_in_files = expand_initial_files_globs(&opt_in, dir.path(), false).unwrap();
        assert!(
            opt_in_files
                .iter()
                .any(|f| f.path == "/workspace/.mytool/config"),
            "opt-in must include .mytool/config, got: {:?}",
            opt_in_files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_expand_initial_files_hard_deny_blocks_opt_in() {
        // Even if a user opts in, .ssh/.env are always rejected.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
        std::fs::write(dir.path().join(".ssh/config"), "Host *").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=key").unwrap();
        std::fs::write(dir.path().join("ok.txt"), "safe").unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".ssh", ".env", "ok.txt"],
            "initial_files_allow_hidden": [".ssh", ".env"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/workspace/ok.txt");
    }

    #[test]
    fn test_extract_allow_hidden_extras_filters() {
        // Non-hidden, denied, and non-string entries are filtered out.
        let agent = serde_json::json!({
            "initial_files_allow_hidden": [
                ".mytool",   // kept
                "regular",   // dropped: not hidden
                ".ssh",      // dropped: hard-denied
                ".env",      // dropped: hard-denied
                42           // dropped: not a string
            ]
        });
        let extras = extract_allow_hidden_extras(&agent);
        assert_eq!(extras, vec![".mytool".to_string()]);
    }

    #[test]
    fn test_is_disallowed_hidden_with_extras_and_deny() {
        // No extras: built-in allowlist + deny floor.
        let base = HiddenPathPolicy::new(&[]);
        assert!(!base.component_is_disallowed(".github"));
        assert!(!base.component_is_disallowed(".claude"));
        assert!(base.component_is_disallowed(".env"));
        assert!(base.component_is_disallowed(".ssh"));

        // User opt-in extends allowlist.
        let with_extras = HiddenPathPolicy::new(&[".mytool".to_string()]);
        assert!(!with_extras.component_is_disallowed(".mytool"));

        // Deny floor is unconditional, even if extras try to override it.
        let bypass = HiddenPathPolicy::new(&[".ssh".to_string(), ".env".to_string()]);
        assert!(bypass.component_is_disallowed(".ssh"));
        assert!(bypass.component_is_disallowed(".env"));

        // Non-hidden components are never gated.
        assert!(!base.component_is_disallowed("regular"));
        assert!(!base.component_is_disallowed(""));

        // Nested deny components are caught by path-level check.
        assert!(base.path_has_disallowed_component(Path::new(".github/.env")));
        assert!(base.path_has_disallowed_component(Path::new(".claude/.ssh/config")));
    }

    #[test]
    fn test_is_valid_basename_extra_rejects_paths() {
        // Reject path separators, relative-path placeholders, and missing dot.
        assert!(!is_valid_basename_extra(""));
        assert!(!is_valid_basename_extra("."));
        assert!(!is_valid_basename_extra(".."));
        assert!(!is_valid_basename_extra("regular"));
        assert!(!is_valid_basename_extra(".mytool/sub"));
        assert!(!is_valid_basename_extra(".mytool\\sub"));
        assert!(!is_valid_basename_extra("../escape"));
        assert!(is_valid_basename_extra(".mytool"));
        assert!(is_valid_basename_extra(".otherproj"));
    }

    #[test]
    fn test_extract_allow_hidden_extras_rejects_path_separators() {
        let agent = serde_json::json!({
            "initial_files_allow_hidden": [
                ".mytool",
                ".otherproj/sub",   // dropped: contains /
                ".escape\\windows", // dropped: contains \
                ".",                // dropped: relative-path placeholder
                "..",               // dropped: relative-path placeholder
            ]
        });
        let extras = extract_allow_hidden_extras(&agent);
        assert_eq!(extras, vec![".mytool".to_string()]);
    }

    #[test]
    fn test_collect_dir_files_rejects_nested_denied_under_allowed_root() {
        // .github/.env must NOT be collected even though .github is allowed.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join(".github/workflows")).unwrap();
        std::fs::write(dir.path().join(".github/workflows/ci.yml"), "name: ci").unwrap();
        std::fs::write(dir.path().join(".github/.env"), "SECRET=key").unwrap();

        let agent = serde_json::json!({
            "name": "test",
            "initial_files": [".github"]
        });

        let files = expand_initial_files_globs(&agent, dir.path(), false).unwrap();
        assert!(
            files
                .iter()
                .any(|f| f.path == "/workspace/.github/workflows/ci.yml"),
            "ci.yml should be collected"
        );
        assert!(
            files.iter().all(|f| !f.path.contains("/.env")),
            ".env nested under .github must NOT be collected, got: {:?}",
            files.iter().map(|f| &f.path).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_glob_initial_files_extras_walks_user_dot_dir() {
        // glob_initial_files (the `--initial-files-dir` path) honors extras too.
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("hello.txt"), "world").unwrap();
        std::fs::create_dir_all(dir.path().join(".mytool")).unwrap();
        std::fs::write(dir.path().join(".mytool/config"), "k=v").unwrap();

        // Without opt-in: .mytool dropped.
        let baseline = glob_initial_files(dir.path().to_str().unwrap(), false, &[]).unwrap();
        assert!(
            baseline.iter().all(|f| !f.path.contains("/.mytool/")),
            "baseline should not include .mytool"
        );

        // With opt-in: .mytool walked.
        let extras = vec![".mytool".to_string()];
        let opt_in = glob_initial_files(dir.path().to_str().unwrap(), false, &extras).unwrap();
        assert!(
            opt_in.iter().any(|f| f.path == "/workspace/.mytool/config"),
            "opt-in must include .mytool/config"
        );
    }
}
