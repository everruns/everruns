// Session Git service — libgit2 with PostgreSQL-backed object storage
//
// Strategy: mempack hydrate/drain
//   1. Load git objects + refs from PG (async, single query each)
//   2. In spawn_blocking: create ODB, hydrate, perform git op, collect results
//   3. Persist new objects + updated refs back to PG (async)
//
// git2::Repository is Send but not Sync, so all libgit2 work happens inside
// spawn_blocking to avoid holding non-Sync references across .await points.

use crate::storage::{
    StorageBackend,
    models::{CreateSessionGitObject, CreateSessionGitRef, SessionGitObjectRow},
};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use git2::{ObjectType, Oid, Repository, Signature};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::debug;
use utoipa::ToSchema;
use uuid::Uuid;

/// A commit entry in the log
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitCommitInfo {
    pub oid: String,
    pub message: String,
    pub author_name: String,
    pub author_email: String,
    pub timestamp: DateTime<Utc>,
    pub parent_oids: Vec<String>,
}

/// A diff entry between two commits
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitDiffEntry {
    pub path: String,
    pub status: String, // "added", "modified", "deleted", "renamed"
    pub old_path: Option<String>,
}

/// A diff with full patch output
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitDiff {
    pub entries: Vec<GitDiffEntry>,
    pub patch: Option<String>,
    pub stats: GitDiffStats,
}

