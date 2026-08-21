// PostgreSQL repository: Session Files (virtual filesystem)
//
// Content offload: when an object-storage blob backend is configured
// (knowledge/runtime-resources/object-storage.md), file *content* bytes are stored in the object
// store and the `workspace_files.content` column is left NULL; a pointer +
// content hash live in `workspace_file_blobs`. Metadata (path, size, flags,
// tree) always stays in PostgreSQL. When no blob backend is configured the
// methods fall back to the original inline `BYTEA` storage with zero overhead.

use super::super::models::*;
use super::Database;
use crate::storage::blob_store::{BlobMetadata, content_sha256, workspace_file_key};
use anyhow::Result;
use uuid::Uuid;

/// Disaster-recovery metadata stamped onto each offloaded file object
/// (knowledge/runtime-resources/object-storage.md). Lets a recovery tool rebuild the `workspace_files`
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

            let old_key = sqlx::query_as::<_, (String,)>(
                "SELECT blob_key FROM workspace_file_blobs WHERE file_id = $1",
            )
            .bind(file_id)
            .fetch_optional(&mut *tx)
            .await?
            .map(|(key,)| key);

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

            if let Some(old_key) = old_key {
                blob.delete(&old_key).await?;
            }

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
            let current_pointer: Option<(String, String)> = sqlx::query_as(
                "SELECT blob_key, content_sha256 FROM workspace_file_blobs WHERE file_id = $1",
            )
            .bind(existing.id)
            .fetch_optional(&mut *tx)
            .await?;

            let matches = match (&current_pointer, &existing.content) {
                (Some((_, sha)), _) => *sha == expected_sha,
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

            if let Some((old_key, _)) = current_pointer {
                blob.delete(&old_key).await?;
            }

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
        // Remove the backing object first (cascade clears the sidecar row).
        if let Some(blob) = self.blob_store()
            && let Some((key,)) = sqlx::query_as::<_, (String,)>(
                r#"
                SELECT b.blob_key FROM workspace_file_blobs b
                JOIN workspace_files f ON f.id = b.file_id
                WHERE f.workspace_id = $1 AND f.path = $2
                "#,
            )
            .bind(session_id)
            .bind(path)
            .fetch_optional(&self.pool)
            .await?
        {
            blob.delete(&key).await?;
        }

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

        // Remove backing objects for the whole subtree before the rows go away.
        if let Some(blob) = self.blob_store() {
            let keys = sqlx::query_as::<_, (String,)>(
                r#"
                SELECT b.blob_key FROM workspace_file_blobs b
                JOIN workspace_files f ON f.id = b.file_id
                WHERE f.workspace_id = $1 AND f.path ~ $2
                "#,
            )
            .bind(session_id)
            .bind(&pattern)
            .fetch_all(&self.pool)
            .await?;
            for (key,) in keys {
                blob.delete(&key).await?;
            }
        }

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
        // Blob keys are addressed by file id (not path), so a move only rewrites
        // path metadata — the object and its sidecar pointer are untouched.
        let mut tx = self.pool.begin().await?;

        // First, check if source exists and is a directory
        let source = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE workspace_id = $1 AND path = $2
            "#,
        )
        .bind(session_id)
        .bind(old_path)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(source) = source else {
            return Ok(None);
        };

        if source.is_directory {
            // Move all children by replacing the prefix
            let old_prefix = format!("{}/", old_path);
            let new_prefix = format!("{}/", new_path);

            sqlx::query(
                r#"
                UPDATE workspace_files
                SET path = $3 || substring(path from $4)
                WHERE workspace_id = $1 AND path LIKE $2
                "#,
            )
            .bind(session_id)
            .bind(format!("{}%", old_prefix))
            .bind(&new_prefix)
            .bind((old_prefix.len() + 1) as i32)
            .execute(&mut *tx)
            .await?;
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
            self.materialize_file_content(row).await?;
        }
        Ok(row)
    }

    /// Copy a file (directories not supported yet)
    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        src_path: &str,
        dst_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        // Offload path: read the source content and write a fresh object for the
        // destination so each file id owns its own blob (clean independent delete).
        if let Some(blob) = self.blob_store() {
            let Some(source) = self.get_session_file(session_id, src_path).await? else {
                return Ok(None);
            };
            if source.is_directory {
                return Ok(None);
            }

            let dest_id = Uuid::now_v7();
            let size_bytes = source.size_bytes;

            // Write the destination blob first (when the source has content), so
            // the row + sidecar can be inserted in one transaction with the blob
            // already durable; clean the blob up on any DB failure.
            let written_key = if let Some(content) = source.content.clone() {
                let sha = content_sha256(&content);
                let key = workspace_file_key(session_id, dest_id, &sha, Uuid::now_v7());
                blob.put(
                    &key,
                    content,
                    &file_blob_metadata(session_id, dest_id, dst_path, size_bytes, &sha),
                )
                .await?;
                Some((key, sha))
            } else {
                None
            };

            let insert = async {
                let mut tx = self.pool.begin().await?;
                let row = sqlx::query_as::<_, SessionFileRow>(
                    r#"
                    INSERT INTO workspace_files (id, workspace_id, path, content, is_directory, is_readonly, size_bytes)
                    VALUES ($1, $2, $3, NULL, FALSE, $4, $5)
                    RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
                    "#,
                )
                .bind(dest_id)
                .bind(session_id)
                .bind(dst_path)
                .bind(source.is_readonly)
                .bind(size_bytes)
                .fetch_one(&mut *tx)
                .await?;
                if let Some((key, sha)) = &written_key {
                    sqlx::query(
                        r#"
                        INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
                        VALUES ($1, $2, $3, $4)
                        "#,
                    )
                    .bind(dest_id)
                    .bind(key)
                    .bind(sha)
                    .bind(size_bytes)
                    .execute(&mut *tx)
                    .await?;
                }
                tx.commit().await?;
                anyhow::Ok(row)
            }
            .await;

            let mut row = match insert {
                Ok(row) => row,
                Err(e) => {
                    if let Some((key, _)) = &written_key {
                        let _ = blob.delete(key).await;
                    }
                    return Err(e);
                }
            };
            row.content = source.content;
            return Ok(Some(row));
        }

        // Inline path: copy the content column directly.
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            INSERT INTO workspace_files (workspace_id, path, content, is_directory, is_readonly, size_bytes)
            SELECT workspace_id, $3, content, is_directory, is_readonly, size_bytes
            FROM workspace_files
            WHERE workspace_id = $1 AND path = $2 AND is_directory = FALSE
            RETURNING id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(src_path)
        .bind(dst_path)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
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
