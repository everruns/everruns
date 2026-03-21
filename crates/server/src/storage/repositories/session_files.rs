// PostgreSQL repository: Session Files (virtual filesystem)

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // Session Files (virtual filesystem)
    // ============================================

    /// Create a new file or directory in the session virtual filesystem
    pub async fn create_session_file(&self, input: CreateSessionFileRow) -> Result<SessionFileRow> {
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64).unwrap_or(0);

        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            INSERT INTO session_files (session_id, path, content, is_directory, is_readonly, size_bytes)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
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
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1 AND path = $2
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
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE id = $1
            "#,
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

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
            SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1 AND path ~ $2
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
            SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1
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
        // Calculate new size if content is being updated
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64);

        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE session_files
            SET
                content = COALESCE($3, content),
                is_readonly = COALESCE($4, is_readonly),
                size_bytes = COALESCE($5, size_bytes)
            WHERE session_id = $1 AND path = $2 AND is_directory = FALSE
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
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
        let size_bytes = input.content.as_ref().map(|c| c.len() as i64);

        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE session_files
            SET
                content = COALESCE($4, content),
                is_readonly = COALESCE($5, is_readonly),
                size_bytes = COALESCE($6, size_bytes)
            WHERE session_id = $1
              AND path = $2
              AND is_directory = FALSE
              AND is_readonly = FALSE
              AND COALESCE(content, '\x'::bytea) = $3
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
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
        let result = sqlx::query("DELETE FROM session_files WHERE session_id = $1 AND path = $2")
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

        let result = sqlx::query("DELETE FROM session_files WHERE session_id = $1 AND path ~ $2")
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
        // For directories, we need to move all children as well
        let mut tx = self.pool.begin().await?;

        // First, check if source exists and is a directory
        let source = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1 AND path = $2
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
                UPDATE session_files
                SET path = $3 || substring(path from $4)
                WHERE session_id = $1 AND path LIKE $2
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
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            UPDATE session_files
            SET path = $3
            WHERE session_id = $1 AND path = $2
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            "#,
        )
        .bind(session_id)
        .bind(old_path)
        .bind(new_path)
        .fetch_optional(&mut *tx)
        .await?;

        tx.commit().await?;

        Ok(row)
    }

    /// Copy a file (directories not supported yet)
    pub async fn copy_session_file(
        &self,
        session_id: Uuid,
        src_path: &str,
        dst_path: &str,
    ) -> Result<Option<SessionFileRow>> {
        let row = sqlx::query_as::<_, SessionFileRow>(
            r#"
            INSERT INTO session_files (session_id, path, content, is_directory, is_readonly, size_bytes)
            SELECT session_id, $3, content, is_directory, is_readonly, size_bytes
            FROM session_files
            WHERE session_id = $1 AND path = $2 AND is_directory = FALSE
            RETURNING id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
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
    ) -> Result<Vec<SessionFileInfoRow>> {
        // Search text files for content matching the pattern
        let rows = if let Some(path_pat) = path_pattern {
            sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM session_files
                WHERE session_id = $1
                    AND is_directory = FALSE
                    AND path ~ $2
                    AND convert_from(content, 'UTF8') ~ $3
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(path_pat)
            .bind(pattern)
            .fetch_all(&self.pool)
            .await?
        } else {
            sqlx::query_as::<_, SessionFileInfoRow>(
                r#"
                SELECT id, session_id, path, is_directory, is_readonly, size_bytes, created_at, updated_at
                FROM session_files
                WHERE session_id = $1
                    AND is_directory = FALSE
                    AND convert_from(content, 'UTF8') ~ $2
                ORDER BY path ASC
                "#,
            )
            .bind(session_id)
            .bind(pattern)
            .fetch_all(&self.pool)
            .await?
        };

        Ok(rows)
    }

    /// Check if a path exists
    pub async fn session_file_exists(&self, session_id: Uuid, path: &str) -> Result<bool> {
        let result: Option<(bool,)> =
            sqlx::query_as("SELECT TRUE FROM session_files WHERE session_id = $1 AND path = $2")
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
            "SELECT TRUE FROM session_files WHERE session_id = $1 AND path ~ $2 LIMIT 1",
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
            "SELECT TRUE FROM session_files WHERE session_id = $1 AND path ~ $2 AND is_readonly = true LIMIT 1",
        )
        .bind(session_id)
        .bind(&pattern)
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.is_some())
    }

    /// Load all non-directory files with content for a session (single query).
    pub async fn load_all_session_files_with_content(
        &self,
        session_id: Uuid,
    ) -> Result<Vec<SessionFileRow>> {
        let rows = sqlx::query_as::<_, SessionFileRow>(
            r#"
            SELECT id, session_id, path, content, is_directory, is_readonly, size_bytes, created_at, updated_at
            FROM session_files
            WHERE session_id = $1 AND is_directory = false
            ORDER BY path ASC
            "#,
        )
        .bind(session_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }
}
