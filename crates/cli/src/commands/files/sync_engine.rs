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

    let ignore_path = local_dir.join(".everrunsignore");
    let legacy_path = local_dir.join(".syncignore");
    if !ignore_path.exists() && legacy_path.exists() {
        eprintln!("warning: .syncignore is deprecated, rename to .everrunsignore");
    }
    // Prefer .everrunsignore, fall back to legacy .syncignore
    let active_ignore = if ignore_path.exists() {
        Some(&ignore_path)
    } else if legacy_path.exists() {
        Some(&legacy_path)
    } else {
        None
    };
    if let Some(path) = active_ignore
        && let Ok(content) = std::fs::read_to_string(path)
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

        // The walker does not follow links, so this is the entry's own type.
        // Only regular files are uploaded: symlinks (to files or dirs, inside
        // or outside the workspace), sockets, FIFOs and devices are skipped.
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            continue;
        }
        if !file_type.is_file() {
            if file_type.is_symlink() {
                eprintln!("warning: skipping symlink {}", path.display());
            }
            continue;
        }

        let rel = path.strip_prefix(local_dir).context("Strip local prefix")?;
        let normalized = normalize_path(rel);

        // Re-check at open time: the entry may have been swapped since the walk.
        let Some(content) = read_workspace_file(local_dir, rel)
            .with_context(|| format!("Read {}", path.display()))?
        else {
            eprintln!("warning: skipping non-regular file {}", path.display());
            continue;
        };
        let hash = content_hash(&content);
        files.insert(normalized, (content, hash));
    }

    Ok(files)
}

/// Read a regular file at `rel` under `base` without following symlinks in
/// any component of `rel`. Returns `Ok(None)` when a component is a symlink or
/// the final entry is not a regular file.
///
/// Design Decision (EVE-1191): upload must never send bytes from outside the
/// workspace. A walk-time file-type check alone is racy (an entry or a parent
/// directory can be swapped for a symlink before the read), so on Unix each
/// component is opened relative to its parent's descriptor with `O_NOFOLLOW`,
/// and the final type check uses `fstat` on the opened handle.
// THREAT[TM-FS-020]: no-follow reads keep uploads inside the workspace.
#[cfg(unix)]
fn read_workspace_file(base: &Path, rel: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use rustix::fs::{Mode, OFlags, openat};
    use rustix::io::Errno;
    use std::io::Read;
    use std::os::fd::OwnedFd;

    let parts = workspace_components(rel)?;
    let Some((last, parents)) = parts.split_last() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "empty workspace path",
        ));
    };
    // ELOOP: final symlink under O_NOFOLLOW; EMLINK: same on FreeBSD;
    // ENOTDIR: a parent component is a symlink (or file) under O_DIRECTORY.
    let rejected = |e: Errno| matches!(e, Errno::LOOP | Errno::MLINK | Errno::NOTDIR);

    // The workspace root itself was chosen by the user and may be a symlink.
    let mut dir: OwnedFd = std::fs::File::open(base)?.into();
    let dir_flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC;
    for part in parents {
        dir = match openat(&dir, *part, dir_flags, Mode::empty()) {
            Ok(fd) => fd,
            Err(e) if rejected(e) => return Ok(None),
            Err(e) => return Err(e.into()),
        };
    }
    // NONBLOCK so a FIFO swapped in cannot hang the open; NOCTTY for devices.
    let file_flags =
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK | OFlags::NOCTTY;
    let fd = match openat(&dir, *last, file_flags, Mode::empty()) {
        Ok(fd) => fd,
        Err(e) if rejected(e) => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut file = std::fs::File::from(fd);
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Ok(None);
    }
    let mut content = Vec::with_capacity(usize::try_from(meta.len()).unwrap_or(0));
    file.read_to_end(&mut content)?;
    Ok(Some(content))
}

