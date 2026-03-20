// Sync engine — reconciles local and remote file trees.
//
// Design Decision: Sync operates on normalized paths (forward slashes, no leading slash for local).
// Design Decision: Conflict resolution is configurable (last-write, local, remote).

use crate::commands::files::remote::{RemoteClient, RemoteFileEntry};
use crate::commands::files::state::{FileSyncState, SyncState, content_hash, state_dir};
use anyhow::{Context, Result};
use ignore::WalkBuilder;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Conflict resolution strategy.
#[derive(Debug, Clone, Copy)]
pub enum Conflict {
    LastWrite,
    Local,
    Remote,
}

impl Conflict {
    pub fn parse(s: &str) -> Self {
        match s {
            "local-wins" => Self::Local,
            "remote-wins" => Self::Remote,
            _ => Self::LastWrite,
        }
    }
}

/// Summary of a sync cycle.
#[derive(Debug, Default)]
pub struct SyncStats {
    pub uploaded: u32,
    pub downloaded: u32,
    pub deleted_local: u32,
    pub deleted_remote: u32,
    pub conflicts: u32,
    pub skipped: u32,
    pub errors: u32,
}

impl std::fmt::Display for SyncStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "↑{} ↓{}", self.uploaded, self.downloaded)?;
        if self.deleted_local > 0 || self.deleted_remote > 0 {
            write!(f, " del:{}", self.deleted_local + self.deleted_remote)?;
        }
        if self.conflicts > 0 {
            write!(f, " conflicts:{}", self.conflicts)?;
        }
        if self.errors > 0 {
            write!(f, " errors:{}", self.errors)?;
        }
        Ok(())
    }
}

/// Walk local directory and collect file paths with their content hashes.
pub fn scan_local(
    local_dir: &Path,
    no_gitignore: bool,
    extra_excludes: &[String],
) -> Result<HashMap<String, (Vec<u8>, String)>> {
    let mut files = HashMap::new();

    let mut builder = WalkBuilder::new(local_dir);
    builder
        .hidden(false)
        .git_ignore(!no_gitignore)
        .git_global(false)
        .git_exclude(false);

    let default_excludes = [
        ".git",
        "node_modules",
        "target",
        "__pycache__",
        ".env",
        ".everruns-sync",
    ];

    let mut overrides = ignore::overrides::OverrideBuilder::new(local_dir);
    for pattern in default_excludes {
        overrides.add(&format!("!{}", pattern))?;
    }
    for pattern in extra_excludes {
        overrides.add(&format!("!{}", pattern))?;
    }

    let syncignore_path = local_dir.join(".syncignore");
    if syncignore_path.exists()
        && let Ok(content) = std::fs::read_to_string(&syncignore_path)
    {
        for line in content.lines() {
            let line = line.trim();
            if !line.is_empty() && !line.starts_with('#') {
                overrides.add(&format!("!{}", line))?;
            }
        }
    }

    builder.overrides(overrides.build()?);

    for entry in builder.build() {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            continue;
        }

        let rel = path.strip_prefix(local_dir).context("Strip local prefix")?;
        let normalized = normalize_path(rel);

        let content = std::fs::read(path).with_context(|| format!("Read {}", path.display()))?;
        let hash = content_hash(&content);
        files.insert(normalized, (content, hash));
    }

    Ok(files)
}

/// Scan remote files via API.
pub async fn scan_remote(client: &RemoteClient) -> Result<HashMap<String, RemoteFileEntry>> {
    let entries = client.list("/", true).await?;
    let mut files = HashMap::new();
    for entry in entries {
        if !entry.is_directory {
            let normalized = entry.path.trim_start_matches('/').to_string();
            files.insert(normalized, entry);
        }
    }
    Ok(files)
}

