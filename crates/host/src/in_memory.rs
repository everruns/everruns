// In-memory host stores.
// Decision: runtime ships batteries-included in-memory stores for public
// embedding so common capabilities work without depending on server internals.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use everruns_core::AgentCapabilityConfig;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::session::ExecutionSession;
use everruns_core::session_file::{
    FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, InitialFile, SessionFile,
    build_grep_search_result,
};
use everruns_core::traits::{
    KeyInfo, SecretInfo, SessionFileSystem, SessionFileSystemFactory,
    SessionFileSystemFactoryContext, SessionMutator, SessionStorageStore, SessionStore,
};
use everruns_core::typed_id::SessionId;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// In-memory `SessionStore` + `SessionMutator` for embedded runtimes.
///
/// This is the default session backend used by [`crate::InProcessRuntime`].
/// Holds portable [`ExecutionSession`] values (EVE-882): no stored Session
/// persistence records exist in the embedded host.
#[derive(Debug, Default, Clone)]
pub struct InMemorySessionStore {
    sessions: Arc<RwLock<HashMap<SessionId, ExecutionSession>>>,
}

impl InMemorySessionStore {
    /// Create an empty in-memory session store.
    pub fn new() -> Self {
        Self {
            sessions: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Insert or replace a session in the store.
    pub async fn add_session(&self, session: ExecutionSession) {
        self.sessions.write().await.insert(session.id, session);
    }
}

#[async_trait]
impl SessionStore for InMemorySessionStore {
    async fn get_session(&self, session_id: SessionId) -> Result<Option<ExecutionSession>> {
        Ok(self.sessions.read().await.get(&session_id).cloned())
    }
}

#[async_trait]
impl SessionMutator for InMemorySessionStore {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> Result<ExecutionSession> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        session.title = Some(title);
        Ok(session.clone())
    }

    async fn upsert_session_capability(
        &self,
        session_id: SessionId,
        capability: AgentCapabilityConfig,
    ) -> Result<ExecutionSession> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        if let Some(existing) = session
            .capabilities
            .iter_mut()
            .find(|existing| existing.capability_id() == capability.capability_id())
        {
            *existing = capability;
        } else {
            session.capabilities.push(capability);
        }
        Ok(session.clone())
    }

    async fn remove_session_capability(
        &self,
        session_id: SessionId,
        capability_id: &str,
    ) -> Result<ExecutionSession> {
        let mut sessions = self.sessions.write().await;
        let session = sessions
            .get_mut(&session_id)
            .ok_or_else(|| AgentLoopError::store(format!("session not found: {session_id}")))?;
        session
            .capabilities
            .retain(|capability| capability.capability_id() != capability_id);
        Ok(session.clone())
    }
}

#[derive(Debug, Clone)]
struct FileEntry {
    file: SessionFile,
}

/// In-memory implementation of the session virtual filesystem.
///
/// Paths accept either canonical session paths (`/notes.txt`) or `/workspace`
/// prefixed paths (`/workspace/notes.txt`).
#[derive(Debug, Default, Clone)]
pub struct InMemorySessionFileStore {
    files: Arc<RwLock<HashMap<(SessionId, String), FileEntry>>>,
}

/// Factory for the runtime's in-memory session filesystem.
#[derive(Debug, Clone, Default)]
pub struct InMemorySessionFileSystemFactory;

#[async_trait]
impl SessionFileSystemFactory for InMemorySessionFileSystemFactory {
    fn name(&self) -> &'static str {
        "InMemorySessionFileSystemFactory"
    }

    async fn create_session_file_system(
        &self,
        _context: SessionFileSystemFactoryContext,
    ) -> Result<Arc<dyn SessionFileSystem>> {
        Ok(Arc::new(InMemorySessionFileStore::new()))
    }
}

