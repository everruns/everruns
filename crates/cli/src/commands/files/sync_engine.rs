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
                // Use content_hash if available, fall back to updated_at for change detection
                let remote_hash = remote_entry
                    .content_hash
                    .as_deref()
                    .or(remote_entry.updated_at.as_deref())
                    .unwrap_or("");

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
                    if !dry_run && let Ok(local_path) = safe_local_path(local_dir, path) {
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
                let remote_hash = remote_entry
                    .content_hash
                    .as_deref()
                    .or(remote_entry.updated_at.as_deref())
                    .unwrap_or("");

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

/// Validate that a path joined with local_dir stays within local_dir (prevents path traversal).
pub fn safe_local_path(local_dir: &Path, path: &str) -> Result<std::path::PathBuf> {
    // Reject absolute paths and traversal components
    if path.starts_with('/') || path.starts_with('\\') || path.contains("..") {
        anyhow::bail!("Unsafe path rejected (traversal or absolute): {}", path);
    }
    let joined = local_dir.join(path);
    // Normalize and verify the result is under local_dir
    // Use lexical check since the file may not exist yet
    let normalized = joined
        .components()
        .fold(std::path::PathBuf::new(), |mut acc, c| {
            match c {
                std::path::Component::ParentDir => {
                    acc.pop();
                }
                std::path::Component::CurDir => {}
                _ => acc.push(c),
            }
            acc
        });
    if !normalized.starts_with(local_dir) {
        anyhow::bail!(
            "Path escapes sync directory: {} -> {}",
            path,
            normalized.display()
        );
    }
    Ok(joined)
}

async fn download_file(client: &RemoteClient, local_dir: &Path, path: &str) -> Result<String> {
    let remote_content = client.read_file(&format!("/{}", path)).await?;
    let bytes = RemoteClient::decode_content(&remote_content)?;
    let local_path = safe_local_path(local_dir, path)?;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_conflict_parse() {
        assert!(matches!(Conflict::parse("local-wins"), Conflict::Local));
        assert!(matches!(Conflict::parse("remote-wins"), Conflict::Remote));
        assert!(matches!(
            Conflict::parse("last-write-wins"),
            Conflict::LastWrite
        ));
        assert!(matches!(Conflict::parse("unknown"), Conflict::LastWrite));
        assert!(matches!(Conflict::parse(""), Conflict::LastWrite));
    }

    #[test]
    fn test_sync_stats_display_minimal() {
        let stats = SyncStats {
            uploaded: 2,
            downloaded: 1,
            ..Default::default()
        };
        assert_eq!(format!("{}", stats), "↑2 ↓1");
    }

    #[test]
    fn test_sync_stats_display_with_deletes() {
        let stats = SyncStats {
            uploaded: 0,
            downloaded: 0,
            deleted_local: 1,
            deleted_remote: 2,
            ..Default::default()
        };
        assert_eq!(format!("{}", stats), "↑0 ↓0 del:3");
    }

    #[test]
    fn test_sync_stats_display_full() {
        let stats = SyncStats {
            uploaded: 5,
            downloaded: 3,
            deleted_local: 1,
            deleted_remote: 0,
            conflicts: 2,
            skipped: 10,
            errors: 1,
        };
        assert_eq!(format!("{}", stats), "↑5 ↓3 del:1 conflicts:2 errors:1");
    }

    #[test]
    fn test_sync_stats_display_zero() {
        let stats = SyncStats::default();
        assert_eq!(format!("{}", stats), "↑0 ↓0");
    }

    #[test]
    fn test_normalize_path_unix() {
        assert_eq!(normalize_path(Path::new("src/main.rs")), "src/main.rs");
    }

    #[test]
    fn test_normalize_path_nested() {
        assert_eq!(normalize_path(Path::new("a/b/c/d.txt")), "a/b/c/d.txt");
    }

    #[test]
    fn test_scan_local_basic() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hello.txt"), "hello").unwrap();
        fs::create_dir_all(dir.path().join("src")).unwrap();
        fs::write(dir.path().join("src/main.rs"), "fn main() {}").unwrap();

        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.contains_key("hello.txt"));
        assert!(files.contains_key("src/main.rs"));
        assert_eq!(files.len(), 2);

        // Verify content and hash
        let (content, hash) = &files["hello.txt"];
        assert_eq!(content, b"hello");
        assert!(hash.starts_with("sha256:"));
    }

    #[test]
    fn test_scan_local_excludes_default_dirs() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();
        fs::write(dir.path().join(".git/config"), "gitconfig").unwrap();
        fs::create_dir_all(dir.path().join("node_modules")).unwrap();
        fs::write(dir.path().join("node_modules/pkg.js"), "module").unwrap();
        fs::create_dir_all(dir.path().join("target")).unwrap();
        fs::write(dir.path().join("target/debug"), "binary").unwrap();
        fs::create_dir_all(dir.path().join(".everruns-sync")).unwrap();
        fs::write(dir.path().join(".everruns-sync/state.json"), "{}").unwrap();

        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.contains_key("keep.txt"));
        assert!(!files.contains_key(".git/config"));
        assert!(!files.contains_key("node_modules/pkg.js"));
        assert!(!files.contains_key("target/debug"));
        assert!(!files.contains_key(".everruns-sync/state.json"));
    }

    #[test]
    fn test_scan_local_extra_excludes() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep").unwrap();
        fs::create_dir_all(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build/out.js"), "output").unwrap();

        let files = scan_local(dir.path(), false, &["build".to_string()]).unwrap();
        assert!(files.contains_key("keep.txt"));
        assert!(!files.contains_key("build/out.js"));
    }

    #[test]
    fn test_scan_local_syncignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep").unwrap();
        fs::write(dir.path().join("secret.key"), "secret").unwrap();
        fs::write(dir.path().join(".syncignore"), "*.key\n# comment\n").unwrap();

        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.contains_key("keep.txt"));
        assert!(!files.contains_key("secret.key"));
    }

    #[test]
    fn test_scan_local_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn test_resolve_conflict_local_wins() {
        let entry = RemoteFileEntry {
            path: "/test.txt".to_string(),
            is_directory: false,
            size_bytes: 5,
            content_hash: None,
            updated_at: Some("2026-01-01T00:00:00Z".to_string()),
            is_readonly: false,
        };
        let result = resolve_conflict(Conflict::Local, Path::new("/tmp"), "test.txt", &entry);
        assert_eq!(result, "local");
    }

    #[test]
    fn test_resolve_conflict_remote_wins() {
        let entry = RemoteFileEntry {
            path: "/test.txt".to_string(),
            is_directory: false,
            size_bytes: 5,
            content_hash: None,
            updated_at: Some("2026-01-01T00:00:00Z".to_string()),
            is_readonly: false,
        };
        let result = resolve_conflict(Conflict::Remote, Path::new("/tmp"), "test.txt", &entry);
        assert_eq!(result, "remote");
    }

    #[test]
    fn test_update_state_new_entry() {
        let mut state = SyncState::new("ses_test");
        update_state(
            &mut state,
            "file.txt",
            Some("sha256:aaa"),
            Some("sha256:bbb"),
        );
        let entry = &state.files["file.txt"];
        assert_eq!(entry.local_hash.as_deref(), Some("sha256:aaa"));
        assert_eq!(entry.remote_hash.as_deref(), Some("sha256:bbb"));
    }

    #[test]
    fn test_update_state_partial_update() {
        let mut state = SyncState::new("ses_test");
        update_state(&mut state, "file.txt", Some("sha256:aaa"), None);
        let entry = &state.files["file.txt"];
        assert_eq!(entry.local_hash.as_deref(), Some("sha256:aaa"));
        assert!(entry.remote_hash.is_none());

        // Now update remote only
        update_state(&mut state, "file.txt", None, Some("sha256:bbb"));
        let entry = &state.files["file.txt"];
        assert_eq!(entry.local_hash.as_deref(), Some("sha256:aaa")); // unchanged
        assert_eq!(entry.remote_hash.as_deref(), Some("sha256:bbb"));
    }

    #[test]
    fn test_safe_local_path_normal() {
        let dir = tempfile::tempdir().unwrap();
        let result = safe_local_path(dir.path(), "src/main.rs").unwrap();
        assert_eq!(result, dir.path().join("src/main.rs"));
    }

    #[test]
    fn test_safe_local_path_rejects_traversal() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_local_path(dir.path(), "../../etc/passwd").is_err());
        assert!(safe_local_path(dir.path(), "../secret").is_err());
        assert!(safe_local_path(dir.path(), "a/../../b").is_err());
    }

    #[test]
    fn test_safe_local_path_rejects_absolute() {
        let dir = tempfile::tempdir().unwrap();
        assert!(safe_local_path(dir.path(), "/etc/passwd").is_err());
    }
}