/// Run a full sync cycle: reconcile local and remote, apply changes.
#[allow(clippy::too_many_arguments)]
pub async fn reconcile(
    client: &RemoteClient,
    local_dir: &Path,
    state: &mut SyncState,
    conflict_strategy: Conflict,
    no_gitignore: bool,
    extra_excludes: &[String],
    dry_run: bool,
    delete: bool,
    verbose: bool,
) -> Result<SyncStats> {
    let mut stats = SyncStats::default();

    let local_files = scan_local(local_dir, no_gitignore, extra_excludes)?;
    let remote_files = scan_remote(client).await?;

    let all_paths: HashSet<&str> = local_files
        .keys()
        .chain(remote_files.keys())
        .map(String::as_str)
        .collect();

    for path in all_paths {
        let local = local_files.get(path);
        let remote = remote_files.get(path);
        let prev = state.files.get(path);

        match (local, remote) {
            (Some((local_content, local_hash)), Some(remote_entry)) => {
                let prev_local = prev.and_then(|p| p.local_hash.as_deref());
                let prev_remote = prev.and_then(|p| p.remote_hash.as_deref());
                let remote_hash = remote_entry.content_hash.as_deref().unwrap_or("");

                let local_changed = prev_local.is_none_or(|h| h != local_hash);
                let remote_changed = prev_remote.is_none_or(|h| h != remote_hash);

                if !local_changed && !remote_changed {
                    stats.skipped += 1;
                    continue;
                }

                if local_changed && !remote_changed {
                    if verbose {
                        eprintln!("  ↑ {}", path);
                    }
                    if !dry_run
                        && let Err(e) = client
                            .write_file(&format!("/{}", path), local_content, false)
                            .await
                    {
                        eprintln!("  x upload {}: {}", path, e);
                        stats.errors += 1;
                        continue;
                    }
                    stats.uploaded += 1;
                    update_state(state, path, Some(local_hash), Some(remote_hash));
                } else if !local_changed && remote_changed {
                    if verbose {
                        eprintln!("  ↓ {}", path);
                    }
                    if !dry_run {
                        match download_file(client, local_dir, path).await {
                            Ok(hash) => {
                                update_state(state, path, Some(&hash), Some(remote_hash));
                            }
                            Err(e) => {
                                eprintln!("  x download {}: {}", path, e);
                                stats.errors += 1;
                                continue;
                            }
                        }
                    }
                    stats.downloaded += 1;
                } else {
                    // Both changed — conflict
                    stats.conflicts += 1;
                    let winner = resolve_conflict(conflict_strategy, local_dir, path, remote_entry);
                    eprintln!("  ! conflict: {} ({} wins)", path, winner);

                    if winner == "local" {
                        if !dry_run
                            && let Err(e) = client
                                .write_file(&format!("/{}", path), local_content, false)
                                .await
                        {
                            eprintln!("  x upload {}: {}", path, e);
                            stats.errors += 1;
                            continue;
                        }
                        stats.uploaded += 1;
                    } else {
                        if !dry_run {
                            match download_file(client, local_dir, path).await {
                                Ok(hash) => {
                                    update_state(state, path, Some(&hash), Some(remote_hash));
                                }
                                Err(e) => {
                                    eprintln!("  x download {}: {}", path, e);
                                    stats.errors += 1;
                                    continue;
                                }
                            }
                        }
                        stats.downloaded += 1;
                    }
                }
            }

            (Some((local_content, local_hash)), None) => {
                let was_synced = prev.is_some();
                if was_synced && delete {
                    if verbose {
                        eprintln!("  del local {}", path);
                    }
                    if !dry_run {
                        let local_path = local_dir.join(path);
                        let _ = std::fs::remove_file(&local_path);
                    }
                    state.files.remove(path);
                    stats.deleted_local += 1;
                } else {
                    if verbose {
                        eprintln!("  ↑ {}", path);
                    }
                    if !dry_run
                        && let Err(e) = client
                            .write_file(&format!("/{}", path), local_content, true)
                            .await
                    {
                        eprintln!("  x upload {}: {}", path, e);
                        stats.errors += 1;
                        continue;
                    }
                    stats.uploaded += 1;
                    update_state(state, path, Some(local_hash), None);
                }
            }

            (None, Some(remote_entry)) => {
                let was_synced = prev.is_some();
                let remote_hash = remote_entry.content_hash.as_deref().unwrap_or("");

                if was_synced && delete {
                    if verbose {
                        eprintln!("  del remote {}", path);
                    }
                    if !dry_run {
                        let _ = client.delete(&format!("/{}", path), false).await;
                    }
                    state.files.remove(path);
                    stats.deleted_remote += 1;
                } else {
                    if verbose {
                        eprintln!("  ↓ {}", path);
                    }
                    if !dry_run {
                        match download_file(client, local_dir, path).await {
                            Ok(hash) => {
                                update_state(state, path, Some(&hash), Some(remote_hash));
                            }
                            Err(e) => {
                                eprintln!("  x download {}: {}", path, e);
                                stats.errors += 1;
                                continue;
                            }
                        }
                    }
                    stats.downloaded += 1;
                }
            }

            (None, None) => unreachable!(),
        }
    }

    state.last_sync = Some(chrono::Utc::now().to_rfc3339());

    if !dry_run {
        let sd = state_dir(local_dir);
        state.save(&sd)?;
    }

    Ok(stats)
}

fn resolve_conflict(
    strategy: Conflict,
    local_dir: &Path,
    path: &str,
    remote_entry: &RemoteFileEntry,
) -> &'static str {
    match strategy {
        Conflict::Local => "local",
        Conflict::Remote => "remote",
        Conflict::LastWrite => {
            let local_mtime = std::fs::metadata(local_dir.join(path))
                .ok()
                .and_then(|m| m.modified().ok());
            let remote_time = remote_entry
                .updated_at
                .as_deref()
                .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
                .map(|t| {
                    std::time::SystemTime::UNIX_EPOCH
                        + std::time::Duration::from_secs(t.timestamp() as u64)
                });

            match (local_mtime, remote_time) {
                (Some(l), Some(r)) if l > r => "local",
                (Some(_), Some(_)) => "remote",
                _ => "local", // tie-break: local wins
            }
        }
    }
}

async fn download_file(client: &RemoteClient, local_dir: &Path, path: &str) -> Result<String> {
    let remote_content = client.read_file(&format!("/{}", path)).await?;
    let bytes = RemoteClient::decode_content(&remote_content)?;
    let local_path = local_dir.join(path);

    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&local_path, &bytes)?;
    Ok(content_hash(&bytes))
}

fn update_state(
    state: &mut SyncState,
    path: &str,
    local_hash: Option<&str>,
    remote_hash: Option<&str>,
) {
    let entry = state
        .files
        .entry(path.to_string())
        .or_insert_with(|| FileSyncState {
            local_hash: None,
            remote_hash: None,
            local_mtime: None,
            remote_updated_at: None,
        });
    if let Some(h) = local_hash {
        entry.local_hash = Some(h.to_string());
    }
    if let Some(h) = remote_hash {
        entry.remote_hash = Some(h.to_string());
    }
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