impl InMemorySessionFileStore {
    /// Create an empty in-memory file store.
    pub fn new() -> Self {
        Self {
            files: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Seed a file into a session workspace.
    pub async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        let path = normalize_path(&file.path);
        self.ensure_parent_directories(session_id, &path).await?;
        self.upsert_file(
            session_id,
            &path,
            &file.content,
            &file.encoding,
            file.is_readonly,
        )
        .await
        .map(|_| ())
    }

    async fn ensure_parent_directories(&self, session_id: SessionId, path: &str) -> Result<()> {
        let mut current = String::new();
        for segment in path.trim_start_matches('/').split('/').collect::<Vec<_>>() {
            if segment.is_empty() {
                continue;
            }
            current.push('/');
            current.push_str(segment);
            let is_leaf = current == path;
            if is_leaf {
                break;
            }
            self.insert_directory_if_missing(session_id, &current)
                .await?;
        }
        Ok(())
    }

    async fn insert_directory_if_missing(&self, session_id: SessionId, path: &str) -> Result<()> {
        let path = normalize_path(path);
        if path == "/" {
            return Ok(());
        }

        let mut files = self.files.write().await;
        files
            .entry((session_id, path.clone()))
            .or_insert_with(|| FileEntry {
                file: SessionFile {
                    id: Uuid::now_v7(),
                    session_id: session_id.uuid(),
                    path: path.clone(),
                    name: FileInfo::name_from_path(&path),
                    content: None,
                    encoding: "text".to_string(),
                    is_directory: true,
                    is_readonly: false,
                    size_bytes: 0,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            });
        Ok(())
    }

    async fn upsert_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
        is_readonly: bool,
    ) -> Result<SessionFile> {
        let now = Utc::now();
        let normalized = normalize_path(path);
        let mut files = self.files.write().await;
        let key = (session_id, normalized.clone());

        let file = files
            .entry(key)
            .and_modify(|entry| {
                entry.file.content = Some(content.to_string());
                entry.file.encoding = encoding.to_string();
                entry.file.is_directory = false;
                entry.file.is_readonly = is_readonly;
                entry.file.size_bytes = content.len() as i64;
                entry.file.updated_at = now;
            })
            .or_insert_with(|| FileEntry {
                file: SessionFile {
                    id: Uuid::now_v7(),
                    session_id: session_id.uuid(),
                    path: normalized.clone(),
                    name: FileInfo::name_from_path(&normalized),
                    content: Some(content.to_string()),
                    encoding: encoding.to_string(),
                    is_directory: false,
                    is_readonly,
                    size_bytes: content.len() as i64,
                    created_at: now,
                    updated_at: now,
                },
            })
            .file
            .clone();

        Ok(file)
    }

    /// Read a text file from the workspace, returning `None` when absent.
    pub async fn read_text(&self, session_id: SessionId, path: &str) -> Option<String> {
        self.read_file(session_id, path)
            .await
            .ok()
            .flatten()
            .and_then(|file| file.content)
    }
}

#[async_trait]
impl SessionFileSystem for InMemorySessionFileStore {
    async fn seed_initial_file(&self, session_id: SessionId, file: &InitialFile) -> Result<()> {
        InMemorySessionFileStore::seed_initial_file(self, session_id, file).await
    }

    async fn read_file(&self, session_id: SessionId, path: &str) -> Result<Option<SessionFile>> {
        let normalized = normalize_path(path);
        if normalized == "/" {
            return Ok(Some(root_directory(session_id)));
        }

        Ok(self
            .files
            .read()
            .await
            .get(&(session_id, normalized))
            .map(|entry| entry.file.clone()))
    }

    async fn write_file(
        &self,
        session_id: SessionId,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        let normalized = normalize_path(path);
        self.ensure_parent_directories(session_id, &normalized)
            .await?;

        if let Some(existing) = self.read_file(session_id, &normalized).await?
            && existing.is_readonly
        {
            return Err(AgentLoopError::tool(format!(
                "file is read-only: {}",
                normalized
            )));
        }

        self.upsert_file(session_id, &normalized, content, encoding, false)
            .await
    }

    async fn delete_file(
        &self,
        session_id: SessionId,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        let normalized = normalize_path(path);
        if normalized == "/" {
            return Ok(false);
        }

        let mut files = self.files.write().await;
        let key = (session_id, normalized.clone());
        let Some(existing) = files.get(&key).cloned() else {
            return Ok(false);
        };

        if existing.file.is_readonly {
            return Ok(false);
        }

        if existing.file.is_directory {
            let prefix = format!("{normalized}/");
            let has_children = files
                .keys()
                .any(|(sid, candidate)| *sid == session_id && candidate.starts_with(&prefix));
            if has_children && !recursive {
                return Ok(false);
            }
            files.retain(|(sid, candidate), _| {
                !(*sid == session_id
                    && (candidate == &normalized || candidate.starts_with(&prefix)))
            });
            return Ok(true);
        }

        Ok(files.remove(&key).is_some())
    }

    async fn list_directory(&self, session_id: SessionId, path: &str) -> Result<Vec<FileInfo>> {
        let normalized = normalize_path(path);
        if normalized != "/" {
            let Some(dir) = self.read_file(session_id, &normalized).await? else {
                return Ok(vec![]);
            };
            if !dir.is_directory {
                return Ok(vec![]);
            }
        }

        let files = self.files.read().await;
        let mut infos = Vec::new();
        let mut seen = BTreeSet::new();

        for ((sid, candidate), entry) in files.iter() {
            if *sid != session_id {
                continue;
            }
            if FileInfo::parent_path(candidate).as_deref() != Some(normalized.as_str()) {
                continue;
            }
            if seen.insert(candidate.clone()) {
                infos.push(file_info(&entry.file));
            }
        }

        infos.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(infos)
    }