/// Diff statistics
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitDiffStats {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

/// A git ref (branch pointer)
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GitRefInfo {
    pub name: String,
    pub target: String, // hex OID
    pub is_symbolic: bool,
}

/// Result of a commit operation
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CommitResult {
    pub oid: String,
    pub tree_oid: String,
    pub objects_created: usize,
}

pub struct SessionGitService {
    db: Arc<StorageBackend>,
}

/// Internal: new objects produced by a git operation (to be persisted to PG)
struct DrainResult {
    new_objects: Vec<CreateSessionGitObject>,
    /// (ref_name, oid_bytes) pairs to upsert
    ref_updates: Vec<(String, Vec<u8>)>,
}

/// Normalize a branch name to refs/heads/ form.
fn normalize_branch(name: &str) -> String {
    if name == "HEAD" || name.starts_with("refs/") {
        name.to_string()
    } else {
        format!("refs/heads/{}", name)
    }
}

impl SessionGitService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    // ========================================================
    // PG helpers (async)
    // ========================================================

    async fn load_ref_oid(&self, session_id: Uuid, name: &str) -> Result<Option<Vec<u8>>> {
        let r = self.db.read_git_ref(session_id, name).await?;
        match r {
            Some(row) if !row.is_symbolic => Ok(Some(row.target)),
            _ => Ok(None),
        }
    }

    async fn persist_drain(&self, session_id: Uuid, drain: DrainResult) -> Result<usize> {
        let count = drain.new_objects.len();
        if !drain.new_objects.is_empty() {
            self.db.write_git_objects_batch(drain.new_objects).await?;
        }
        let session_id_typed = everruns_core::SessionId::from_uuid(session_id);
        for (name, target) in &drain.ref_updates {
            self.db
                .write_git_ref(CreateSessionGitRef {
                    session_id: session_id_typed,
                    name: name.clone(),
                    target: target.clone(),
                    is_symbolic: false,
                })
                .await?;
        }
        debug!(session_id = %session_id, new_objects = count, refs = drain.ref_updates.len(), "persisted git drain");
        Ok(count)
    }

    // ========================================================
    // Public API: commit
    // ========================================================

    pub async fn commit(
        &self,
        session_id: Uuid,
        message: &str,
        author_name: &str,
        author_email: &str,
        branch: Option<&str>,
    ) -> Result<CommitResult> {
        // Normalize branch name (fix: accept short names like "main")
        let branch_name = normalize_branch(branch.unwrap_or("refs/heads/main"));

        // 1. Load existing objects in one query (fix: N+1)
        let existing_objects = self.db.load_all_git_objects(session_id).await?;
        let parent_oid_bytes = self.load_ref_oid(session_id, &branch_name).await?;

        // 2. Load all session files with content in one query (fix: N+1)
        let all_files = self
            .db
            .load_all_session_files_with_content(session_id)
            .await?;
        let file_contents: Vec<(String, Vec<u8>)> = all_files
            .into_iter()
            .map(|f| {
                let git_path = f.path.strip_prefix('/').unwrap_or(&f.path).to_string();
                // Fix: treat None content as empty blob so empty files are committed
                let content = f.content.unwrap_or_default();
                (git_path, content)
            })
            .collect();

        // 3. All git work in spawn_blocking (libgit2 is not Sync)
        let message = message.to_string();
        let author_name = author_name.to_string();
        let author_email = author_email.to_string();
        let session_id_typed = everruns_core::SessionId::from_uuid(session_id);

        let (result, drain) =
            tokio::task::spawn_blocking(move || -> Result<(CommitResult, DrainResult)> {
                // Hydrate ODB
                let (repo, known_oids) = hydrate_odb(&existing_objects)?;
                let odb = repo.odb()?;

                // Track every OID we create so we can drain
                let mut created_oids: Vec<Oid> = Vec::new();

                // Build blobs
                let mut tree_entries: Vec<(String, Oid)> = Vec::new();
                for (path, content) in &file_contents {
                    let blob_oid = odb.write(ObjectType::Blob, content)?;
                    if !known_oids.contains(blob_oid.as_bytes()) {
                        created_oids.push(blob_oid);
                    }
                    tree_entries.push((path.clone(), blob_oid));
                }

                // Build tree
                let tree_oid = build_tree_from_paths_tracking(
                    &repo,
                    &tree_entries,
                    &known_oids,
                    &mut created_oids,
                )?;

                // Parent commit
                let parent_commit = match &parent_oid_bytes {
                    Some(bytes) => {
                        let oid = oid_from_bytes(bytes)?;
                        Some(repo.find_commit(oid)?)
                    }
                    None => None,
                };

                // Create commit
                let tree = repo.find_tree(tree_oid)?;
                let sig = Signature::now(&author_name, &author_email)?;
                let parents: Vec<&git2::Commit<'_>> = parent_commit.iter().collect();
                let commit_oid = repo.commit(None, &sig, &sig, &message, &tree, &parents)?;
                created_oids.push(commit_oid);

                // Read created objects from ODB to build drain payload
                let new_objects = read_objects_by_oid(&odb, &created_oids, session_id_typed)?;
                let objects_created = new_objects.len();

                // Ref updates
                let commit_oid_vec = commit_oid.as_bytes().to_vec();
                let ref_updates = vec![
                    (branch_name, commit_oid_vec.clone()),
                    ("HEAD".to_string(), commit_oid_vec),
                ];

                Ok((
                    CommitResult {
                        oid: commit_oid.to_string(),
                        tree_oid: tree_oid.to_string(),
                        objects_created,
                    },
                    DrainResult {
                        new_objects,
                        ref_updates,
                    },
                ))
            })
            .await??;

        // 4. Persist to PG
        self.persist_drain(session_id, drain).await?;

        Ok(result)
    }

    // ========================================================
    // Public API: log
    // ========================================================

    pub async fn log(
        &self,
        session_id: Uuid,
        from_ref: Option<&str>,
        limit: Option<usize>,
    ) -> Result<Vec<GitCommitInfo>> {
        let ref_name = from_ref.unwrap_or("HEAD");
        let start_oid_bytes = self
            .load_ref_oid(session_id, ref_name)
            .await?
            .ok_or_else(|| anyhow!("ref '{}' not found", ref_name))?;

        let existing_objects = self.db.load_all_git_objects(session_id).await?;
        let max = limit.unwrap_or(100);

        tokio::task::spawn_blocking(move || -> Result<Vec<GitCommitInfo>> {
            let (repo, _) = hydrate_odb(&existing_objects)?;
            let start_oid = oid_from_bytes(&start_oid_bytes)?;

            let mut revwalk = repo.revwalk()?;
            revwalk.push(start_oid)?;
            revwalk.set_sorting(git2::Sort::TIME)?;

            let mut commits = Vec::new();
            for oid_result in revwalk {
                if commits.len() >= max {
                    break;
                }
                let oid = oid_result?;
                let commit = repo.find_commit(oid)?;
                let author = commit.author();
                let time = commit.time();
                let timestamp = DateTime::from_timestamp(time.seconds(), 0).unwrap_or_default();

                commits.push(GitCommitInfo {
                    oid: oid.to_string(),
                    message: commit.message().unwrap_or("").to_string(),
                    author_name: author.name().unwrap_or("").to_string(),
                    author_email: author.email().unwrap_or("").to_string(),
                    timestamp,
                    parent_oids: commit.parent_ids().map(|id| id.to_string()).collect(),
                });
            }
            Ok(commits)
        })
        .await?
    }

    // ========================================================
    // Public API: diff
    // ========================================================

    pub async fn diff(
        &self,
        session_id: Uuid,
        from_oid_hex: &str,
        to_oid_hex: Option<&str>,
    ) -> Result<GitDiff> {
        let existing_objects = self.db.load_all_git_objects(session_id).await?;
        let from_hex = from_oid_hex.to_string();
        let to_hex = to_oid_hex.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || -> Result<GitDiff> {
            let (repo, _) = hydrate_odb(&existing_objects)?;

            let new_commit = repo.find_commit(Oid::from_str(&from_hex)?)?;
            let new_tree = new_commit.tree()?;

            let old_tree = match &to_hex {
                Some(hex) => {
                    let old_commit = repo.find_commit(Oid::from_str(hex)?)?;
                    Some(old_commit.tree()?)
                }
                None => {
                    if new_commit.parent_count() > 0 {
                        Some(new_commit.parent(0)?.tree()?)
                    } else {
                        None
                    }
                }
            };

            let diff = repo.diff_tree_to_tree(old_tree.as_ref(), Some(&new_tree), None)?;
            let stats = diff.stats()?;
            let mut entries = Vec::new();

            diff.foreach(
                &mut |delta, _progress| {
                    let status = match delta.status() {
                        git2::Delta::Added => "added",
                        git2::Delta::Deleted => "deleted",
                        git2::Delta::Modified => "modified",
                        git2::Delta::Renamed => "renamed",
                        git2::Delta::Copied => "copied",
                        _ => "unknown",
                    };
                    // Fix: use old_file().path() for deletions, new_file().path() otherwise
                    let path = match delta.status() {
                        git2::Delta::Deleted => delta
                            .old_file()
                            .path()
                            .or_else(|| delta.new_file().path())
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default(),
                        _ => delta
                            .new_file()
                            .path()
                            .or_else(|| delta.old_file().path())
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default(),
                    };
                    let old_path = if delta.status() == git2::Delta::Renamed {
                        delta
                            .old_file()
                            .path()
                            .map(|p| p.to_string_lossy().to_string())
                    } else {
                        None
                    };
                    entries.push(GitDiffEntry {
                        path,
                        status: status.to_string(),
                        old_path,
                    });
                    true
                },
                None,
                None,
                None,
            )?;

            // Bounded patch text
            let mut patch_text = String::new();
            let max_patch_bytes = 64 * 1024;
            diff.print(git2::DiffFormat::Patch, |_delta, _hunk, line| {
                if patch_text.len() < max_patch_bytes {
                    let origin = line.origin();
                    if origin == '+' || origin == '-' || origin == ' ' {
                        patch_text.push(origin);
                    }
                    if let Ok(content) = std::str::from_utf8(line.content()) {
                        patch_text.push_str(content);
                    }
                }
                true
            })?;

            if patch_text.len() >= max_patch_bytes {
                patch_text.truncate(max_patch_bytes);
                patch_text.push_str("\n... (truncated)");
            }

            Ok(GitDiff {
                entries,
                patch: if patch_text.is_empty() {
                    None
                } else {
                    Some(patch_text)
                },
                stats: GitDiffStats {
                    files_changed: stats.files_changed(),
                    insertions: stats.insertions(),
                    deletions: stats.deletions(),
                },
            })
        })
        .await?
    }

    // ========================================================
    // Public API: branches (refs)
    // ========================================================

    pub async fn list_refs(&self, session_id: Uuid) -> Result<Vec<GitRefInfo>> {
        let refs = self.db.list_git_refs(session_id).await?;
        Ok(refs
            .into_iter()
            .map(|r| GitRefInfo {
                name: r.name,
                target: hex::encode(&r.target),
                is_symbolic: r.is_symbolic,
            })
            .collect())
    }

    pub async fn create_branch(
        &self,
        session_id: Uuid,
        branch_name: &str,
        commit_oid_hex: &str,
    ) -> Result<()> {
        let oid = Oid::from_str(commit_oid_hex)?;
        let ref_name = normalize_branch(branch_name);
        let session_id_typed = everruns_core::SessionId::from_uuid(session_id);
        self.db
            .write_git_ref(CreateSessionGitRef {
                session_id: session_id_typed,
                name: ref_name,
                target: oid.as_bytes().to_vec(),
                is_symbolic: false,
            })
            .await
    }

    pub async fn delete_branch(&self, session_id: Uuid, branch_name: &str) -> Result<bool> {
        let ref_name = normalize_branch(branch_name);
        self.db.delete_git_ref(session_id, &ref_name).await
    }

    // ========================================================
    // Public API: fork session
    // ========================================================

    pub async fn fork_session(
        &self,
        source_session_id: Uuid,
        target_session_id: Uuid,
    ) -> Result<(u64, u64)> {
        let objects_copied = self
            .db
            .fork_git_objects(source_session_id, target_session_id)
            .await?;
        let refs_copied = self
            .db
            .fork_git_refs(source_session_id, target_session_id)
            .await?;
        Ok((objects_copied, refs_copied))
    }
}

