// PostgreSQL repository: Session Files (virtual filesystem)
//
// Content offload: when an object-storage blob backend is configured
// (specs/object-storage.md), file *content* bytes are stored in the object
// store and the `workspace_files.content` column is left NULL; a pointer +
// content hash live in `workspace_file_blobs`. Metadata (path, size, flags,
// tree) always stays in PostgreSQL. When no blob backend is configured the
// methods fall back to the original inline `BYTEA` storage with zero overhead.

use super::super::models::*;
use super::Database;
use crate::storage::blob_store::{content_sha256, workspace_file_key};
use anyhow::Result;
use uuid::Uuid;

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
        let key: Option<(String,)> =
            sqlx::query_as("SELECT blob_key FROM workspace_file_blobs WHERE file_id = $1")
                .bind(row.id)
                .fetch_optional(&self.pool)
                .await?;
        if let Some((key,)) = key {
            let bytes = blob.get(&key).await?.unwrap_or_default();
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
            let key = workspace_file_key(workspace_id, file_id);
            let sha = content_sha256(content);

            // Write the blob first; if the row insert then fails (e.g. duplicate
            // path) we clean the orphan object up rather than leaving it behind.
            blob.put(&key, content.clone()).await?;

            let mut row = match sqlx::query_as::<_, SessionFileRow>(
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
            .fetch_one(&self.pool)
            .await
            {
                Ok(row) => row,
                Err(e) => {
                    let _ = blob.delete(&key).await;
                    return Err(e.into());
                }
            };

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
            .execute(&self.pool)
            .await?;

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

    /// Get a file by ID
    pub async fn get_session_file_by_id(&self, id: Uuid) -> Result<Option<SessionFileRow>> {
        let mut row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, workspace_id AS session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM workspace_files
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(row) = row.as_mut() {
            self.materialize_file_content(row).await?;
        }
        Ok(row)
    }

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
        // Offload path: a content update with a configured blob backend. Clears
        // the inline column and writes the bytes to the object store.
        if let (Some(blob), Some(content)) = (self.blob_store(), input.content.as_ref()) {
            let size_bytes = content.len() as i64;
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
            .fetch_optional(&self.pool)
            .await?;

            if let Some(row) = row.as_mut() {
                let key = workspace_file_key(session_id, row.id);
                let sha = content_sha256(content);
                blob.put(&key, content.clone()).await?;
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
                .bind(row.id)
                .bind(&key)
                .bind(&sha)
                .bind(size_bytes)
                .execute(&self.pool)
                .await?;
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

            // Resolve current bytes: inline column if present, else the blob.
            let current = if let Some(c) = existing.content.clone() {
                c
            } else if let Some((key,)) = sqlx::query_as::<_, (String,)>(
                "SELECT blob_key FROM workspace_file_blobs WHERE file_id = $1",
            )
            .bind(existing.id)
            .fetch_optional(&mut *tx)
            .await?
            {
                blob.get(&key).await?.unwrap_or_default()
            } else {
                Vec::new()
            };

            if current != expected_content {
                tx.rollback().await?;
                return Ok(None);
            }

            let size_bytes = new_content.len() as i64;
            let key = workspace_file_key(session_id, existing.id);
            let sha = content_sha256(&new_content);
            blob.put(&key, new_content.clone()).await?;
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
            let mut row = match sqlx::query_as::<_, SessionFileRow>(
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
            .fetch_optional(&self.pool)
            .await?
            {
                Some(row) => row,
                None => return Ok(None),
            };

            if let Some(content) = source.content.clone() {
                let key = workspace_file_key(session_id, dest_id);
                let sha = content_sha256(&content);
                blob.put(&key, content.clone()).await?;
                sqlx::query(
                    r#"
                    INSERT INTO workspace_file_blobs (file_id, blob_key, content_sha256, size_bytes)
                    VALUES ($1, $2, $3, $4)
                    "#,
                )
                .bind(dest_id)
                .bind(&key)
                .bind(&sha)
                .bind(size_bytes)
                .execute(&self.pool)
                .await?;
                row.content = Some(content);
            }
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
        max_file_bytes: i64,
    ) -> Result<Vec<SessionFileInfoRow>> {
        // Offload path: content is in the object store, so PostgreSQL cannot grep
        // it. Return size/path-filtered candidates; the service-layer scan fetches
        // each blob and matches lines (TM-DOS-008 per-file and total-scan caps
        // still bound the work).
        if self.blob_store().is_some() {
            let rows = if let Some(path_pat) = path_pattern {
                sqlx::query_as::<_, SessionFileInfoRow>(
                    r#"
                    SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                    FROM workspace_files
                    WHERE workspace_id = $1
                        AND is_directory = FALSE
                        AND size_bytes <= $2
                        AND path ~ $3
                    ORDER BY path ASC
                    "#,
                )
                .bind(session_id)
                .bind(max_file_bytes)
                .bind(path_pat)
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
                    ORDER BY path ASC
                    "#,
                )
                .bind(session_id)
                .bind(max_file_bytes)
                .fetch_all(&self.pool)
                .await?
            };
            return Ok(rows);
        }

        // Inline path: TM-DOS-008 size_bytes filter keeps Postgres from scanning
        // large files; content match happens in-database.
        let rows = if let Some(path_pat) = path_pattern {
            sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, workspace_id AS session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM workspace_files
                WHERE workspace_id = $1
                    AND is_directory = FALSE
                    AND size_bytes <= $2
                    AND path ~ $3
                    AND convert_from(content, 'UTF8') ~ $4
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(max_file_bytes)
            .bind(path_pat)
            .bind(pattern)
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
                    AND convert_from(content, 'UTF8') ~ $3
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(max_file_bytes)
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

    /// Load all non-directory files with content for a session (single query).
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
