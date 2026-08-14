// Memory virtual filesystem service.
//
// Wraps the `memory_files` storage with path validation and the
// source-backed read-only guarantee. Source-backed Memories reject all
// write/delete/update calls so that file contents stay byte-identical to
// the synced repository snapshot.

use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateMemoryFileRow, MemoryFileInfoRow, MemoryFileRow, MemoryRow, UpdateMemoryFile,
};
use anyhow::{Result, anyhow};
use everruns_provider::typed_id::MemoryId;
use std::sync::Arc;
use uuid::Uuid;

/// Per-file read response (text or binary, with content_hash for stale-edit guards).
#[derive(Debug, Clone)]
pub struct MemoryFileRead {
    pub path: String,
    pub content: Vec<u8>,
    pub size_bytes: i64,
    pub content_hash: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone)]
pub struct GrepMatchInfo {
    pub path: String,
    pub size_bytes: i64,
}

#[derive(thiserror::Error, Debug)]
pub enum MemoryFsError {
    #[error("memory not found")]
    MemoryNotFound,
    #[error("memory is read-only (source-backed)")]
    Readonly,
    #[error("invalid path: {0}")]
    InvalidPath(String),
    #[error("invalid pattern: {0}")]
    InvalidPattern(String),
    #[error("not found")]
    NotFound,
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("directory not empty")]
    DirectoryNotEmpty,
    #[error("path is a directory; refuse to read as file")]
    IsDirectory,
    #[error("path is not a directory")]
    NotDirectory,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// Maximum file size considered by the grep path scan (matches session_files default).
const GREP_MAX_FILE_BYTES: i64 = 1_048_576; // 1 MiB

/// Max total bytes scanned across all path-filtered candidates in a single grep
/// call (TM-DOS-008). Mirrors the session-file grep budget so a broad glob over
/// many sub-`GREP_MAX_FILE_BYTES` files cannot force an unbounded aggregate
/// content scan (and unbounded per-candidate fetches).
const MAX_GREP_TOTAL_SCAN_BYTES: usize = 5 * 1024 * 1024; // 5 MiB

#[derive(Clone)]
pub struct MemoryFileService {
    db: Arc<StorageBackend>,
}