// ========================================================
// Pure functions (no async, safe for spawn_blocking)
// ========================================================

fn obj_type_from_i16(t: i16) -> Result<ObjectType> {
    match t {
        1 => Ok(ObjectType::Commit),
        2 => Ok(ObjectType::Tree),
        3 => Ok(ObjectType::Blob),
        4 => Ok(ObjectType::Tag),
        _ => bail!("unknown git object type: {}", t),
    }
}

fn obj_type_to_i16(t: ObjectType) -> i16 {
    match t {
        ObjectType::Commit => 1,
        ObjectType::Tree => 2,
        ObjectType::Blob => 3,
        ObjectType::Tag => 4,
        _ => 0,
    }
}

fn oid_from_bytes(bytes: &[u8]) -> Result<Oid> {
    let arr: [u8; 20] = bytes
        .try_into()
        .map_err(|_| anyhow!("OID is not 20 bytes: {} bytes", bytes.len()))?;
    Ok(Oid::from_bytes(&arr)?)
}

/// Create a mempack-backed ODB and load existing objects into it.
/// Returns (Repository, set of known OIDs).
fn hydrate_odb(objects: &[SessionGitObjectRow]) -> Result<(Repository, HashSet<[u8; 20]>)> {
    let odb = git2::Odb::new()?;
    let _mempack = odb.add_new_mempack_backend(1000)?;

    let mut known_oids = HashSet::with_capacity(objects.len());

    for obj in objects {
        let obj_type = obj_type_from_i16(obj.obj_type)?;
        let oid = odb.write(obj_type, &obj.data)?;

        let stored_oid: [u8; 20] = obj
            .oid
            .as_slice()
            .try_into()
            .map_err(|_| anyhow!("stored OID is not 20 bytes: {} bytes", obj.oid.len()))?;
        if oid.as_bytes() != stored_oid {
            bail!(
                "OID mismatch: stored {} but computed {}",
                hex::encode(&obj.oid),
                oid
            );
        }
        known_oids.insert(stored_oid);
    }

    let repo = Repository::from_odb(odb)?;
    Ok((repo, known_oids))
}

