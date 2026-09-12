//! File attachment storage (model-input files, e.g. PDFs).
//!
//! Mirrors the images repository minus thumbnails and object-store offload:
//! files are size-capped at upload and stored inline as BYTEA.

use super::super::models::*;
use super::Database;
use crate::kernel_imports::everruns_provider::typed_id::FileId;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    pub async fn create_file(&self, org_id: i64, input: CreateFileRow) -> Result<FileRow> {
        let id = FileId::new();
        let row = sqlx::query_as::<_, FileRow>(
            r#"INSERT INTO files (id, org_id, filename, content_type, size_bytes, data, metadata)
               VALUES ($1, $2, $3, $4, $5, $6, $7)
               RETURNING *"#,
        )
        .bind(id.uuid())
        .bind(org_id)
        .bind(&input.filename)
        .bind(&input.content_type)
        .bind(input.size_bytes)
        .bind(&input.data)
        .bind(&input.metadata)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_file(&self, org_id: i64, id: Uuid) -> Result<Option<FileRow>> {
        let row =
            sqlx::query_as::<_, FileRow>(r#"SELECT * FROM files WHERE id = $1 AND org_id = $2"#)
                .bind(id)
                .bind(org_id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row)
    }

    pub async fn get_file_info(&self, org_id: i64, id: Uuid) -> Result<Option<FileInfoRow>> {
        let row = sqlx::query_as::<_, FileInfoRow>(
            r#"SELECT id, org_id, filename, content_type, size_bytes, metadata, created_at
               FROM files WHERE id = $1 AND org_id = $2"#,
        )
        .bind(id)
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_file(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let res = sqlx::query(r#"DELETE FROM files WHERE id = $1 AND org_id = $2"#)
            .bind(id)
            .bind(org_id)
            .execute(&self.pool)
            .await?;
        Ok(res.rows_affected() > 0)
    }

    pub async fn list_files(&self, org_id: i64, limit: i64) -> Result<Vec<FileInfoRow>> {
        let rows = sqlx::query_as::<_, FileInfoRow>(
            r#"SELECT id, org_id, filename, content_type, size_bytes, metadata, created_at
               FROM files WHERE org_id = $1 ORDER BY created_at DESC LIMIT $2"#,
        )
        .bind(org_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }
}