impl MemoryFileService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    /// Resolve the Memory by its public_id, scoped to the org.
    pub async fn resolve_memory(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<MemoryRow, MemoryFsError> {
        let memory_id: MemoryId = public_id
            .parse()
            .map_err(|_| MemoryFsError::MemoryNotFound)?;
        let memory = self
            .db
            .get_memory(org_id, memory_id)
            .await?
            .ok_or(MemoryFsError::MemoryNotFound)?;
        Ok(memory)
    }

    /// Read a single file from a Memory.
    pub async fn read_file(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<MemoryFileRead, MemoryFsError> {
        let normalized = normalize_path(path)?;
        let row = self
            .db
            .get_memory_file(memory_id, &normalized)
            .await?
            .ok_or(MemoryFsError::NotFound)?;
        if row.is_directory {
            return Err(MemoryFsError::IsDirectory);
        }
        Ok(MemoryFileRead {
            path: row.path,
            content: row.content.unwrap_or_default(),
            size_bytes: row.size_bytes,
            content_hash: row.content_hash,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// List immediate children of a directory.
    pub async fn list_directory(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<Vec<MemoryFileInfoRow>, MemoryFsError> {
        let normalized = normalize_path(path)?;
        if normalized != "/" {
            let info = self
                .db
                .get_memory_file_info(memory_id, &normalized)
                .await?
                .ok_or(MemoryFsError::NotFound)?;
            if !info.is_directory {
                return Err(MemoryFsError::NotDirectory);
            }
        }
        Ok(self.db.list_memory_files(memory_id, &normalized).await?)
    }

    /// Stat a single path (file or directory).
    pub async fn stat(
        &self,
        memory_id: Uuid,
        path: &str,
    ) -> Result<MemoryFileInfoRow, MemoryFsError> {
        let normalized = normalize_path(path)?;
        let info = self
            .db
            .get_memory_file_info(memory_id, &normalized)
            .await?
            .ok_or(MemoryFsError::NotFound)?;
        Ok(info)
    }

    /// Create a new file. Source-backed Memories reject. Auto-creates parent directories.
    pub async fn create_file(
        &self,
        memory: &MemoryRow,
        input: NewFileInput,
    ) -> Result<MemoryFileRow, MemoryFsError> {
        if memory.is_readonly {
            return Err(MemoryFsError::Readonly);
        }
        let path = normalize_path(&input.path)?;
        if path == "/" {
            return Err(MemoryFsError::InvalidPath("cannot create root".into()));
        }
        self.ensure_parents_exist(memory.id, &path).await?;
        let exists = self.db.memory_file_exists(memory.id, &path).await?;
        if exists {
            return Err(MemoryFsError::Conflict(format!(
                "path already exists: {path}"
            )));
        }
        let content_bytes = if input.is_directory {
            None
        } else {
            Some(input.content.unwrap_or_default())
        };
        let row = self
            .db
            .create_memory_file(
                memory.id,
                CreateMemoryFileRow {
                    path: path.clone(),
                    content: content_bytes,
                    is_directory: input.is_directory,
                    content_hash: None,
                },
            )
            .await?;
        Ok(row)
    }

    /// Replace a file's content.
    pub async fn update_file(
        &self,
        memory: &MemoryRow,
        path: &str,
        content: Vec<u8>,
    ) -> Result<MemoryFileRow, MemoryFsError> {
        if memory.is_readonly {
            return Err(MemoryFsError::Readonly);
        }
        let normalized = normalize_path(path)?;
        let row = self
            .db
            .update_memory_file(
                memory.id,
                &normalized,
                UpdateMemoryFile {
                    content: Some(content),
                    content_hash: Some(None),
                },
            )
            .await?
            .ok_or(MemoryFsError::NotFound)?;
        Ok(row)
    }

    /// Delete a file or directory. `recursive` is required to delete a non-empty directory.
    pub async fn delete(
        &self,
        memory: &MemoryRow,
        path: &str,
        recursive: bool,
    ) -> Result<u64, MemoryFsError> {
        if memory.is_readonly {
            return Err(MemoryFsError::Readonly);
        }
        let normalized = normalize_path(path)?;
        let info = self
            .db
            .get_memory_file_info(memory.id, &normalized)
            .await?
            .ok_or(MemoryFsError::NotFound)?;
        if info.is_directory {
            let has_children = self
                .db
                .memory_directory_has_children(memory.id, &normalized)
                .await?;
            if has_children && !recursive {
                return Err(MemoryFsError::DirectoryNotEmpty);
            }
            if recursive {
                let n = self
                    .db
                    .delete_memory_file_recursive(memory.id, &normalized)
                    .await?;
                return Ok(n);
            }
        }
        let removed = self.db.delete_memory_file(memory.id, &normalized).await?;
        Ok(if removed { 1 } else { 0 })
    }

    /// Grep across non-directory files. Returns matched file metadata; callers fetch content.
    pub async fn grep(
        &self,
        memory_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatchInfo>, MemoryFsError> {
        if pattern.is_empty() {
            return Err(MemoryFsError::InvalidPattern("empty pattern".into()));
        }
        // Validate regex up-front so client errors surface as 400 rather than
        // bubbling up from Postgres as a generic 500.
        let content_matcher = regex::Regex::new(pattern)
            .map_err(|e| MemoryFsError::InvalidPattern(format!("pattern: {e}")))?;
        let path_matcher = path_pattern
            .map(everruns_core::session_path::GrepPathPattern::new)
            .transpose()
            .map_err(|e| MemoryFsError::InvalidPattern(format!("path_pattern: {e}")))?;
        let rows = self
            .db
            .grep_memory_files(memory_id, pattern, path_pattern, GREP_MAX_FILE_BYTES)
            .await?;
        let mut matches = Vec::new();
        let mut total_scanned: usize = 0;
        for row in rows {
            if path_matcher
                .as_ref()
                .is_some_and(|matcher| !matcher.is_match(&row.path))
            {
                continue;
            }
            // With a path filter, storage deliberately avoids a content scan;
            // fetch and match only the path-scoped candidates here.
            if path_pattern.is_some() {
                // TM-DOS-008: bound the aggregate scan (and the number of
                // per-candidate fetches) so a broad glob cannot amplify. Charge
                // by candidate size before fetching content.
                total_scanned = total_scanned.saturating_add(row.size_bytes.max(0) as usize);
                if total_scanned > MAX_GREP_TOTAL_SCAN_BYTES {
                    return Err(MemoryFsError::Other(anyhow::anyhow!(
                        "Grep request exceeds maximum scan size ({MAX_GREP_TOTAL_SCAN_BYTES} bytes); narrow the path filter or pattern"
                    )));
                }
                let Some(file) = self.db.get_memory_file(memory_id, &row.path).await? else {
                    continue;
                };
                let Some(content) = file.content else {
                    continue;
                };
                let Ok(text) = std::str::from_utf8(&content) else {
                    continue;
                };
                if !content_matcher.is_match(text) {
                    continue;
                }
            }
            matches.push(GrepMatchInfo {
                path: row.path,
                size_bytes: row.size_bytes,
            });
        }
        Ok(matches)
    }

    async fn ensure_parents_exist(&self, memory_id: Uuid, path: &str) -> Result<(), MemoryFsError> {
        // path is normalized and starts with /.
        let mut current = String::new();
        let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if segments.len() <= 1 {
            return Ok(());
        }
        // Iterate parents but stop before the final segment (which is the new entry).
        for seg in &segments[..segments.len() - 1] {
            current.push('/');
            current.push_str(seg);
            let info = self.db.get_memory_file_info(memory_id, &current).await?;
            match info {
                Some(row) if row.is_directory => continue,
                Some(_) => {
                    return Err(MemoryFsError::Conflict(format!(
                        "parent {current} is a file, not a directory"
                    )));
                }
                None => {
                    // Auto-create the directory. Race-safe: a concurrent
                    // request may have inserted the same parent between our
                    // get_info check and this insert. On insert failure,
                    // re-fetch and treat "already exists as directory" as
                    // success; "already exists as file" surfaces as Conflict.
                    if let Err(e) = self
                        .db
                        .create_memory_file(
                            memory_id,
                            CreateMemoryFileRow {
                                path: current.clone(),
                                content: None,
                                is_directory: true,
                                content_hash: None,
                            },
                        )
                        .await
                    {
                        match self.db.get_memory_file_info(memory_id, &current).await? {
                            Some(row) if row.is_directory => continue,
                            Some(_) => {
                                return Err(MemoryFsError::Conflict(format!(
                                    "parent {current} is a file, not a directory"
                                )));
                            }
                            None => return Err(MemoryFsError::Other(e)),
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// Input for create_file. `content` is raw bytes so binary uploads round-trip
/// without lossy UTF-8 conversion at the service layer.
#[derive(Debug, Clone)]
pub struct NewFileInput {
    pub path: String,
    pub content: Option<Vec<u8>>,
    pub is_directory: bool,
}

/// Normalize and validate a path (mirrors the SQL CHECK constraint).
fn normalize_path(path: &str) -> Result<String, MemoryFsError> {
    if path.is_empty() {
        return Err(MemoryFsError::InvalidPath("empty path".into()));
    }
    if path.contains('\0') {
        return Err(MemoryFsError::InvalidPath("null byte in path".into()));
    }
    if path.contains("//") {
        return Err(MemoryFsError::InvalidPath("double slash in path".into()));
    }
    if path.split('/').any(|s| s == "..") {
        return Err(MemoryFsError::InvalidPath("'..' in path".into()));
    }
    if !path.starts_with('/') {
        return Err(MemoryFsError::InvalidPath(format!(
            "path must start with '/', got '{path}'"
        )));
    }
    // Reserve underscore-prefixed segments for fs/_ action routes.
    if path
        .split('/')
        .any(|segment| segment.starts_with('_') && !segment.is_empty())
    {
        return Err(MemoryFsError::InvalidPath(
            "path segments starting with '_' are reserved".into(),
        ));
    }
    let trimmed = if path.len() > 1 && path.ends_with('/') {
        path.trim_end_matches('/').to_string()
    } else {
        path.to_string()
    };
    if trimmed.is_empty() {
        return Ok("/".to_string());
    }
    Ok(trimmed)
}

impl MemoryFsError {
    pub fn into_anyhow(self) -> anyhow::Error {
        match self {
            MemoryFsError::Other(e) => e,
            other => anyhow!(other.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::StorageBackend;
    use crate::storage::models::CreateMemoryRow;
    use everruns_provider::typed_id::MemoryId;
    use std::sync::Arc;

    fn make_db() -> Arc<StorageBackend> {
        Arc::new(StorageBackend::in_memory())
    }

    async fn seed_memory(db: &StorageBackend, is_readonly: bool) -> MemoryRow {
        db.create_memory(
            everruns_core::DEFAULT_ORG_ID,
            CreateMemoryRow {
                public_id: MemoryId::new().to_string(),
                name: format!(
                    "test-memory-{}",
                    if is_readonly { "readonly" } else { "manual" }
                ),
                description: None,
                scope: "org".to_string(),
                owner_agent_id: None,
                owner_user_id: None,
                source_type: if is_readonly {
                    "github".into()
                } else {
                    "manual".into()
                },
                source_config: if is_readonly {
                    serde_json::json!({
                        "provider": "github",
                        "repository": "acme/docs",
                        "branch": "main"
                    })
                } else {
                    serde_json::json!({})
                },
                is_readonly,
                sync_status: if is_readonly {
                    "pending".into()
                } else {
                    "idle".into()
                },
                owner_principal_id: None,
                resolved_owner_user_id: None,
            },
        )
        .await
        .expect("seed memory")
    }

    #[test]
    fn normalize_strips_trailing_slash() {
        assert_eq!(normalize_path("/a/b/").unwrap(), "/a/b");
        assert_eq!(normalize_path("/").unwrap(), "/");
    }

    #[test]
    fn normalize_rejects_relative_and_dotdot() {
        assert!(normalize_path("").is_err());
        assert!(normalize_path("a/b").is_err());
        assert!(normalize_path("/a/../b").is_err());
        assert!(normalize_path("/a//b").is_err());
        assert!(normalize_path("/a\0b").is_err());
    }

    #[tokio::test]
    async fn create_then_read_round_trip() {
        let db = make_db();
        let mem = seed_memory(&db, false).await;
        let svc = MemoryFileService::new(db.clone());

        svc.create_file(
            &mem,
            NewFileInput {
                path: "/docs/notes.md".into(),
                content: Some("hello memory".into()),
                is_directory: false,
            },
        )
        .await
        .expect("create");

        let read = svc.read_file(mem.id, "/docs/notes.md").await.expect("read");
        assert_eq!(read.content, b"hello memory");

        let listing = svc.list_directory(mem.id, "/docs").await.expect("list");
        assert_eq!(listing.len(), 1);
        assert_eq!(listing[0].path, "/docs/notes.md");
    }

    #[tokio::test]
    async fn list_root_lists_top_level_entries() {
        let db = make_db();
        let mem = seed_memory(&db, false).await;
        let svc = MemoryFileService::new(db.clone());

        svc.create_file(
            &mem,
            NewFileInput {
                path: "/a.txt".into(),
                content: Some("a".into()),
                is_directory: false,
            },
        )
        .await
        .unwrap();
        svc.create_file(
            &mem,
            NewFileInput {
                path: "/dir/b.txt".into(),
                content: Some("b".into()),
                is_directory: false,
            },
        )
        .await
        .unwrap();

        let rows = svc.list_directory(mem.id, "/").await.unwrap();
        let paths: Vec<&str> = rows.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"/a.txt"));
        assert!(paths.contains(&"/dir"));
    }

    #[tokio::test]
    async fn source_backed_memory_rejects_writes() {
        let db = make_db();
        let mem = seed_memory(&db, true).await;
        let svc = MemoryFileService::new(db.clone());

        let err = svc
            .create_file(
                &mem,
                NewFileInput {
                    path: "/forbidden.txt".into(),
                    content: Some("nope".into()),
                    is_directory: false,
                },
            )
            .await
            .expect_err("expected readonly");
        assert!(matches!(err, MemoryFsError::Readonly));

        let err = svc
            .update_file(&mem, "/forbidden.txt", b"nope".to_vec())
            .await
            .expect_err("expected readonly");
        assert!(matches!(err, MemoryFsError::Readonly));

        let err = svc
            .delete(&mem, "/anything", false)
            .await
            .expect_err("expected readonly");
        assert!(matches!(err, MemoryFsError::Readonly));
    }

    #[tokio::test]
    async fn delete_non_empty_directory_requires_recursive() {
        let db = make_db();
        let mem = seed_memory(&db, false).await;
        let svc = MemoryFileService::new(db.clone());

        svc.create_file(
            &mem,
            NewFileInput {
                path: "/d/a.txt".into(),
                content: Some("a".into()),
                is_directory: false,
            },
        )
        .await
        .unwrap();

        let err = svc
            .delete(&mem, "/d", false)
            .await
            .expect_err("expected err");
        assert!(matches!(err, MemoryFsError::DirectoryNotEmpty));

        let count = svc.delete(&mem, "/d", true).await.expect("recursive");
        assert!(count >= 2);
        assert!(svc.read_file(mem.id, "/d/a.txt").await.is_err());
    }

    #[tokio::test]
    async fn grep_returns_matched_files() {
        let db = make_db();
        let mem = seed_memory(&db, false).await;
        let svc = MemoryFileService::new(db.clone());

        svc.create_file(
            &mem,
            NewFileInput {
                path: "/notes/alpha.md".into(),
                content: Some("the quick brown fox".into()),
                is_directory: false,
            },
        )
        .await
        .unwrap();
        svc.create_file(
            &mem,
            NewFileInput {
                path: "/notes/beta.md".into(),
                content: Some("nothing here".into()),
                is_directory: false,
            },
        )
        .await
        .unwrap();

        let hits = svc.grep(mem.id, "quick", None).await.unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].path, "/notes/alpha.md");
    }

    #[tokio::test]
    async fn grep_filters_paths_with_globs() {
        let db = make_db();
        let mem = seed_memory(&db, false).await;
        let svc = MemoryFileService::new(db.clone());

        for path in ["/notes/alpha.md", "/notes/nested/beta.md", "/notes.txt"] {
            svc.create_file(
                &mem,
                NewFileInput {
                    path: path.into(),
                    content: Some("needle".into()),
                    is_directory: false,
                },
            )
            .await
            .unwrap();
        }

        let hits = svc
            .grep(mem.id, "needle", Some("notes/*.md"))
            .await
            .unwrap();
        let paths: Vec<_> = hits.into_iter().map(|hit| hit.path).collect();

        assert_eq!(paths, vec!["/notes/alpha.md"]);
    }

    #[tokio::test]
    async fn invalid_path_rejected() {
        let db = make_db();
        let mem = seed_memory(&db, false).await;
        let svc = MemoryFileService::new(db.clone());

        let err = svc
            .create_file(
                &mem,
                NewFileInput {
                    path: "no-leading-slash".into(),
                    content: None,
                    is_directory: false,
                },
            )
            .await
            .expect_err("expected invalid path");
        assert!(matches!(err, MemoryFsError::InvalidPath(_)));
    }
}
