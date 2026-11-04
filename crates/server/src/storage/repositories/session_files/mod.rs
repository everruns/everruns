// PostgreSQL repository: Session Files (virtual filesystem)
//
// Content offload: when an object-storage blob backend is configured
// (knowledge/runtime-resources/object-storage.md), file *content* bytes are stored in the object
// store and the `workspace_files.content` column is left NULL; a pointer +
// content hash live in `workspace_file_blobs`. Metadata (path, size, flags,
// tree) always stays in PostgreSQL. When no blob backend is configured the
// methods fall back to the original inline `BYTEA` storage with zero overhead.
//
// Decision: objects are never deleted inline. Delete, overwrite and CAS only
// drop or repoint the sidecar row; the blob GC (background/blob_gc.rs) removes
// an object once no pointer references it. That lets copy and fork share one
// object between files instead of duplicating bytes. Every pointer change
// holds the owning `workspace_files` row lock, and copies take a share lock on
// the source row, so a copy can never point at an object the GC already
// decided was unreferenced.

pub(super) mod rows;
use rows::*;

use super::Database;
use crate::storage::blob_store::{BlobMetadata, content_sha256, workspace_file_key};
use anyhow::Result;
use uuid::Uuid;

/// Disaster-recovery metadata stamped onto each offloaded file object. Lets a recovery tool rebuild the `workspace_files`
/// row from the object alone.
fn file_blob_metadata(
    workspace_id: Uuid,
    file_id: Uuid,
    path: &str,
    size_bytes: i64,
    sha: &str,
) -> BlobMetadata {
    BlobMetadata::new(
        "workspace_file",
        serde_json::json!({
            "v": 1,
            "workspace_id": workspace_id,
            "file_id": file_id,
            "path": path,
            "size_bytes": size_bytes,
            "content_sha256": sha,
        }),
    )
}

impl Database {
    // ============================================
    // Session Files (virtual filesystem)
    // ============================================

    /// Fill `row.content` from the object store when the file is offloaded.
    ///
    /// No-op (single branch) when no blob backend is configured or when the
    /// content is already present inline, so default deployments pay nothing.
    async fn materialize_file_content(&self, row: &mut SessionFileRow) -> Result<()> {
        if row.is_directory || row.content.is_some() {
            return Ok(());
        }
        let Some(blob) = self.blob_store() else {
            return Ok(());
        };
        let pointer: Option<(String, String)> = sqlx::query_as(
            "SELECT blob_key, content_sha256 FROM workspace_file_blobs WHERE file_id = $1",
        )
        .bind(row.id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((key, expected_sha)) = pointer {
            if row.size_bytes > MAX_LOADED_FILE_BYTES {
                return Err(FileTooLargeToLoad {
                    path: row.path.clone(),
                    size_bytes: row.size_bytes,
                    limit: MAX_LOADED_FILE_BYTES,
                }
                .into());
            }
            // A sidecar pointer exists, so a missing object means data loss —
            // surface it as an error rather than silently returning empty content.
            let bytes = blob
                .get(&key)
                .await?
                .ok_or_else(|| anyhow::anyhow!("offloaded file blob missing for {key}"))?;
            let actual_sha = content_sha256(&bytes);
            anyhow::ensure!(
                actual_sha == expected_sha,
                "offloaded file blob hash mismatch for {key}: expected {expected_sha}, got {actual_sha}"
            );
            row.content = Some(bytes);
        }
        Ok(())
    }

    /// Like [`Self::materialize_file_content`], but leaves `content` empty for
    /// files too large to load instead of failing. For results of move and
    /// copy, where the caller asked for the operation rather than the bytes.
    async fn materialize_loadable_file_content(&self, row: &mut SessionFileRow) -> Result<()> {
        if row.size_bytes > MAX_LOADED_FILE_BYTES && row.content.is_none() {
            return Ok(());
        }
        self.materialize_file_content(row).await
    }

    /// Create a new file or directory in the session virtual filesystem
    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);
        let workspace_id = input.session_id.uuid();

