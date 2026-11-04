//! Read one immutable GitHub tree through the shared package codec; no repository code runs.
use super::*;
use base64::{Engine, engine::general_purpose::STANDARD};
use everruns_core::agent_package::{AgentPackage, Format, MAX_FILES, MAX_PACKAGE_BYTES};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::Path;
use utoipa::ToSchema;

/// Repository, branch and package path that define an Agent's GitHub source.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SourceLocation {
    /// GitHub repository in owner/name form.
    #[schema(example = "acme/agents")]
    pub repository: String,
    /// Branch whose future commits the Agent follows.
    #[schema(example = "main")]
    pub branch: String,
    /// Repository-relative package folder or manifest; empty selects the repository root.
    #[serde(default)]
    #[schema(example = "agents/support")]
    pub path: String,
}
impl SourceLocation {
    pub fn validate(&mut self) -> Result<(), CommandError> {
        self.repository =
            crate::domains::git_sources::normalize_github_repository(&self.repository)
                .map_err(CommandError::bad_request)?;
        if self.branch.len() > 255
            || !git2::Reference::is_valid_name(&format!("refs/heads/{}", self.branch))
        {
            return Err(CommandError::bad_request("Select a valid branch name"));
        }
        if self.path.starts_with('/') {
            return Err(CommandError::bad_request(
                "Select a repository-relative package path",
            ));
        }
        self.path = self.path.trim_end_matches('/').to_string();
        if self.path.len() > 1024
            || self.path.contains(['\\', ':', '\0'])
            || (!self.path.is_empty()
                && self
                    .path
                    .split('/')
                    .any(|p| p.is_empty() || matches!(p, "." | ".." | ".git")))
        {
            return Err(CommandError::bad_request(
                "Select a repository-relative package path",
            ));
        }
        Ok(())
    }
}

pub(super) struct Snapshot {
    pub package: AgentPackage,
    pub entries: BTreeMap<String, Vec<u8>>,
    pub manifest: String,
    pub package_root: String,
    pub root_tree: String,
    pub modes: BTreeMap<String, String>,
    pub commit: String,
    pub repository_id: i64,
    pub repository: String,
}

pub(super) fn segment(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

pub(super) async fn read(
    ctx: &Ctx,
    repo: &str,
    suffix: &str,
    token: &str,
) -> Result<Value, CommandError> {
    ctx.github_apps
        .api
        .read_source_json(&format!("/repos/{repo}/{suffix}"), token)
        .await
        .map_err(|e| CommandError::bad_request(e.to_string()))
}

pub(super) async fn metadata(ctx: &Ctx, repo: &str, token: &str) -> Result<Value, CommandError> {
    ctx.github_apps
        .api
        .read_source_json(&format!("/repos/{repo}"), token)
        .await
        .map_err(|e| CommandError::bad_request(e.to_string()))
}

pub(super) async fn branches(
    ctx: &Ctx,
    repo: &str,
    token: &str,
) -> Result<Vec<String>, CommandError> {
    let mut names = Vec::new();
    for page in 1..=3 {
        let value = read(
            ctx,
            repo,
            &format!("branches?per_page=100&page={page}"),
            token,
        )
        .await?;
        let batch = value.as_array().ok_or_else(invalid)?;
        names.extend(
            batch
                .iter()
                .filter_map(|v| v["name"].as_str().map(str::to_string)),
        );
        if batch.len() < 100 {
            break;
        }
    }
    Ok(names)
}

fn invalid() -> CommandError {
    CommandError::bad_request("GitHub returned an incomplete source response")
}
fn field(value: &Value, key: &str) -> Result<String, CommandError> {
    value[key].as_str().map(str::to_string).ok_or_else(invalid)
}
pub(super) fn sha(value: &Value) -> Result<String, CommandError> {
    let sha = field(value, "sha")?;
    if sha.len() != 40 || !sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(invalid());
    }
    Ok(sha)
}

pub(super) async fn root(
    ctx: &Ctx,
    location: &SourceLocation,
    token: &str,
) -> Result<(String, String), CommandError> {
    let reference = read(
        ctx,
        &location.repository,
        &format!("git/ref/heads/{}", segment(&location.branch)),
        token,
    )
    .await?;
    let commit = sha(&reference["object"])?;
    let record = read(
        ctx,
        &location.repository,
        &format!("git/commits/{commit}"),
        token,
    )
    .await?;
    Ok((commit, sha(&record["tree"])?))
}