    async fn stat_file(&self, session_id: SessionId, path: &str) -> Result<Option<FileStat>> {
        let normalized = normalize_path(path);
        if normalized == "/" {
            let root = root_directory(session_id);
            return Ok(Some(FileStat {
                path: root.path,
                name: root.name,
                is_directory: true,
                is_readonly: false,
                size_bytes: 0,
                created_at: root.created_at,
                updated_at: root.updated_at,
            }));
        }

        Ok(self
            .files
            .read()
            .await
            .get(&(session_id, normalized))
            .map(|entry| FileStat {
                path: entry.file.path.clone(),
                name: entry.file.name.clone(),
                is_directory: entry.file.is_directory,
                is_readonly: entry.file.is_readonly,
                size_bytes: entry.file.size_bytes,
                created_at: entry.file.created_at,
                updated_at: entry.file.updated_at,
            }))
    }

    async fn grep_files(
        &self,
        session_id: SessionId,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let regex = crate::grep_limits::build_regex(pattern)?;
        crate::grep_limits::validate_path_pattern(path_pattern)?;
        let path_pattern = path_pattern
            .map(everruns_core::session_path::GrepPathPattern::new)
            .transpose()?;
        let files = self.files.read().await;
        let mut matches = Vec::new();
        let mut total_scanned = 0;

        for ((sid, path), entry) in files.iter() {
            if *sid != session_id || entry.file.is_directory || entry.file.encoding != "text" {
                continue;
            }
            if let Some(path_pattern) = &path_pattern
                && !path_pattern.is_match(path)
            {
                continue;
            }

            let Some(content) = &entry.file.content else {
                continue;
            };
            if !crate::grep_limits::account_scan(&mut total_scanned, content.len())? {
                continue;
            }

            for (idx, line) in content.lines().enumerate() {
                if regex.is_match(line) {
                    matches.push(GrepMatch {
                        path: path.clone(),
                        line_number: idx + 1,
                        line: line.to_string(),
                    });
                }
            }
        }

        Ok(matches)
    }

    async fn grep_files_with_options(
        &self,
        session_id: SessionId,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        let regex = crate::grep_limits::build_regex(pattern)?;
        crate::grep_limits::validate_path_pattern(options.path_pattern.as_deref())?;
        let path_pattern = options
            .path_pattern
            .as_deref()
            .map(everruns_core::session_path::GrepPathPattern::new)
            .transpose()?;
        let files = self.files.read().await;
        let mut total_scanned = 0;
        let mut text_files = Vec::new();
        for ((_, path), entry) in files.iter().filter(|((sid, path), entry)| {
            *sid == session_id
                && !entry.file.is_directory
                && entry.file.encoding == "text"
                && path_pattern
                    .as_ref()
                    .is_none_or(|matcher| matcher.is_match(path))
        }) {
            let Some(content) = entry.file.content.as_ref() else {
                continue;
            };
            if crate::grep_limits::account_scan(&mut total_scanned, content.len())? {
                text_files.push((path.clone(), content.clone()));
            }
        }
        Ok(build_grep_search_result(text_files, &regex, options))
    }

    async fn create_directory(&self, session_id: SessionId, path: &str) -> Result<FileInfo> {
        let normalized = normalize_path(path);
        self.ensure_parent_directories(session_id, &normalized)
            .await?;
        self.insert_directory_if_missing(session_id, &normalized)
            .await?;
        let file = self
            .read_file(session_id, &normalized)
            .await?
            .ok_or_else(|| AgentLoopError::store(format!("directory not found: {normalized}")))?;
        Ok(file_info(&file))
    }

    fn is_mount_resolver(&self) -> bool {
        false
    }
}

/// In-memory implementation of session key/value and secret storage.
#[derive(Debug, Default, Clone)]
pub struct InMemorySessionStorageStore {
    values: Arc<RwLock<HashMap<(SessionId, String), StorageValue>>>,
    secrets: Arc<RwLock<HashMap<(SessionId, String), StorageValue>>>,
}

#[derive(Debug, Clone)]
struct StorageValue {
    value: String,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

impl InMemorySessionStorageStore {
    /// Create an empty in-memory storage store.
    pub fn new() -> Self {
        Self {
            values: Arc::new(RwLock::new(HashMap::new())),
            secrets: Arc::new(RwLock::new(HashMap::new())),
        }
    }
}

#[async_trait]
impl SessionStorageStore for InMemorySessionStorageStore {
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> Result<()> {
        upsert_storage(&self.values, session_id, key, value).await;
        Ok(())
    }