        // Offload path: configured blob backend + a file (not directory) with
        // content. Empty files and directories stay metadata-only.
        if let (Some(blob), Some(content)) = (self.blob_store(), input.content.as_ref())
            && !input.is_directory
        {
            let file_id = Uuid::now_v7();
            let sha = content_sha256(content);
            let key = workspace_file_key(workspace_id, file_id, &sha, Uuid::now_v7());

            // Write the blob first; if the row insert then fails (e.g. duplicate
            // path) we clean the orphan object up rather than leaving it behind.
            blob.put(
                &key,
                content.clone(),
                &file_blob_metadata(workspace_id, file_id, &input.path, size_bytes, &sha),
            )
            .await?;

            // Insert the file row and its sidecar pointer in one transaction so a
            // failure can't leave a row with NULL content and no pointer. The
            // object store is not transactional, so we delete the orphan blob on
            // any failure (incl. a duplicate-path unique violation).
            let insert = async {
                let mut tx = self.pool.begin().await?;
                let row = sqlx::query_as::<_, SessionFileRow>(
                    r#"
                    INSERT INTO workspace_files (id, workspace_id, path, content, is_directory, is_readonly, size_bytes)
                    VALUES ($1, $2, $3, NULL, FALSE, $4, $5)
                    RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
                    "#,
                )
                .bind(file_id)
                .bind(workspace_id)
                .bind(&input.path)
                .bind(input.is_readonly)
                .bind(size_bytes)
                .fetch_one(&mut *tx)
                .await?;
                sqlx::query(
                    r#"
                    INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
                    VALUES ($1, $2, $3, $4)
                    "#,
                )
                .bind(file_id)
                .bind(&key)
                .bind(&sha)
                .bind(size_bytes)
                .execute(&mut *tx)
                .await?;
                tx.commit().await?;
                anyhow::Ok(row)
            }
            .await;

            let mut row = match insert {
                Ok(row) => row,
                Err(e) => {
                    let _ = blob.delete(&key).await;
                    return Err(e);
                }
            };

            row.content = Some(content.clone());
            return Ok(row);
        }

        // Inline path (default backend, directories, or empty content).
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            INSERT INTO workspace_files (workspace_id, path, content, is_directory, is_readonly, size_bytes)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(input.session_id)
        .bind(&input.path)
        .bind(&input.content)
        .bind(input.is_directory)
        .bind(input.is_readonly)
        .bind(size_bytes)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create or replace a file from a byte stream, without holding it in
    /// memory when object storage is configured. Fails with
    /// [`BlobTooLarge`](crate::storage::blob_store::BlobTooLarge) past
    /// `max_bytes`. `is_readonly` applies to a new file, and to an existing
    /// one only when set.
    ///
    /// Without a blob store the stream is collected (bounded by `max_bytes`)
    /// and stored inline, so the inline backend keeps its own limits.
    pub async fn write_session_file_stream(
        &self,
        workspace_id: Uuid,
        path: &str,
        mut body: crate::storage::blob_store::BlobByteStream,
        max_bytes: u64,
        is_readonly: Option<bool>,
    ) -> Result<SessionFileInfoRow> {
        use crate::storage::blob_store::{BlobTooLarge, workspace_file_stream_key};
        use futures::StreamExt;

        let existing = self.get_session_file_info(workspace_id, path).await?;
        if existing.as_ref().is_some_and(|row| row.is_directory) {
            anyhow::bail!("Cannot write a directory: {path}");
        }

        let Some(blob) = self.blob_store() else {
            let mut content = Vec::new();
            while let Some(chunk) = body.next().await {
                let chunk = chunk?;
                if (content.len() + chunk.len()) as u64 > max_bytes {
                    return Err(BlobTooLarge { limit: max_bytes }.into());
                }
                content.extend_from_slice(&chunk);
            }
            let row = match existing {
                Some(_) => self
                    .update_session_file(
                        workspace_id,
                        path,
                        UpdateSessionFile {
                            content: Some(content),
                            is_readonly,
                        },
                    )
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("File not found: {path}"))?,
                None => {
                    self.create_session_file(CreateSessionFileRow {
                        session_id:
                            crate::kernel_imports::contracts::typed_id::SessionId::from_uuid(
                                workspace_id,
                            ),
                        path: path.to_string(),
                        content: Some(content),
                        is_directory: false,
                        is_readonly: is_readonly.unwrap_or(false),
                    })
                    .await?
                }
            };
            return Ok(SessionFileInfoRow {
                id: row.id,
                session_id: row.session_id,
                path: row.path,
                is_directory: row.is_directory,
                is_readonly: row.is_readonly,
                size_bytes: row.size_bytes,
                created_at: row.created_at,
                updated_at: row.updated_at,
            });
        };

