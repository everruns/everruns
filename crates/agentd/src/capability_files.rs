//! The files a session's capabilities bring, on the data plane (data plane
//! v2 parity, Phase B): the `platform` capability's product documentation
//! at `/workspace/docs`, a declarative capability's files, Daytona's and
//! data knowledge's notes. A v1 worker's session has them from its create
//! (`session_files::apply_capability_mounts` in the server); a shard's had
//! none, so an agent told to read `/workspace/docs` found nothing.
//!
//! Decisions:
//! - A virtual tree (the product docs, hundreds of files) is mounted
//!   read-only from the shard's memory at each activation, never copied:
//!   the tree is built once per process and shared by every session, as the
//!   server's `VirtualMountRegistry` serves it on v1.
//! - Inline files and directories are starter files: written into the
//!   session's workspace with the create, only where absent, read-only when
//!   the capability says so, as v1 copies them. Small, and a writable one
//!   is the session's to change.
//! - Resolved from the session's own definition with the shard's capability
//!   registry and each capability's configuration, so a declarative
//!   capability's files come from its definition.
//! - A subagent's child gets the docs (its definition is its parent's agent)
//!   but no inline files: the control plane's create seeds those, and a child
//!   is created on the shard.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::typed_id::SessionId;
use everruns_core::capability_types::{MountPoint, MountSource, VirtualFileTree};
use everruns_core::host::HostBackends;
use everruns_core::session_file::{FileInfo, FileStat, GrepMatch, InitialFile, SessionFile};
use everruns_core::session_files::SessionFileSystem;
use serde_json::Value;

/// Largest file a grep reads, as the shard's workspace applies it.
const GREP_FILE_MAX: usize = 512 * 1024;
const PATTERN_MAX: usize = 1_000;

/// The mounts `definition`'s capabilities (and their dependencies) bring.
fn mounts(definition: &Value) -> Vec<MountPoint> {
    let Ok(package) = crate::host::package(definition) else {
        return Vec::new();
    };
    let registry = crate::host::hosted_registry();
    let Ok(resolved) = everruns_core::capabilities::resolve_capability_configs(
        &package.manifest().capabilities,
        registry,
    ) else {
        return Vec::new();
    };
    let mut mounts = Vec::new();
    for config in &resolved {
        let Some(capability) = registry.get(config.capability_id()) else {
            continue;
        };
        if !capability.status().is_active() {
            continue;
        }
        mounts.extend(capability.mounts());
    }
    mounts
}

/// `definition`'s virtual trees, by where they hang. Resolved once per
/// shared definition (`state::definitions`), not at every activation.
fn trees(definition: &Arc<Value>) -> Arc<Vec<(String, Arc<VirtualFileTree>)>> {
    type Cache = std::collections::HashMap<
        usize,
        (
            std::sync::Weak<Value>,
            Arc<Vec<(String, Arc<VirtualFileTree>)>>,
        ),
    >;
    static CACHE: parking_lot::Mutex<Option<Cache>> = parking_lot::Mutex::new(None);
    let at = Arc::as_ptr(definition) as usize;
    if let Some((held, trees)) = CACHE.lock().as_ref().and_then(|cache| cache.get(&at))
        && held
            .upgrade()
            .is_some_and(|held| Arc::ptr_eq(&held, definition))
    {
        return trees.clone();
    }
    let trees: Arc<Vec<_>> = Arc::new(
        mounts(definition)
            .into_iter()
            .filter_map(|mount| match mount.source {
                MountSource::Virtual { tree } => Some((normalized(&mount.path), tree)),
                _ => None,
            })
            .collect(),
    );
    let mut cache = CACHE.lock();
    let cache = cache.get_or_insert_with(Default::default);
    if cache.len() > 1024 {
        cache.retain(|_, (held, _)| held.strong_count() > 0);
    }
    cache.insert(at, (Arc::downgrade(definition), trees.clone()));
    trees
}

/// `backends` with `definition`'s read-only virtual trees mounted.
pub(crate) fn attach(mut backends: HostBackends, definition: &Arc<Value>) -> HostBackends {
    for (root, tree) in trees(definition).iter() {
        let root = root.clone();
        let tree = tree.clone();
        let files: Arc<dyn SessionFileSystem> = Arc::new(TreeFiles { tree });
        // Both spellings a tool uses: `/docs` and `/workspace/docs` (the
        // workspace view of the same backend would otherwise win by its
        // longer prefix).
        backends
            .file_mounts
            .push((format!("/workspace{root}"), files.clone(), root.clone()));
        backends.file_mounts.push((root.clone(), files, root));
    }
    backends
}

/// `definition`'s inline capability files, as starter files.
pub(crate) fn starter_files(definition: &Value) -> Vec<InitialFile> {
    let mut files = Vec::new();
    for mount in mounts(definition) {
        let readonly = mount.is_readonly();
        flatten(
            &normalized(&mount.path),
            &mount.source,
            readonly,
            &mut files,
        );
    }
    files
}