    async fn get_value(&self, session_id: SessionId, key: &str) -> Result<Option<String>> {
        Ok(self
            .values
            .read()
            .await
            .get(&(session_id, key.to_string()))
            .map(|value| value.value.clone()))
    }

    async fn delete_value(&self, session_id: SessionId, key: &str) -> Result<bool> {
        Ok(self
            .values
            .write()
            .await
            .remove(&(session_id, key.to_string()))
            .is_some())
    }

    async fn list_keys(&self, session_id: SessionId) -> Result<Vec<KeyInfo>> {
        Ok(list_storage(&self.values, session_id)
            .await
            .into_iter()
            .map(|(key, value)| KeyInfo {
                key,
                created_at: value.created_at,
                updated_at: value.updated_at,
            })
            .collect())
    }

    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> Result<()> {
        upsert_storage(&self.secrets, session_id, name, value).await;
        Ok(())
    }

    async fn get_secret(&self, session_id: SessionId, name: &str) -> Result<Option<String>> {
        Ok(self
            .secrets
            .read()
            .await
            .get(&(session_id, name.to_string()))
            .map(|value| value.value.clone()))
    }

    async fn delete_secret(&self, session_id: SessionId, name: &str) -> Result<bool> {
        Ok(self
            .secrets
            .write()
            .await
            .remove(&(session_id, name.to_string()))
            .is_some())
    }

    async fn list_secrets(&self, session_id: SessionId) -> Result<Vec<SecretInfo>> {
        Ok(list_storage(&self.secrets, session_id)
            .await
            .into_iter()
            .map(|(name, value)| SecretInfo {
                name,
                created_at: value.created_at,
                updated_at: value.updated_at,
            })
            .collect())
    }
}

async fn upsert_storage(
    map: &Arc<RwLock<HashMap<(SessionId, String), StorageValue>>>,
    session_id: SessionId,
    key: &str,
    value: &str,
) {
    let mut map = map.write().await;
    let now = Utc::now();
    map.entry((session_id, key.to_string()))
        .and_modify(|stored| {
            stored.value = value.to_string();
            stored.updated_at = now;
        })
        .or_insert_with(|| StorageValue {
            value: value.to_string(),
            created_at: now,
            updated_at: now,
        });
}

async fn list_storage(
    map: &Arc<RwLock<HashMap<(SessionId, String), StorageValue>>>,
    session_id: SessionId,
) -> Vec<(String, StorageValue)> {
    let mut values: Vec<_> = map
        .read()
        .await
        .iter()
        .filter(|((sid, _), _)| *sid == session_id)
        .map(|((_, key), value)| (key.clone(), value.clone()))
        .collect();
    values.sort_by(|a, b| a.0.cmp(&b.0));
    values
}

fn normalize_path(path: &str) -> String {
    // Single workspace path normalizer (EVE-660): the in-memory VFS shares the
    // same `/workspace`-alias handling as every other backend.
    everruns_core::session_path::to_session_path(path)
}

fn root_directory(session_id: SessionId) -> SessionFile {
    let now = Utc::now();
    SessionFile {
        id: Uuid::nil(),
        session_id: session_id.uuid(),
        path: "/".to_string(),
        name: "/".to_string(),
        content: None,
        encoding: "text".to_string(),
        is_directory: true,
        is_readonly: false,
        size_bytes: 0,
        created_at: now,
        updated_at: now,
    }
}

fn file_info(file: &SessionFile) -> FileInfo {
    FileInfo {
        id: file.id,
        session_id: file.session_id,
        path: file.path.clone(),
        name: file.name.clone(),
        is_directory: file.is_directory,
        is_readonly: file.is_readonly,
        size_bytes: file.size_bytes,
        created_at: file.created_at,
        updated_at: file.updated_at,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn grep_path_pattern_supports_globs_and_substring_compatibility() {
        let store = InMemorySessionFileStore::new();
        let session = SessionId::from_seed(1);
        for path in [
            "/src/lib.rs",
            "/src/nested/mod.rs",
            "/docs/readme.md",
            "/notes.txt",
        ] {
            store
                .write_file(session, path, "needle", "text")
                .await
                .unwrap();
        }

        let mut glob_paths: Vec<_> = store
            .grep_files(session, "needle", Some("src/**/*.rs"))
            .await
            .unwrap()
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        glob_paths.sort();
        assert_eq!(glob_paths, vec!["/src/lib.rs", "/src/nested/mod.rs"]);

        let substring_paths: Vec<_> = store
            .grep_files(session, "needle", Some("docs"))
            .await
            .unwrap()
            .into_iter()
            .map(|hit| hit.path)
            .collect();
        assert_eq!(substring_paths, vec!["/docs/readme.md"]);
    }
}