        let file_id = existing.as_ref().map_or_else(Uuid::now_v7, |row| row.id);
        let key = workspace_file_stream_key(workspace_id, file_id, Uuid::now_v7());
        // The hash is not known until the upload ends, so the recovery record
        // carries identity only; size comes from the object itself.
        let metadata = BlobMetadata::new(
            "workspace_file",
            serde_json::json!({
                "v": 1,
                "workspace_id": workspace_id,
                "file_id": file_id,
                "path": path,
            }),
        );
        let stored = blob.put_stream(&key, body, max_bytes, &metadata).await?;
        let size_bytes = stored.size_bytes as i64;

        // An object left behind by a failure below has no pointer, so the
        // blob GC reclaims it.
        let mut tx = self.pool.begin().await?;
        let row = match existing {
            Some(_) => sqlx::query_as::<_, SessionFileInfoRow>(
                    r#"
                    UPDATE workspace_files
                    SET content = NULL,
                        size_bytes = $4,
                        is_readonly = COALESCE($5, is_readonly)
                    WHERE id = $1 AND workspace_id = $2 AND path = $3 AND is_directory = FALSE
                    RETURNING id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                    "#,
                )
                .bind(file_id)
                .bind(workspace_id)
                .bind(path)
                .bind(size_bytes)
                .bind(is_readonly)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or_else(|| anyhow::anyhow!("File changed while uploading: {path}"))?,
            None => {
                sqlx::query_as::<_, SessionFileInfoRow>(
                    r#"
                    INSERT INTO workspace_files (id, workspace_id, path, content, is_directory, is_readonly, size_bytes)
                    VALUES ($1, $2, $3, NULL, FALSE, $4, $5)
                    RETURNING id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                    "#,
                )
                .bind(file_id)
                .bind(workspace_id)
                .bind(path)
                .bind(is_readonly.unwrap_or(false))
                .bind(size_bytes)
                .fetch_one(&mut *tx)
                .await?
            }
        };
        sqlx::query(
            r#"
            INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (file_id) DO UPDATE
                SET blob_key = EXCLUDED.blob_key,
                    content_sha256 = EXCLUDED.content_sha256,
                    size_bytes = EXCLUDED.size_bytes,
                    updated_at = NOW()
            "#,
        )
        .bind(file_id)
        .bind(&key)
        .bind(&stored.content_sha256)
        .bind(size_bytes)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }

    /// Get a file by session and path
    pub async fn get_session_file(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let mut row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE workspace_id = $1 AND path = $2
            "#,
        )
        .bind(session_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row.as_mut() {
            self.materialize_file_content(row).await?;
        }
        Ok(row)
    }

    /// Get file metadata (no content) — for lightweight pre-checks before atomic updates.
    pub async fn get_session_file_info(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<Option<SessionFileInfoRow>> {
        let row = sqlx::query_as::<_, SessionFileInfoRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE workspace_id = $1 AND path = $2
            "#,
        )
        .bind(session_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Open a file's bytes as a stream, optionally only `range` (half-open,
    /// already clamped to the file size). `Ok(None)` when the path does not
    /// exist; a directory yields an empty body.
    ///
    /// Offloaded content streams straight from the object store, so even a
    /// multi-gigabyte recording never sits in memory. Inline content reads
    /// only the requested slice of the column.
    pub async fn open_session_file_content(
        &self,
        session_id: Uuid,
        path: &str,
        range: Option<std::ops::Range<u64>>,
    ) -> Result<Option<SessionFileContent>> {
        use futures::StreamExt;

        let Some(info) = self.get_session_file_info(session_id, path).await? else {
            return Ok(None);
        };
        let empty = || futures::stream::empty().boxed();
        if info.is_directory {
            return Ok(Some(SessionFileContent {
                info,
                content_sha256: None,
                body: empty(),
            }));
        }
        let pointer: Option<(String, String)> = sqlx::query_as(
            "SELECT blob_key, content_sha256 FROM workspace_file_blobs WHERE file_id = $1",
        )
        .bind(info.id)
        .fetch_optional(&self.pool)
        .await?;
        if let Some((key, sha)) = pointer {
            let blob = self.blob_store().ok_or_else(|| {
                anyhow::anyhow!("file {path} is offloaded but no blob store is configured")
            })?;
            // A zero-length range never reaches the store (S3 rejects it).
            let body = match range {
                Some(ref r) if r.is_empty() => empty(),
                _ => blob
                    .get_stream(&key, range)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("offloaded file blob missing for {key}"))?,
            };
            return Ok(Some(SessionFileContent {
                info,
                content_sha256: Some(sha),
                body,
            }));
        }

        let (offset, length) = match range {
            Some(r) => (r.start as i64, (r.end - r.start) as i64),
            None => (0, info.size_bytes),
        };
        // SQL substring is 1-based.
        let (bytes,): (Option<Vec<u8>>,) = sqlx::query_as(
            "SELECT substring(content FROM $2::int FOR $3::int) FROM workspace_files WHERE id = $1",
        )
        .bind(info.id)
        .bind(offset + 1)
        .bind(length)
        .fetch_one(&self.pool)
        .await?;
        let body = match bytes {
            Some(bytes) if !bytes.is_empty() => {
                futures::stream::once(async move { Ok(bytes::Bytes::from(bytes)) }).boxed()
            }
            _ => empty(),
        };
        Ok(Some(SessionFileContent {
            info,
            content_sha256: None,
            body,
        }))
    }

    // Intentionally no `get_session_file_by_id(id)`: a bare `WHERE id = $1`
    // lookup that returns file *content* with no session scoping is a
    // tenant-isolation hazard (TM-TENANT-012). It had no callers, so it was
    // removed rather than left as a footgun for a future cross-session reader.
    // All real reads scope by `workspace_id` (the session id) — see
    // `get_session_file` / `get_session_file_info`.

    /// List files in a directory (immediate children only, no content)
    pub async fn list_session_files(
        &self,
        session_id: Uuid,
        parent_path: &str,
    ) -> Result<Vec<SessionFileInfoRow>> {
        // Root directory case
        let pattern = if parent_path == "/" {
            "^/[^/]+$".to_string()
        } else {
            format!("^{}/[^/]+$", regex::escape(parent_path))
        };

        let rows = sqlx::query_as::<_, SessionFileInfoRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE workspace_id = $1 AND path ~ $2
            ORDER BY is_directory DESC, path ASC
            "#,
        )
        .bind(session_id)
        .bind(&pattern)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// List all files in a session (recursive, no content)
    pub async fn list_all_session_files(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileInfoRow>> {
        let rows = sqlx::query_as::<_, SessionFileInfoRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE workspace_id = $1
            ORDER BY path ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Update a session file (content and/or metadata)
    pub async fn update_session_file(
        &self,
        session_id: Uuid,
        path: &str,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        // Offload path: a content update with a configured blob backend. Write
        // the new blob first, then update the row + sidecar in one transaction,
        // so a failed object write never leaves the row pointing at missing
        // content. The blob key embeds the content SHA-256, so each revision
        // writes to a distinct, immutable object instead of overwriting in place.
        if let (Some(blob), Some(content)) = (self.blob_store(), input.content.as_ref()) {
            let size_bytes = content.len() as i64;

            let Some((file_id,)) = sqlx::query_as::<_, (Uuid,)>(
                "SELECT id FROM workspace_files WHERE workspace_id = $1 AND path = $2 AND is_directory = FALSE",
            )
            .bind(session_id)
            .bind(path)
            .fetch_optional(&self.pool)
            .await?
            else {
                return Ok(None);
            };

            let sha = content_sha256(content);
            let key = workspace_file_key(session_id, file_id, &sha, Uuid::now_v7());
            blob.put(
                &key,
                content.clone(),
                &file_blob_metadata(session_id, file_id, path, size_bytes, &sha),
            )
            .await?;

            // Only hold the database connection while changing the pointer.
            // Locking the same file ID also prevents a delete/recreate race
            // from attaching the uploaded object to a different file.
            let mut tx = self.pool.begin().await?;
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT TRUE FROM workspace_files WHERE id = $1 AND workspace_id = $2 AND path = $3 AND is_directory = FALSE FOR UPDATE",
            )
            .bind(file_id)
            .bind(session_id)
            .bind(path)
            .fetch_optional(&mut *tx)
            .await?
            .is_some();
            if !exists {
                drop(tx);
                blob.delete(&key).await?;
                return Ok(None);
            }

            sqlx::query(
                r#"
                INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (file_id) DO UPDATE
                    SET blob_key = EXCLUDED.blob_key,
                        content_sha256 = EXCLUDED.content_sha256,
                        size_bytes = EXCLUDED.size_bytes,
                        updated_at = NOW()
                "#,
            )
            .bind(file_id)
            .bind(&key)
            .bind(&sha)
            .bind(size_bytes)
            .execute(&mut *tx)
            .await?;
            let mut row = sqlx::query_as::<_, SessionFileRow>(
                r#"
                UPDATE workspace_files
                SET content = NULL,
                    is_readonly = COALESCE($3, is_readonly),
                    size_bytes = $4
                WHERE workspace_id = $1 AND path = $2 AND is_directory = FALSE
                RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
                "#,
            )
            .bind(session_id)
            .bind(path)
            .bind(input.is_readonly)
            .bind(size_bytes)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;

            if let Some(row) = row.as_mut() {
                row.content = Some(content.clone());
            }
            return Ok(row);
        }

        // Inline path / metadata-only update (no content change).
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64);

        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE workspace_files
            SET
                content = COALESCE($3, content),
                is_readonly = COALESCE($4, is_readonly),
                size_bytes = COALESCE($5, size_bytes)
            WHERE workspace_id = $1 AND path = $2 AND is_directory = FALSE
            RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(path)
        .bind(&input.content)
        .bind(input.is_readonly)
        .bind(size_bytes)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn update_session_file_if_content_matches(
        &self,
        session_id: Uuid,
        path: &str,
        expected_content: Vec<u8>,
        input: UpdateSessionFile,
    ) -> Result<Option<SessionFileRow>> {
        // Offload path: compare against the offloaded (or still-inline) current
        // content under a row lock, then offload the new content atomically.
        if let Some(blob) = self.blob_store() {
            let Some(new_content) = input.content.clone() else {
                // CAS is only meaningful for a content write.
                return Ok(None);
            };

            let mut tx = self.pool.begin().await?;
            let existing = sqlx::query_as::<_, SessionFileRow>(
                r#"
                SELECT id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM workspace_files
                WHERE workspace_id = $1 AND path = $2
                FOR UPDATE
                "#,
            )
            .bind(session_id)
            .bind(path)
            .fetch_optional(&mut *tx)
            .await?;

            let Some(existing) = existing else {
                tx.rollback().await?;
                return Ok(None);
            };
            if existing.is_directory || existing.is_readonly {
                tx.rollback().await?;
                return Ok(None);
            }

            // Compare by content hash rather than fetching the (possibly large)
            // offloaded bytes over the network while holding the row lock. For an
            // offloaded file the sidecar holds the current SHA-256; for a
            // still-inline file (pre-offload row) compare the column directly.
            let expected_sha = content_sha256(&expected_content);
            let current_sha: Option<(String,)> = sqlx::query_as(
                "SELECT content_sha256 FROM workspace_file_blobs WHERE file_id = $1",
            )
            .bind(existing.id)
            .fetch_optional(&mut *tx)
            .await?;

            let matches = match (&current_sha, &existing.content) {
                (Some((sha,)), _) => *sha == expected_sha,
                (None, Some(content)) => *content == expected_content,
                (None, None) => expected_content.is_empty(),
            };
            if !matches {
                tx.rollback().await?;
                return Ok(None);
            }

            let size_bytes = new_content.len() as i64;
            let sha = content_sha256(&new_content);
            let key = workspace_file_key(session_id, existing.id, &sha, Uuid::now_v7());
            blob.put(
                &key,
                new_content.clone(),
                &file_blob_metadata(session_id, existing.id, path, size_bytes, &sha),
            )
            .await?;
            sqlx::query(
                r#"
                INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
                VALUES ($1, $2, $3, $4)
                ON CONFLICT (file_id) DO UPDATE
                    SET blob_key = EXCLUDED.blob_key,
                        content_sha256 = EXCLUDED.content_sha256,
                        size_bytes = EXCLUDED.size_bytes,
                        updated_at = NOW()
                "#,
            )
            .bind(existing.id)
            .bind(&key)
            .bind(&sha)
            .bind(size_bytes)
            .execute(&mut *tx)
            .await?;

            let mut row = sqlx::query_as::<_, SessionFileRow>(
                r#"
                UPDATE workspace_files
                SET content = NULL,
                    size_bytes = $3,
                    is_readonly = COALESCE($4, is_readonly)
                WHERE id = $1 AND workspace_id = $2
                RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
                "#,
            )
            .bind(existing.id)
            .bind(session_id)
            .bind(size_bytes)
            .bind(input.is_readonly)
            .fetch_optional(&mut *tx)
            .await?;
            tx.commit().await?;

            if let Some(row) = row.as_mut() {
                row.content = Some(new_content);
            }
            return Ok(row);
        }

        // Inline path: atomic compare-and-set against the content column.
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64);

        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE workspace_files
            SET
                content = COALESCE($4, content),
                is_readonly = COALESCE($5, is_readonly),
                size_bytes = COALESCE($6, size_bytes)
            WHERE workspace_id = $1
              AND path = $2
              AND is_directory = FALSE
              AND is_readonly = FALSE
              AND COALESCE(content, '\x'::bytea) = $3
            RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(path)
        .bind(&expected_content)
        .bind(&input.content)
        .bind(input.is_readonly)
        .bind(size_bytes)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Delete a file or directory (directories must be empty)
    pub async fn delete_session_file(&self, session_id: Uuid, path: &str) -> Result<bool> {
        // The cascade drops the sidecar pointer; the blob GC reclaims the
        // object once no other file (copy, fork) points at it.
        let result =
            sqlx::query("DELETE FROM workspace_files WHERE workspace_id = $1 AND path = $2")
                .bind(session_id)
                .bind(path)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected() > 0)
    }

    /// Delete a directory and all its contents recursively
    pub async fn delete_session_file_recursive(&self, session_id: Uuid, path: &str) -> Result<u64> {
        // Delete the directory and all paths that start with it
        let pattern = if path == "/" {
            // Delete all files in session
            "^/".to_string()
        } else {
            format!("^{}(/|$)", regex::escape(path))
        };

        let result =
            sqlx::query("DELETE FROM workspace_files WHERE workspace_id = $1 AND path ~ $2")
                .bind(session_id)
                .bind(&pattern)
                .execute(&self.pool)
                .await?;

        Ok(result.rows_affected())
    }

    /// Move/rename a file or directory
    pub async fn move_session_file(
        &self,
        session_id: Uuid,
        old_path: &str,
        new_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        anyhow::ensure!(
            old_path != "/"
                && new_path != "/"
                && old_path != new_path
                && !new_path.starts_with(&format!("{old_path}/")),
            "Invalid move destination"
        );
        // Blob keys are addressed by file id (not path), so a move only rewrites
        // path metadata — the object and its sidecar pointer are untouched.
        let mut tx = self.pool.begin().await?;

        // First, check if source exists and is a directory
        let source = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE workspace_id = $1 AND path = $2
            FOR UPDATE
            "#,
        )
        .bind(session_id)
        .bind(old_path)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(source) = source else {
            return Ok(None);
        };

        // A directory's destination must be absent even when its parent row is
        // implicit. Never merge trees or relocate a readonly descendant.
        let (blocked,): (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM workspace_files WHERE workspace_id = $1 AND
             ((path = $2 OR starts_with(path, $2 || '/')) OR
              (is_readonly AND (path = $3 OR starts_with(path, $3 || '/')))))",
        )
        .bind(session_id)
        .bind(new_path)
        .bind(old_path)
        .fetch_one(&mut *tx)
        .await?;
        anyhow::ensure!(
            !blocked,
            "Destination exists or source contains readonly files"
        );

        if source.is_directory {
            // Move all children by replacing the prefix
            let old_prefix = format!("{}/", old_path);
            let new_prefix = format!("{}/", new_path);

            // THREAT[TM-FS-005]: inspect the rows actually moved, including a
            // child marked readonly after the preflight check. An error rolls
            // back the entire tree; never commit a partial protected move.
            let readonly = sqlx::query_scalar::<_, bool>(
                r#"
                WITH moved AS (
                    UPDATE workspace_files
                    SET path = $3 || substring(path from char_length($2) + 1)
                    WHERE workspace_id = $1 AND starts_with(path, $2)
                    RETURNING is_readonly
                )
                SELECT COALESCE(bool_or(is_readonly), FALSE) FROM moved
                "#,
            )
            .bind(session_id)
            .bind(&old_prefix)
            .bind(&new_prefix)
            .fetch_one(&mut *tx)
            .await?;
            anyhow::ensure!(!readonly, "Cannot move readonly descendants");
        }

        // Move the file/directory itself
        let mut row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE workspace_files
            SET path = $3
            WHERE workspace_id = $1 AND path = $2
            RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(old_path)
        .bind(new_path)
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;

        if let Some(row) = row.as_mut() {
            self.materialize_loadable_file_content(row).await?;
        }
        Ok(row)
    }

    /// Copy a file (directories not supported yet)
    ///
    /// The copy shares the source's stored object (see the header): only the
    /// row and, when offloaded, the sidecar pointer are duplicated, so copying
    /// a large recording costs no bytes. Inline content is copied as before.
    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        src_path: &str,
        dst_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let mut tx = self.pool.begin().await?;
        // Share lock: an overwrite or delete of the source waits until the new
        // pointer is committed, so the GC always sees one of the two pointers.
        let source: Option<(Uuid,)> = sqlx::query_as(
            "SELECT id FROM workspace_files WHERE workspace_id = $1 AND path = $2 AND is_directory = FALSE FOR SHARE",
        )
        .bind(session_id)
        .bind(src_path)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((source_id,)) = source else {
            return Ok(None);
        };
        let mut row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            INSERT INTO workspace_files (workspace_id, path, content, is_directory, is_readonly, size_bytes)
            SELECT workspace_id, $2, content, FALSE, is_readonly, size_bytes
            FROM workspace_files
            WHERE id = $1
            RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(source_id)
        .bind(dst_path)
        .fetch_one(&mut *tx)
        .await?;
        sqlx::query(
            r#"
            INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
            SELECT $2, blob_key, content_sha256, size_bytes
            FROM workspace_file_blobs
            WHERE file_id = $1
            "#,
        )
        .bind(source_id)
        .bind(row.id)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;

        self.materialize_loadable_file_content(&mut row).await?;
        Ok(Some(row))
    }

    /// Copy every file and directory of one workspace into another (session
    /// fork), skipping paths the destination already has. Offloaded files share
    /// the source objects, so a fork never duplicates stored bytes.
    pub async fn copy_all_session_files(
        &self,
        source_workspace_id: Uuid,
        destination_workspace_id: Uuid,
    ) -> Result<u64> {
        let mut tx = self.pool.begin().await?;
        // Same share lock as `copy_session_file`, for the whole source tree.
        sqlx::query("SELECT id FROM workspace_files WHERE workspace_id = $1 FOR SHARE")
            .bind(source_workspace_id)
            .execute(&mut *tx)
            .await?;
        let copied: Vec<(Uuid, Uuid)> = sqlx::query_as(
            r#"
            WITH source AS (
                SELECT id AS source_id, uuidv7() AS new_id, path, content, is_directory, is_readonly, size_bytes
                FROM workspace_files
                WHERE workspace_id = $1
            ),
            inserted AS (
                INSERT INTO workspace_files (id, workspace_id, path, content, is_directory, is_readonly, size_bytes)
                SELECT new_id, $2, path, content, is_directory, is_readonly, size_bytes
                FROM source
                ON CONFLICT (workspace_id, path) DO NOTHING
                RETURNING id
            )
            SELECT source.source_id, source.new_id
            FROM source
            JOIN inserted ON inserted.id = source.new_id
            "#,
        )
        .bind(source_workspace_id)
        .bind(destination_workspace_id)
        .fetch_all(&mut *tx)
        .await?;
        if !copied.is_empty() {
            let (source_ids, new_ids): (Vec<Uuid>, Vec<Uuid>) = copied.iter().copied().unzip();
            sqlx::query(
                r#"
                INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
                SELECT pair.new_id, b.blob_key, b.content_sha256, b.size_bytes
                FROM UNNEST($1::uuid[], $2::uuid[]) AS pair(source_id, new_id)
                JOIN workspace_file_blobs b ON b.file_id = pair.source_id
                "#,
            )
            .bind(&source_ids)
            .bind(&new_ids)
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(copied.len() as u64)
    }

    /// Search file contents using regex pattern (grep-like)
    pub async fn grep_session_files(
        &self,
        session_id: Uuid,
        pattern: &str,
        path_pattern: Option<&str>,
        excluded_path_prefix: Option<&str>,
        max_file_bytes: i64,
    ) -> Result<Vec<SessionFileInfoRow>> {
        // Offload path: content is in the object store, so PostgreSQL cannot grep
        // it. Return size/path-filtered candidates; the service-layer scan fetches
        // each blob and matches lines (TM-DOS-008 per-file and total-scan caps
        // still bound the work).
        if self.blob_store().is_some() {
            // The candidate query is path-pattern agnostic here: PostgreSQL
            // returns size-bounded rows and the service layer applies the glob
            // filter before fetching blob content, so both cases share one query.
            let rows = sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM workspace_files
                WHERE workspace_id = $1
                    AND is_directory = FALSE
                    AND size_bytes <= $2
                    AND ($3::text IS NULL OR (path <> $3 AND path NOT LIKE $3 || '/%'))
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(max_file_bytes)
            .bind(excluded_path_prefix)
            .fetch_all(&self.pool)
            .await?;
            return Ok(rows);
        }

        // Inline path: TM-DOS-008 size_bytes filter keeps Postgres from scanning
        // large files; content match happens in-database.
        // Glob syntax is not PostgreSQL regex syntax. When a path filter is
        // present, return cheap metadata candidates so the service can apply the
        // exact shared matcher before fetching or scanning any content.
        let rows = if path_pattern.is_some() {
            sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM workspace_files
                WHERE workspace_id = $1
                    AND is_directory = FALSE
                    AND size_bytes <= $2
                    AND ($3::text IS NULL OR (path <> $3 AND path NOT LIKE $3 || '/%'))
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(max_file_bytes)
            .bind(excluded_path_prefix)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM workspace_files
                WHERE workspace_id = $1
                    AND is_directory = FALSE
                    AND size_bytes <= $2
                    AND ($3::text IS NULL OR (path <> $3 AND path NOT LIKE $3 || '/%'))
                    AND convert_from(content, 'UTF8') ~ $4
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(max_file_bytes)
            .bind(excluded_path_prefix)
            .bind(pattern)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    /// Check if a path exists
    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT TRUE FROM workspace_files WHERE workspace_id = $1 AND path = $2",
        )
        .bind(session_id)
        .bind(path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    /// Check if a directory has any children
    pub async fn session_directory_has_children(
        &self,
        session_id: Uuid,
        path: &str,
    ) -> Result<bool> {
        let pattern = if path == "/" {
            "^/[^/]+".to_string()
        } else {
            format!("^{}/[^/]+", regex::escape(path))
        };

        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT TRUE FROM workspace_files WHERE workspace_id = $1 AND path ~ $2 LIMIT 1",
        )
        .bind(session_id)
        .bind(&pattern)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    /// Check if a path or its subtree contains any readonly files
    pub async fn has_readonly_session_files(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let pattern = if path == "/" {
            "^/".to_string()
        } else {
            format!("^{}(/|$)", regex::escape(path))
        };

        let result: Option<(bool,)> = sqlx::query_as(
            "SELECT TRUE FROM workspace_files WHERE workspace_id = $1 AND path ~ $2 AND is_readonly = true LIMIT 1",
        )
        .bind(session_id)
        .bind(&pattern)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    /// Sum of size_bytes across all non-directory files in a session.
    pub async fn total_session_file_bytes(&self, session_id: Uuid) -> Result<i64> {
        let row: (i64,) = sqlx::query_as(
            "SELECT COALESCE(SUM(size_bytes), 0)::bigint FROM workspace_files WHERE workspace_id = $1 AND is_directory = false",
        )
        .bind(session_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row.0)
    }

    /// Load all non-directory files with content for a session.
    ///
    /// Inline backend: a single SQL query. With the blob backend enabled, the
    /// row query is followed by a per-file sidecar lookup + object fetch to
    /// materialize offloaded content, so this is no longer a single round trip.
    pub async fn load_all_session_files_with_content(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileRow>> {
        let mut rows = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE workspace_id = $1 AND is_directory = false
            ORDER BY path ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        // Materialize offloaded content (no-op for the inline backend).
        if self.blob_store().is_some() {
            for row in rows.iter_mut() {
                self.materialize_file_content(row).await?;
            }
        }

        Ok(rows)
    }
}