async fn tree(
    ctx: &Ctx,
    repo: &str,
    tree: &str,
    recursive: bool,
    token: &str,
) -> Result<Vec<Value>, CommandError> {
    let value = read(
        ctx,
        repo,
        &format!(
            "git/trees/{tree}{}",
            if recursive { "?recursive=1" } else { "" }
        ),
        token,
    )
    .await?;
    if value["truncated"].as_bool() != Some(false) {
        return Err(CommandError::bad_request(
            "Source tree is too large. Select a smaller package folder",
        ));
    }
    Ok(value["tree"].as_array().ok_or_else(invalid)?.clone())
}

async fn descend(
    ctx: &Ctx,
    repo: &str,
    mut current: String,
    path: &str,
    token: &str,
) -> Result<String, CommandError> {
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let entries = tree(ctx, repo, &current, false, token).await?;
        let entry = entries
            .iter()
            .find(|v| v["path"].as_str() == Some(part) && v["type"] == "tree")
            .ok_or_else(|| {
                CommandError::bad_request(
                    "Source folder was not found; symlinks and submodules cannot be followed",
                )
            })?;
        current = sha(entry)?;
    }
    Ok(current)
}

pub(super) async fn browse(
    ctx: &Ctx,
    location: &SourceLocation,
    token: &str,
) -> Result<Vec<String>, CommandError> {
    let (_, root) = root(ctx, location, token).await?;
    let current = descend(ctx, &location.repository, root, &location.path, token).await?;
    let prefix = if location.path.is_empty() {
        String::new()
    } else {
        format!("{}/", location.path)
    };
    Ok(tree(ctx, &location.repository, &current, false, token)
        .await?
        .iter()
        .filter(|v| {
            v["type"] == "tree"
                || (v["type"] == "blob"
                    && v["mode"] != "120000"
                    && is_manifest(v["path"].as_str().unwrap_or_default()))
        })
        .filter_map(|v| {
            v["path"]
                .as_str()
                .map(|p| format!("{prefix}{p}{}", if v["type"] == "tree" { "/" } else { "" }))
        })
        .collect())
}

fn is_manifest(path: &str) -> bool {
    matches!(
        Path::new(path).extension().and_then(|s| s.to_str()),
        Some("toml" | "md" | "yaml" | "yml" | "json")
    )
}

async fn blob(ctx: &Ctx, repo: &str, entry: &Value, token: &str) -> Result<Vec<u8>, CommandError> {
    if entry["mode"] == "120000" || entry["type"] != "blob" {
        return Err(CommandError::bad_request(
            "Package assets must be regular files; symlinks and submodules are not allowed",
        ));
    }
    if entry["size"]
        .as_u64()
        .is_none_or(|size| size > MAX_PACKAGE_BYTES as u64)
    {
        return Err(CommandError::bad_request("Package file exceeds 10 MiB"));
    }
    let value = read(ctx, repo, &format!("git/blobs/{}", sha(entry)?), token).await?;
    if value["encoding"] != "base64" {
        return Err(invalid());
    }
    let bytes = STANDARD
        .decode(field(&value, "content")?.replace(['\r', '\n'], ""))
        .map_err(|_| invalid())?;
    if bytes.len() > MAX_PACKAGE_BYTES {
        return Err(CommandError::bad_request("Package file exceeds 10 MiB"));
    }
    Ok(bytes)
}

/// Cheap periodic checks avoid downloading assets when the applied commit is still current.
pub(super) async fn unchanged(
    ctx: &Ctx,
    location: &SourceLocation,
    token: &str,
    repository_id: i64,
    applied: Option<&str>,
) -> Result<bool, CommandError> {
    let info = metadata(ctx, &location.repository, token).await?;
    if info["id"].as_i64() != Some(repository_id) {
        return Err(CommandError::bad_request(
            "The repository identity changed. Select the source again",
        ));
    }
    let reference = read(
        ctx,
        &location.repository,
        &format!("git/ref/heads/{}", segment(&location.branch)),
        token,
    )
    .await?;
    Ok(applied == Some(sha(&reference["object"])?.as_str()))
}