/// Read specific objects from ODB by OID and package them for persistence.
fn read_objects_by_oid(
    odb: &git2::Odb<'_>,
    oids: &[Oid],
    session_id: everruns_core::SessionId,
) -> Result<Vec<CreateSessionGitObject>> {
    let mut new_objects: Vec<CreateSessionGitObject> = Vec::with_capacity(oids.len());
    let mut seen: HashSet<Oid> = HashSet::new();
    for oid in oids {
        if !seen.insert(*oid) {
            continue;
        }
        let obj = odb.read(*oid)?;
        new_objects.push(CreateSessionGitObject {
            session_id,
            oid: oid.as_bytes().to_vec(),
            obj_type: obj_type_to_i16(obj.kind()),
            size: obj.len() as i64,
            data: obj.data().to_vec(),
        });
    }
    Ok(new_objects)
}

/// Build a tree from paths, tracking newly-created tree OIDs for drain.
fn build_tree_from_paths_tracking(
    repo: &Repository,
    entries: &[(String, Oid)],
    known_oids: &HashSet<[u8; 20]>,
    created_oids: &mut Vec<Oid>,
) -> Result<Oid> {
    let oid = build_tree_from_paths(repo, entries)?;
    collect_tree_oids(repo, oid, known_oids, created_oids);
    Ok(oid)
}

