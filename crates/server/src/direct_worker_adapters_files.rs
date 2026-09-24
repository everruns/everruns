//! `DirectWorkerAdapters`' session file operations.
//!
//! Bodies for the `WorkerAdapters` file methods. A trait impl cannot span
//! modules, so the work lives here as inherent methods and the trait impl in
//! `super::direct_worker_adapters` forwards to them — the same split
//! `grpc_service::worker` uses, and what keeps the parent file under the
//! source-size ratchet.

use super::direct_worker_adapters::{DirectWorkerAdapters, name_from_path, store_error};
use everruns_core::{FileInfo, FileStat, GrepMatch, GrepOptions, GrepSearchResult, SessionFile};
use everruns_provider::error::Result;
use everruns_provider::typed_id::SessionId;
use uuid::Uuid;

impl DirectWorkerAdapters {
    pub(crate) async fn read_file(
        &self,
        _org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFile>> {
        // Check virtual mounts first
        if let Some(registry) = &self.virtual_registry
            && let Some(vf) = registry.read_file(&session_id, path)
        {
            let now = chrono::Utc::now();
            let (content, encoding) = if vf.is_directory {
                (None, "text".to_string())
            } else {
                let (c, e) = SessionFile::encode_content(&vf.content);
                (Some(c), e)
            };
            return Ok(Some(SessionFile {
                id: uuid::Uuid::nil(),
                session_id,
                path: vf.path.clone(),
                name: name_from_path(&vf.path),
                content,
                encoding,
                is_directory: vf.is_directory,
                is_readonly: true,
                size_bytes: vf.content.len() as i64,
                created_at: now,
                updated_at: now,
            }));
        }

        let row = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to read file: {}", e);
                store_error("Failed to read file")
            })?;