pub(super) async fn snapshot(
    ctx: &Ctx,
    location: &SourceLocation,
    token: &str,
) -> Result<Snapshot, CommandError> {
    let info = metadata(ctx, &location.repository, token).await?;
    let repository_id = info["id"].as_i64().ok_or_else(invalid)?;
    let repository = field(&info, "full_name")?;
    // Every file is addressed by immutable Git object ID from this one commit.
    let (commit, root) = root(ctx, location, token).await?;
    // Git entry type, rather than the path suffix, distinguishes a package
    // folder such as `agents/support.md` from a standalone manifest.
    let root_tree = root.clone();
    let mut selected = None;
    let current = if location.path.is_empty() {
        root
    } else {
        let path = Path::new(&location.path);
        let name = path
            .file_name()
            .and_then(|p| p.to_str())
            .ok_or_else(invalid)?;
        let parent = path.parent().and_then(|p| p.to_str()).unwrap_or("");
        let parent = descend(ctx, &location.repository, root, parent, token).await?;
        let siblings = tree(ctx, &location.repository, &parent, false, token).await?;
        let entry = siblings
            .iter()
            .find(|entry| entry["path"].as_str() == Some(name))
            .ok_or_else(|| CommandError::bad_request("Source path was not found"))?;
        if entry["type"] == "tree" {
            sha(entry)?
        } else if entry["type"] == "blob" && is_manifest(name) {
            selected = Some(name);
            parent
        } else {
            return Err(CommandError::bad_request(
                "Select a package folder or supported manifest file; submodules cannot be followed",
            ));
        }
    };
    let entries = tree(ctx, &location.repository, &current, true, token).await?;
    let manifests: Vec<_> = entries
        .iter()
        .filter(|v| {
            let p = v["path"].as_str().unwrap_or_default();
            match selected {
                Some(selected) => p == selected,
                None => matches!(
                    p,
                    "agent.toml" | "agent.md" | "agent.yaml" | "agent.yml" | "agent.json"
                ),
            }
        })
        .collect();
    if manifests.len() != 1 {
        return Err(CommandError::bad_request(
            "Select a folder containing exactly one Agent manifest, or select a manifest file",
        ));
    }
    let manifest = manifests[0];
    let name = field(manifest, "path")?;
    let content = blob(ctx, &location.repository, manifest, token).await?;
    let package = AgentPackage::parse(
        std::str::from_utf8(&content)
            .map_err(|_| CommandError::bad_request("Agent manifest must be UTF-8"))?,
        Format::from_extension(Path::new(&name)),
    )
    .map_err(packages::package_error)?;
    let asset_paths = package
        .referenced_paths(entries.iter().filter_map(|v| v["path"].as_str()))
        .map_err(packages::package_error)?;
    let mut total = content.len();
    let mut files = BTreeMap::new();
    files.insert(name.clone(), content);
    for entry in &entries {
        let path = entry["path"].as_str().ok_or_else(invalid)?;
        if path == name || !asset_paths.contains(path) || entry["type"] == "tree" {
            continue;
        }
        if files.len() >= MAX_FILES + 2 {
            return Err(CommandError::bad_request(
                "Agent package has too many files",
            ));
        }
        let bytes = blob(ctx, &location.repository, entry, token).await?;
        total += bytes.len();
        if total > MAX_PACKAGE_BYTES {
            return Err(CommandError::bad_request("Agent package exceeds 10 MiB"));
        }
        files.insert(path.into(), bytes);
    }
    let package = AgentPackage::from_manifest_entries(files.clone(), &name)
        .map_err(packages::package_error)?;
    let package_root = if selected.is_some() {
        Path::new(&location.path)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or("")
            .to_string()
    } else {
        location.path.clone()
    };
    let modes = entries
        .iter()
        .filter_map(|e| {
            Some((
                e["path"].as_str()?.to_string(),
                e["mode"].as_str()?.to_string(),
            ))
        })
        .collect();
    Ok(Snapshot {
        entries: files,
        manifest: name,
        package_root,
        root_tree,
        modes,
        package,
        commit,
        repository_id,
        repository,
    })
}