/// Non-Unix fallback: reject any symlinked component, then confirm the opened
/// handle is a regular file. Narrower race window than Unix, not race-free.
#[cfg(not(unix))]
fn read_workspace_file(base: &Path, rel: &Path) -> std::io::Result<Option<Vec<u8>>> {
    use std::io::Read;

    let mut current = base.to_path_buf();
    for part in workspace_components(rel)? {
        current.push(part);
        if std::fs::symlink_metadata(&current)?
            .file_type()
            .is_symlink()
        {
            return Ok(None);
        }
    }
    let mut file = std::fs::File::open(&current)?;
    let meta = file.metadata()?;
    if !meta.file_type().is_file() {
        return Ok(None);
    }
    let mut content = Vec::new();
    file.read_to_end(&mut content)?;
    Ok(Some(content))
}

/// Split a walker-relative path into plain components, refusing anything that
/// could escape the base (`..`, absolute, prefixes).
fn workspace_components(rel: &Path) -> std::io::Result<Vec<&std::ffi::OsStr>> {
    rel.components()
        .filter(|c| !matches!(c, std::path::Component::CurDir))
        .map(|c| match c {
            std::path::Component::Normal(part) => Ok(part),
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("non-workspace path component in {}", rel.display()),
            )),
        })
        .collect()
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
                        update_state(state, path, Some(local_hash), Some(remote_hash));
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
                    // Use local hash as remote_hash so next cycle won't
                    // treat the remote as changed (server may update hash).
                    update_state(state, path, Some(local_hash), Some(local_hash));
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
    let base_dir = local_dir
        .canonicalize()
        .unwrap_or_else(|_| local_dir.to_path_buf());
    let mut relative = std::path::PathBuf::new();

    for component in std::path::Path::new(path).components() {
        match component {
            std::path::Component::Normal(part) => relative.push(part),
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {
                anyhow::bail!("Unsafe path rejected (traversal or absolute): {}", path);
            }
        }
    }

    let joined = base_dir.join(&relative);
    if !joined.starts_with(&base_dir) {
        anyhow::bail!(
            "Path escapes sync directory: {} -> {}",
            path,
            joined.display()
        );
    }

    let mut current = base_dir.clone();
    for component in relative.components() {
        current.push(component.as_os_str());
        if let Ok(meta) = std::fs::symlink_metadata(&current)
            && meta.file_type().is_symlink()
        {
            anyhow::bail!("Unsafe path rejected (symlink component): {}", path);
        }
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
    fn test_normalize_path_nested() {
        // Already-forward-slash paths pass through unchanged...
        assert_eq!(normalize_path(Path::new("a/b/c/d.txt")), "a/b/c/d.txt");
        // ...and literal backslashes (as produced by Windows path components) are converted.
        assert_eq!(normalize_path(Path::new("a\\b\\c")), "a/b/c");
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
    fn test_scan_local_everrunsignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep").unwrap();
        fs::write(dir.path().join("secret.key"), "secret").unwrap();
        fs::write(dir.path().join(".everrunsignore"), "*.key\n# comment\n").unwrap();

        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.contains_key("keep.txt"));
        assert!(!files.contains_key("secret.key"));
    }

    #[test]
    fn test_scan_local_gitignore_respected() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep me").unwrap();
        fs::create_dir_all(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build/output.js"), "compiled").unwrap();
        fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
        // WalkBuilder only honors .gitignore inside a git repo.
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.contains_key("keep.txt"));
        assert!(files.contains_key(".gitignore"));
        assert!(
            !files.contains_key("build/output.js"),
            "build/ should be gitignored, found: {:?}",
            files.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_scan_local_no_gitignore_flag_disables_gitignore() {
        let dir = tempfile::tempdir().unwrap();
        fs::create_dir_all(dir.path().join("build")).unwrap();
        fs::write(dir.path().join("build/output.js"), "compiled").unwrap();
        fs::write(dir.path().join(".gitignore"), "build/\n").unwrap();
        fs::create_dir_all(dir.path().join(".git")).unwrap();

        // no_gitignore=true must bypass .gitignore rules.
        let files = scan_local(dir.path(), true, &[]).unwrap();
        assert!(files.contains_key("build/output.js"));
    }

    #[test]
    fn test_scan_local_legacy_syncignore_fallback() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep").unwrap();
        fs::write(dir.path().join("secret.key"), "secret").unwrap();
        // Legacy .syncignore should still be respected as fallback
        fs::write(dir.path().join(".syncignore"), "*.key\n").unwrap();

        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.contains_key("keep.txt"));
        assert!(!files.contains_key("secret.key"));
    }

    #[test]
    fn test_scan_local_everrunsignore_takes_precedence() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("keep.txt"), "keep").unwrap();
        fs::write(dir.path().join("a.key"), "secret").unwrap();
        fs::write(dir.path().join("b.log"), "log").unwrap();
        // Both files exist — .everrunsignore should win
        fs::write(dir.path().join(".syncignore"), "*.log\n").unwrap();
        fs::write(dir.path().join(".everrunsignore"), "*.key\n").unwrap();

        let files = scan_local(dir.path(), false, &[]).unwrap();
        assert!(files.contains_key("keep.txt"));
        assert!(!files.contains_key("a.key")); // excluded by .everrunsignore
        assert!(files.contains_key("b.log")); // NOT excluded (.syncignore ignored)
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
        assert_eq!(
            result,
            dir.path().canonicalize().unwrap().join("src/main.rs")
        );
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

    // EVE-1191: a symlink inside the workspace must never cause bytes from
    // outside the workspace to be uploaded.
    #[cfg(unix)]
    mod symlink_upload {
        use super::*;
        use crate::commands::files::test_server::FsApiStub;
        use std::os::unix::fs::symlink;

        const SECRET: &str = "TOP-SECRET-OUTSIDE-WORKSPACE";

        fn outside_secret() -> (tempfile::TempDir, std::path::PathBuf) {
            let outside = tempfile::tempdir().unwrap();
            let secret = outside.path().join("id_rsa");
            fs::write(&secret, SECRET).unwrap();
            (outside, secret)
        }

        fn assert_no_secret(stub: &FsApiStub) {
            for (path, body) in stub.uploads() {
                assert!(
                    !body.contains(SECRET),
                    "outside bytes uploaded via {path}: {body}"
                );
            }
        }

        #[test]
        fn scan_local_skips_symlink_to_outside_file() {
            let dir = tempfile::tempdir().unwrap();
            let (_outside, secret) = outside_secret();
            fs::write(dir.path().join("keep.txt"), "keep").unwrap();
            symlink(&secret, dir.path().join("leak.txt")).unwrap();

            let files = scan_local(dir.path(), false, &[]).unwrap();
            assert!(files.contains_key("keep.txt"));
            assert!(!files.contains_key("leak.txt"));
            assert_eq!(files.len(), 1);
        }

        #[test]
        fn scan_local_skips_symlink_to_inside_file() {
            // Even in-workspace links are rejected: the target can be swapped.
            let dir = tempfile::tempdir().unwrap();
            fs::write(dir.path().join("real.txt"), "real").unwrap();
            symlink(dir.path().join("real.txt"), dir.path().join("alias.txt")).unwrap();

            let files = scan_local(dir.path(), false, &[]).unwrap();
            assert!(files.contains_key("real.txt"));
            assert!(!files.contains_key("alias.txt"));
        }

        #[test]
        fn scan_local_skips_symlinked_directory() {
            let dir = tempfile::tempdir().unwrap();
            let (outside, _secret) = outside_secret();
            symlink(outside.path(), dir.path().join("linked")).unwrap();

            let files = scan_local(dir.path(), false, &[]).unwrap();
            assert!(
                files.is_empty(),
                "got {:?}",
                files.keys().collect::<Vec<_>>()
            );
        }

        #[test]
        fn read_rejects_final_symlink_swapped_in_after_scan() {
            // Simulates a file replaced by a symlink between walk and open.
            let dir = tempfile::tempdir().unwrap();
            let (_outside, secret) = outside_secret();
            symlink(&secret, dir.path().join("a.txt")).unwrap();

            let read = read_workspace_file(dir.path(), Path::new("a.txt")).unwrap();
            assert!(read.is_none());
        }

        #[test]
        fn read_rejects_parent_dir_swapped_for_symlink_after_scan() {
            // Simulates `sub/` replaced by a symlink to an outside directory
            // between walk and open: the parent component must not be followed.
            let dir = tempfile::tempdir().unwrap();
            let (outside, _secret) = outside_secret();
            symlink(outside.path(), dir.path().join("sub")).unwrap();

            let read = read_workspace_file(dir.path(), Path::new("sub/id_rsa")).unwrap();
            assert!(read.is_none());
        }

        #[test]
        fn read_returns_regular_file_bytes() {
            let dir = tempfile::tempdir().unwrap();
            fs::create_dir_all(dir.path().join("a/b")).unwrap();
            fs::write(dir.path().join("a/b/c.txt"), "ok").unwrap();

            let read = read_workspace_file(dir.path(), Path::new("a/b/c.txt")).unwrap();
            assert_eq!(read.as_deref(), Some(&b"ok"[..]));
        }

        #[tokio::test]
        async fn push_does_not_upload_symlink_target() {
            let stub = FsApiStub::start().await;
            let dir = tempfile::tempdir().unwrap();
            let (_outside, secret) = outside_secret();
            fs::write(dir.path().join("keep.txt"), "keep").unwrap();
            symlink(&secret, dir.path().join("leak.txt")).unwrap();

            crate::commands::files::push::run(
                &stub.url,
                "key",
                None,
                crate::output::OutputFormat::Json,
                true,
                "ses_test".to_string(),
                dir.path().to_string_lossy().into_owned(),
                false,
                false,
            )
            .await
            .unwrap();

            let uploads = stub.uploads();
            assert!(uploads.iter().any(|(p, _)| p.ends_with("/keep.txt")));
            assert!(!uploads.iter().any(|(p, _)| p.ends_with("/leak.txt")));
            assert_no_secret(&stub);
        }

        #[tokio::test]
        async fn live_sync_ignores_symlink_added_after_start() {
            let stub = FsApiStub::start().await;
            let client = RemoteClient::new(&stub.url, "key", "ses_test");
            let dir = tempfile::tempdir().unwrap();
            let (_outside, secret) = outside_secret();
            fs::write(dir.path().join("keep.txt"), "keep").unwrap();
            let mut state = SyncState::new("ses_test");

            let first = reconcile(
                &client,
                dir.path(),
                &mut state,
                Conflict::LastWrite,
                false,
                &[],
                false,
                false,
                false,
            )
            .await
            .unwrap();
            assert_eq!(first.uploaded, 1);

            // A symlink to an outside secret appears while sync is running.
            symlink(&secret, dir.path().join("leak.txt")).unwrap();
            let second = reconcile(
                &client,
                dir.path(),
                &mut state,
                Conflict::LastWrite,
                false,
                &[],
                false,
                false,
                false,
            )
            .await
            .unwrap();
            // The stub's listing stays empty, so keep.txt re-uploads each cycle.
            assert_eq!(second.uploaded, 1);
            assert!(!state.files.contains_key("leak.txt"));
            assert!(!stub.uploads().iter().any(|(p, _)| p.ends_with("/leak.txt")));
            assert_no_secret(&stub);
        }
    }

    #[test]
    fn test_safe_local_path_rejects_symlink_component() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(outside.path(), &link).unwrap();
        #[cfg(windows)]
        std::os::windows::fs::symlink_dir(outside.path(), &link).unwrap();

        assert!(safe_local_path(dir.path(), "link/secret.txt").is_err());
    }
}