        Ok(row.map(|r| {
            let (content, encoding) = if let Some(bytes) = &r.content {
                match String::from_utf8(bytes.clone()) {
                    Ok(text) => (Some(text), "text".to_string()),
                    Err(_) => {
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(bytes);
                        (Some(b64), "base64".to_string())
                    }
                }
            } else {
                (None, "text".to_string())
            };

            SessionFile {
                id: r.id,
                session_id: r.session_id.uuid(),
                path: r.path.clone(),
                name: name_from_path(&r.path),
                content,
                encoding,
                is_directory: r.is_directory,
                is_readonly: r.is_readonly,
                size_bytes: r.size_bytes,
                created_at: r.created_at,
                updated_at: r.updated_at,
            }
        }))
    }

    pub(crate) async fn write_file(
        &self,
        _org_id: i64,
        session_id: Uuid,
        path: &str,
        content: &str,
        encoding: &str,
    ) -> Result<SessionFile> {
        // Virtual files are readonly
        if let Some(registry) = &self.virtual_registry
            && registry.is_virtual_path(&session_id, path)
        {
            return Err(store_error(format!(
                "Cannot modify readonly file: {}",
                path
            )));
        }
        use crate::storage::models::{CreateSessionFileRow, UpdateSessionFile};

        let content_bytes = if encoding == "base64" {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD
                .decode(content)
                .map_err(|e| store_error(format!("Invalid base64 content: {}", e)))?
        } else {
            content.as_bytes().to_vec()
        };

        let existing = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check existing file: {}", e);
                store_error("Failed to write file")
            })?;

        // Quota check (TM-FS-008 / TM-DOS-005)
        {
            use crate::domains::session_files::limits::check_write_quota;
            let incoming = content_bytes.len() as i64;
            let existing_size = existing.as_ref().map(|f| f.size_bytes).unwrap_or(0);
            check_write_quota(&self.db, session_id, incoming, existing_size, &self.quota)
                .await
                .map_err(|e| store_error(e.to_string()))?;
        }

        let row = if existing.is_some() {
            let update = UpdateSessionFile {
                content: Some(content_bytes.clone()),
                ..Default::default()
            };
            self.db
                .update_session_file(session_id, path, update)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to update file: {}", e);
                    store_error("Failed to write file")
                })?
                .ok_or_else(|| store_error("File disappeared during update"))?
        } else {
            // Ensure parent directory exists
            if let Some(parent) = FileInfo::parent_path(path) {
                self.ensure_directory_exists(session_id, &parent).await?;
            }

            let create = CreateSessionFileRow {
                session_id: SessionId::from_uuid(session_id),
                path: path.to_string(),
                content: Some(content_bytes.clone()),
                is_directory: false,
                is_readonly: false,
            };
            match self.db.create_session_file(create).await {
                Ok(row) => row,
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("duplicate key")
                        || msg.contains("unique constraint")
                        || msg.contains("UNIQUE constraint")
                    {
                        // Race: file was created concurrently; fall back to update
                        let update = UpdateSessionFile {
                            content: Some(content_bytes.clone()),
                            ..Default::default()
                        };
                        self.db
                            .update_session_file(session_id, path, update)
                            .await
                            .map_err(|e| {
                                tracing::error!("Failed to update file after race: {}", e);
                                store_error("Failed to write file")
                            })?
                            .ok_or_else(|| store_error("File disappeared during update"))?
                    } else {
                        tracing::error!("Failed to create file: {}", e);
                        return Err(store_error("Failed to write file"));
                    }
                }
            }
        };

        Ok(SessionFile {
            id: row.id,
            session_id: row.session_id.uuid(),
            path: row.path.clone(),
            name: name_from_path(&row.path),
            content: Some(content.to_string()),
            encoding: encoding.to_string(),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }

    /// See the trait's declaration for why this takes eight arguments.
    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn write_file_if_content_matches(
        &self,
        _org_id: i64,
        session_id: Uuid,
        path: &str,
        expected_content: &str,
        expected_encoding: &str,
        content: &str,
        encoding: &str,
    ) -> Result<Option<SessionFile>> {
        if let Some(registry) = &self.virtual_registry
            && registry.is_virtual_path(&session_id, path)
        {
            return Err(store_error(format!(
                "Cannot modify readonly file: {}",
                path
            )));
        }
        use crate::storage::models::UpdateSessionFile;

        let expected_bytes = SessionFile::decode_content(expected_content, expected_encoding)
            .map_err(|e| store_error(format!("Invalid expected content encoding: {}", e)))?;
        let content_bytes = SessionFile::decode_content(content, encoding)
            .map_err(|e| store_error(format!("Invalid content encoding: {}", e)))?;

        // Fetch metadata only (no content blob) — content equality is enforced
        // atomically in SQL by update_session_file_if_content_matches below.
        let existing = self
            .db
            .get_session_file_info(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to check existing file: {}", e);
                store_error("Failed to write file")
            })?;
        let Some(existing) = existing else {
            return Ok(None);
        };
        if existing.is_directory || existing.is_readonly {
            return Ok(None);
        }

        {
            use crate::domains::session_files::limits::check_write_quota;
            check_write_quota(
                &self.db,
                session_id,
                content_bytes.len() as i64,
                existing.size_bytes,
                &self.quota,
            )
            .await
            .map_err(|e| store_error(e.to_string()))?;
        }

        let row = self
            .db
            .update_session_file_if_content_matches(
                session_id,
                path,
                expected_bytes,
                UpdateSessionFile {
                    content: Some(content_bytes),
                    ..Default::default()
                },
            )
            .await
            .map_err(|e| {
                tracing::error!("Failed to conditionally update file: {}", e);
                store_error("Failed to write file")
            })?;

        Ok(row.map(|row| {
            let (content, encoding) = if let Some(bytes) = row.content {
                SessionFile::encode_content(&bytes)
            } else {
                (String::new(), "text".to_string())
            };

            SessionFile {
                id: row.id,
                session_id: row.session_id.uuid(),
                path: row.path.clone(),
                name: name_from_path(&row.path),
                content: Some(content),
                encoding,
                is_directory: row.is_directory,
                is_readonly: row.is_readonly,
                size_bytes: row.size_bytes,
                created_at: row.created_at,
                updated_at: row.updated_at,
            }
        }))
    }

    pub(crate) async fn delete_file(
        &self,
        _org_id: i64,
        session_id: Uuid,
        path: &str,
        recursive: bool,
    ) -> Result<bool> {
        if let Some(registry) = &self.virtual_registry
            && registry.is_virtual_path(&session_id, path)
        {
            return Err(store_error(format!(
                "Cannot delete readonly file: {}",
                path
            )));
        }
        if recursive {
            let count = self
                .db
                .delete_session_file_recursive(session_id, path)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to delete file recursively: {}", e);
                    store_error("Failed to delete file")
                })?;
            Ok(count > 0)
        } else {
            self.db
                .delete_session_file(session_id, path)
                .await
                .map_err(|e| {
                    tracing::error!("Failed to delete file: {}", e);
                    store_error("Failed to delete file")
                })
        }
    }

    pub(crate) async fn list_directory(
        &self,
        _org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Vec<FileInfo>> {
        let rows = self
            .db
            .list_session_files(session_id, path)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("not found") || msg.contains("not a directory") {
                    tracing::debug!("Directory not found: {}", path);
                } else {
                    tracing::error!("Failed to list directory: {}", e);
                }
                store_error("Failed to list directory")
            })?;

        let mut entries: Vec<FileInfo> = rows
            .into_iter()
            .map(|r| FileInfo {
                id: r.id,
                session_id: r.session_id.uuid(),
                path: r.path.clone(),
                name: name_from_path(&r.path),
                is_directory: r.is_directory,
                is_readonly: r.is_readonly,
                size_bytes: r.size_bytes,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect();

        // Merge virtual entries (virtual wins on name conflict)
        if let Some(registry) = &self.virtual_registry {
            let virtual_entries = registry.list_directory(&session_id, path);
            let now = chrono::Utc::now();
            for vf in virtual_entries {
                let name = name_from_path(&vf.path);
                entries.retain(|e| e.name != name);
                entries.push(FileInfo {
                    id: uuid::Uuid::nil(),
                    session_id,
                    path: vf.path,
                    name,
                    is_directory: vf.is_directory,
                    is_readonly: true,
                    size_bytes: vf.size_bytes,
                    created_at: now,
                    updated_at: now,
                });
            }
            entries.sort_by(|a, b| {
                b.is_directory
                    .cmp(&a.is_directory)
                    .then_with(|| a.path.cmp(&b.path))
            });
        }

        Ok(entries)
    }

    pub(crate) async fn stat_file(
        &self,
        _org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<FileStat>> {
        // Check virtual mounts first
        if let Some(registry) = &self.virtual_registry
            && let Some(vf) = registry.read_file(&session_id, path)
        {
            return Ok(Some(FileStat {
                path: vf.path.clone(),
                name: name_from_path(&vf.path),
                is_directory: vf.is_directory,
                is_readonly: true,
                size_bytes: vf.content.len() as i64,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            }));
        }

        let row = self
            .db
            .get_session_file(session_id, path)
            .await
            .map_err(|e| {
                tracing::error!("Failed to stat file: {}", e);
                store_error("Failed to stat file")
            })?;

        Ok(row.map(|r| FileStat {
            path: r.path.clone(),
            name: name_from_path(&r.path),
            is_directory: r.is_directory,
            is_readonly: r.is_readonly,
            size_bytes: r.size_bytes,
            created_at: r.created_at,
            updated_at: r.updated_at,
        }))
    }

    pub(crate) async fn grep_files(
        &self,
        _org_id: i64,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
    ) -> Result<Vec<GrepMatch>> {
        let results = crate::domains::session_files::grep_session_files(
            &self.db,
            session_id,
            pattern,
            path_pattern,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to grep files: {}", e);
            store_error(format!("Failed to grep files: {}", e))
        })?;

        let mut matches: Vec<GrepMatch> = results.into_iter().flat_map(|r| r.matches).collect();

        // Also search virtual mounts with the shared canonical-path glob semantics.
        if let Some(registry) = &self.virtual_registry
            && let Ok(regex) = regex::Regex::new(pattern)
        {
            let path_matcher = path_pattern
                .map(everruns_core::session_path::GrepPathPattern::new)
                .transpose()
                .unwrap_or(None);
            let virtual_matches = registry.grep(&session_id, &regex, None, None, 512 * 1024);
            matches.extend(
                virtual_matches
                    .into_iter()
                    .filter(|vm| {
                        path_matcher
                            .as_ref()
                            .is_none_or(|matcher| matcher.is_match(&vm.path))
                    })
                    .map(|vm| GrepMatch {
                        path: vm.path,
                        line_number: vm.line_number,
                        line: vm.line,
                    }),
            );
        }

        Ok(matches)
    }

    pub(crate) async fn grep_files_with_options(
        &self,
        _org_id: i64,
        session_id: Uuid,
        pattern: &str,
        options: &GrepOptions,
    ) -> Result<GrepSearchResult> {
        crate::domains::session_files::service::grep_session_files_with_options(
            &self.db,
            self.virtual_registry.as_deref(),
            session_id,
            pattern,
            options,
        )
        .await
        .map_err(|error| store_error(format!("Failed to grep files: {error}")))
    }

    pub(crate) async fn create_directory(
        &self,
        _org_id: i64,
        session_id: Uuid,
        path: &str,
    ) -> Result<FileInfo> {
        use crate::storage::models::CreateSessionFileRow;

        let create = CreateSessionFileRow {
            session_id: SessionId::from_uuid(session_id),
            path: path.to_string(),
            content: None,
            is_directory: true,
            is_readonly: false,
        };

        let row = self.db.create_session_file(create).await.map_err(|e| {
            tracing::error!("Failed to create directory: {}", e);
            store_error("Failed to create directory")
        })?;

        Ok(FileInfo {
            id: row.id,
            session_id: row.session_id.uuid(),
            path: row.path.clone(),
            name: name_from_path(&row.path),
            is_directory: row.is_directory,
            is_readonly: row.is_readonly,
            size_bytes: row.size_bytes,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}