fn flatten(path: &str, source: &MountSource, readonly: bool, files: &mut Vec<InitialFile>) {
    match source {
        MountSource::InlineFile { content, encoding } => files.push(InitialFile {
            path: path.to_owned(),
            content: content.clone(),
            encoding: encoding.clone(),
            is_readonly: readonly,
        }),
        MountSource::InlineDirectory { entries } => {
            let mut names: Vec<&String> = entries.keys().collect();
            names.sort();
            for name in names {
                let child = format!("{}/{}", path.trim_end_matches('/'), name);
                flatten(&child, &entries[name].source, readonly, files);
            }
        }
        MountSource::Virtual { .. } => {}
    }
}

/// `/docs` for `docs`, `/docs/` or `/workspace/docs`.
fn normalized(path: &str) -> String {
    let path = path.trim_end_matches('/');
    let path = path.strip_prefix("/workspace").unwrap_or(path);
    if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    }
}

/// A shared, read-only tree of files, keyed by its full paths (`/docs/...`).
struct TreeFiles {
    tree: Arc<VirtualFileTree>,
}

fn epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::UNIX_EPOCH
}

fn read_only(path: &str) -> AgentLoopError {
    AgentLoopError::tool(format!("{path} is read-only"))
}

impl TreeFiles {
    fn key(path: &str) -> String {
        let path = path.trim_end_matches('/');
        if path.is_empty() {
            "/".into()
        } else {
            path.to_owned()
        }
    }

    fn info(&self, session: SessionId, path: &str, is_directory: bool, size: usize) -> FileInfo {
        FileInfo {
            id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, path.as_bytes()),
            session_id: session.uuid(),
            path: path.to_owned(),
            name: FileInfo::name_from_path(path),
            is_directory,
            is_readonly: true,
            size_bytes: size as i64,
            created_at: epoch(),
            updated_at: epoch(),
        }
    }
}

#[async_trait]
impl SessionFileSystem for TreeFiles {
    fn is_mount_resolver(&self) -> bool {
        false
    }

    async fn read_file(&self, session: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let key = Self::key(path);
        Ok(self.tree.get(&key).map(|file| SessionFile {
            id: uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, key.as_bytes()),
            session_id: session.uuid(),
            name: FileInfo::name_from_path(&key),
            content: (!file.is_directory)
                .then(|| String::from_utf8_lossy(&file.content).into_owned()),
            encoding: "text".into(),
            is_directory: file.is_directory,
            is_readonly: true,
            size_bytes: file.content.len() as i64,
            created_at: epoch(),
            updated_at: epoch(),
            path: key,
        }))
    }

    async fn create_file_if_absent(&self, _: SessionId, path: &str, _: &str) -> Result<bool> {
        Err(read_only(path))
    }

    async fn write_file(&self, _: SessionId, path: &str, _: &str, _: &str) -> Result<SessionFile> {
        Err(read_only(path))
    }

    async fn write_file_if_content_matches(
        &self,
        _: SessionId,
        path: &str,
        _: &str,
        _: &str,
        _: &str,
        _: &str,
    ) -> Result<Option<SessionFile>> {
        Err(read_only(path))
    }

    async fn delete_file(&self, _: SessionId, path: &str, _: bool) -> Result<bool> {
        Err(read_only(path))
    }

    async fn list_directory(&self, session: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let mut entries: Vec<FileInfo> = self
            .tree
            .list_directory(&Self::key(path))
            .into_iter()
            .map(|(path, file)| self.info(session, &path, file.is_directory, file.content.len()))
            .collect();
        entries.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(entries)
    }

    async fn stat_file(&self, _: SessionId, path: &str) -> Result<Option<FileStat>> {
        let key = Self::key(path);
        Ok(self.tree.get(&key).map(|file| FileStat {
            name: FileInfo::name_from_path(&key),
            is_directory: file.is_directory,
            is_readonly: true,
            size_bytes: file.content.len() as i64,
            created_at: epoch(),
            updated_at: epoch(),
            path: key,
        }))
    }

    async fn grep_files(
        &self,
        _: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        if pattern.len() > PATTERN_MAX || path_pattern.is_some_and(|p| p.len() > PATTERN_MAX) {
            return Err(AgentLoopError::tool(format!(
                "Pattern too long (max {PATTERN_MAX} characters)"
            )));
        }
        let regex = regex::Regex::new(pattern)
            .map_err(|error| AgentLoopError::tool(format!("Invalid regex pattern: {error}")))?;
        let matcher = path_pattern
            .map(everruns_core::session_path::GrepPathPattern::new)
            .transpose()?;
        let mut files: Vec<(&str, &[u8])> = self
            .tree
            .all_files()
            .filter(|(path, file)| {
                file.content.len() <= GREP_FILE_MAX
                    && matcher.as_ref().is_none_or(|m| m.is_match(path))
            })
            .map(|(path, file)| (path, file.content.as_slice()))
            .collect();
        files.sort_by_key(|(path, _)| *path);
        let mut matches = Vec::new();
        for (path, content) in files {
            let Ok(text) = std::str::from_utf8(content) else {
                continue;
            };
            for (index, line) in text.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(GrepMatch {
                        path: path.to_owned(),
                        line_number: index + 1,
                        line: line.to_owned(),
                    });
                }
            }
        }
        Ok(matches)
    }

    async fn create_directory(&self, _: SessionId, path: &str) -> Result<FileInfo> {
        Err(read_only(path))
    }
}

#[cfg(test)]
#[path = "capability_files_tests.rs"]
mod tests;