/// Recursively collect tree OIDs that are new.
fn collect_tree_oids(
    repo: &Repository,
    tree_oid: Oid,
    known_oids: &HashSet<[u8; 20]>,
    created_oids: &mut Vec<Oid>,
) {
    if !known_oids.contains(tree_oid.as_bytes()) {
        created_oids.push(tree_oid);
    }
    if let Ok(tree) = repo.find_tree(tree_oid) {
        for entry in tree.iter() {
            if entry.kind() == Some(ObjectType::Tree) {
                collect_tree_oids(repo, entry.id(), known_oids, created_oids);
            }
        }
    }
}

/// Build a tree from a flat list of (path, blob_oid) pairs.
fn build_tree_from_paths(repo: &Repository, entries: &[(String, Oid)]) -> Result<Oid> {
    let mut root_files: Vec<(&str, Oid)> = Vec::new();
    let mut subdirs: std::collections::HashMap<&str, Vec<(String, Oid)>> =
        std::collections::HashMap::new();

    for (path, oid) in entries {
        if let Some((dir, rest)) = path.split_once('/') {
            subdirs
                .entry(dir)
                .or_default()
                .push((rest.to_string(), *oid));
        } else {
            root_files.push((path.as_str(), *oid));
        }
    }

    let odb = repo.odb()?;
    let empty_tree_oid = odb.write(ObjectType::Tree, &[])?;
    let empty_tree = repo.find_tree(empty_tree_oid)?;
    let mut builder = repo.treebuilder(Some(&empty_tree))?;
    builder.clear()?;

    for (name, oid) in &root_files {
        builder.insert(name, *oid, 0o100644)?;
    }

    for (dir_name, sub_entries) in &subdirs {
        let subtree_oid = build_tree_from_paths(repo, sub_entries)?;
        builder.insert(dir_name, subtree_oid, 0o040000)?;
    }

    let tree_oid = builder.write()?;
    Ok(tree_oid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_obj_type_roundtrip() {
        for (i, t) in [
            (1i16, ObjectType::Commit),
            (2, ObjectType::Tree),
            (3, ObjectType::Blob),
            (4, ObjectType::Tag),
        ] {
            assert_eq!(obj_type_to_i16(t), i);
            assert_eq!(obj_type_from_i16(i).unwrap(), t);
        }
    }

    #[test]
    fn test_oid_from_bytes() {
        let bytes = [0u8; 20];
        let oid = oid_from_bytes(&bytes).unwrap();
        assert_eq!(oid.as_bytes(), &bytes);
    }

    #[test]
    fn test_oid_from_bytes_wrong_len() {
        assert!(oid_from_bytes(&[0u8; 19]).is_err());
        assert!(oid_from_bytes(&[0u8; 21]).is_err());
    }

    #[test]
    fn test_normalize_branch() {
        assert_eq!(normalize_branch("main"), "refs/heads/main");
        assert_eq!(normalize_branch("feature-x"), "refs/heads/feature-x");
        assert_eq!(normalize_branch("refs/heads/main"), "refs/heads/main");
        assert_eq!(normalize_branch("refs/tags/v1"), "refs/tags/v1");
        assert_eq!(normalize_branch("HEAD"), "HEAD");
    }

    #[test]
    fn test_build_tree_from_paths() {
        let odb = git2::Odb::new().unwrap();
        let _mempack = odb.add_new_mempack_backend(1000).unwrap();
        let repo = Repository::from_odb(odb).unwrap();

        let odb = repo.odb().unwrap();
        let blob_oid = odb.write(ObjectType::Blob, b"hello world").unwrap();

        let entries = vec![
            ("README.md".to_string(), blob_oid),
            ("src/main.rs".to_string(), blob_oid),
            ("src/lib.rs".to_string(), blob_oid),
        ];

        let tree_oid = build_tree_from_paths(&repo, &entries).unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();

        assert!(tree.get_name("README.md").is_some());
        assert!(tree.get_name("src").is_some());

        let src_entry = tree.get_name("src").unwrap();
        let src_tree = repo.find_tree(src_entry.id()).unwrap();
        assert!(src_tree.get_name("main.rs").is_some());
        assert!(src_tree.get_name("lib.rs").is_some());
    }

    #[test]
    fn test_hydrate_empty() {
        let (repo, known) = hydrate_odb(&[]).unwrap();
        assert!(known.is_empty());
        let odb = repo.odb().unwrap();
        let _blob = odb.write(ObjectType::Blob, b"test").unwrap();
    }
}
