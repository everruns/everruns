// PostgreSQL repository: Knowledge Index + Document CRUD.
// See specs/knowledge-indexes.md.

use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

const INDEX_COLUMNS: &str = "id, org_id, public_id, name, description, source_type, source_config, \
     embedding_model_id, vector_dim, vector_namespace, owner_principal_id, resolved_owner_user_id, \
     status, sync_status, last_synced_at, last_sync_error, created_at, updated_at, archived_at, deleted_at";
const DOCUMENT_COLUMNS: &str = "id, index_id, public_id, source_uri, title, mime_type, content_hash, \
     size_bytes, chunk_count, last_seen_at, created_at, updated_at";

impl Database {
    // ------------- knowledge_indexes -------------

    pub async fn create_knowledge_index(
        &self,
        org_id: i64,
        input: CreateKnowledgeIndexRow,
    ) -> Result<KnowledgeIndexRow> {
        let sql = format!(
            "INSERT INTO knowledge_indexes \
                (org_id, public_id, name, description, source_type, source_config, \
                 embedding_model_id, vector_namespace, owner_principal_id, resolved_owner_user_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10) \
             RETURNING {INDEX_COLUMNS}"
        );
        let row = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(&input.public_id)
            .bind(&input.name)
            .bind(&input.description)
            .bind(&input.source_type)
            .bind(&input.source_config)
            .bind(input.embedding_model_id.uuid())
            .bind(&input.vector_namespace)
            .bind(&input.owner_principal_id)
            .bind(input.resolved_owner_user_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn get_knowledge_index_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let sql = format!(
            "SELECT {INDEX_COLUMNS} FROM knowledge_indexes \
             WHERE org_id = $1 AND public_id = $2 AND status != 'deleted'"
        );
        let row = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(public_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn get_knowledge_index_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let sql = format!(
            "SELECT {INDEX_COLUMNS} FROM knowledge_indexes \
             WHERE org_id = $1 AND id = $2 AND status != 'deleted'"
        );
        let row = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn list_knowledge_indexes(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeIndexRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status = 'active'"
        };
        let sql = format!(
            "SELECT {INDEX_COLUMNS} FROM knowledge_indexes \
             WHERE org_id = $1{status_sql}{search_sql} ORDER BY created_at DESC"
        );
        let mut query =
            sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pattern in &patterns {
            query = query.bind(pattern);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_knowledge_index(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeIndex,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let sql = format!(
            "UPDATE knowledge_indexes \
             SET \
                name = COALESCE($3, name), \
                description = CASE WHEN $4 THEN $5 ELSE description END, \
                source_config = COALESCE($6, source_config), \
                resolved_owner_user_id = CASE WHEN $7 THEN $8 ELSE resolved_owner_user_id END, \
                embedding_model_id = COALESCE($9, embedding_model_id), \
                status = COALESCE($10, status), \
                archived_at = CASE \
                    WHEN $10 = 'archived' THEN COALESCE(archived_at, NOW()) \
                    WHEN $10 = 'active' THEN NULL \
                    ELSE archived_at \
                END, \
                deleted_at = CASE \
                    WHEN $10 = 'deleted' THEN COALESCE(deleted_at, NOW()) \
                    ELSE deleted_at \
                END, \
                updated_at = NOW() \
             WHERE org_id = $1 AND id = $2 AND status != 'deleted' \
             RETURNING {INDEX_COLUMNS}"
        );
        let row = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(id)
            .bind(&input.name)
            .bind(input.description.is_some())
            .bind(input.description.flatten())
            .bind(&input.source_config)
            .bind(input.resolved_owner_user_id.is_changed())
            .bind(input.resolved_owner_user_id.into_value())
            .bind(input.embedding_model_id.map(|m| m.uuid()))
            .bind(&input.status)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn archive_knowledge_index(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE knowledge_indexes
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ------------- knowledge_index_documents -------------

    pub async fn list_knowledge_index_documents(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexDocumentRow>> {
        let sql = format!(
            "SELECT {DOCUMENT_COLUMNS} FROM knowledge_index_documents \
             WHERE index_id = $1 ORDER BY created_at DESC"
        );
        let rows =
            sqlx::query_as::<_, KnowledgeIndexDocumentRow>(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(index_id)
                .fetch_all(&self.pool)
                .await?;
        Ok(rows)
    }

    pub async fn list_knowledge_index_chunks(
        &self,
        index_id: Uuid,
    ) -> Result<Vec<KnowledgeIndexChunkRow>> {
        let rows = sqlx::query_as::<_, KnowledgeIndexChunkRow>(
            "SELECT id, document_id, index_id, public_id, ordinal, text, location, token_count, created_at \
             FROM knowledge_index_chunks WHERE index_id = $1 ORDER BY document_id, ordinal",
        )
        .bind(index_id)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows)
    }

    /// Hydrate chunk + owning-document citation metadata for a set of chunk
    /// `public_id`s within an index. Used by `search_index` to turn vector-store
    /// matches into citations. Missing ids are simply absent from the result.
    pub async fn get_knowledge_index_chunks_with_documents(
        &self,
        index_id: Uuid,
        chunk_public_ids: &[String],
    ) -> Result<Vec<KnowledgeIndexChunkWithDocument>> {
        if chunk_public_ids.is_empty() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query_as::<
            _,
            (
                String,
                String,
                Option<serde_json::Value>,
                i32,
                String,
                Option<String>,
            ),
        >(
            "SELECT c.public_id, c.text, c.location, c.ordinal, d.source_uri, d.title \
             FROM knowledge_index_chunks c \
             JOIN knowledge_index_documents d ON d.id = c.document_id \
             WHERE c.index_id = $1 AND c.public_id = ANY($2)",
        )
        .bind(index_id)
        .bind(chunk_public_ids)
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(chunk_public_id, text, location, ordinal, source_uri, document_title)| {
                    KnowledgeIndexChunkWithDocument {
                        chunk_public_id,
                        text,
                        location,
                        ordinal,
                        source_uri,
                        document_title,
                    }
                },
            )
            .collect())
    }

    // ------------- Syncout pipeline -------------

    /// Mark an index pending so the sync worker claims it. Idempotent: a
    /// pending/syncing index is left unchanged.
    pub async fn enqueue_knowledge_index_sync(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let sql = format!(
            "UPDATE knowledge_indexes \
             SET sync_status = 'pending', updated_at = NOW() \
             WHERE org_id = $1 AND id = $2 AND status = 'active' \
               AND sync_status NOT IN ('pending', 'syncing') \
             RETURNING {INDEX_COLUMNS}"
        );
        if let Some(row) = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?
        {
            return Ok(Some(row));
        }
        // Already pending/syncing (idempotent) or not found: return current row.
        self.get_knowledge_index_by_id(org_id, id).await
    }

    /// Claim the next index needing a sync (pending or a stale syncing claim).
    pub async fn claim_next_knowledge_index_sync(&self) -> Result<Option<KnowledgeIndexRow>> {
        let sql = format!(
            "UPDATE knowledge_indexes \
             SET sync_status = 'syncing', last_sync_error = NULL, updated_at = NOW() \
             WHERE id = ( \
                SELECT id FROM knowledge_indexes \
                WHERE status = 'active' \
                  AND ( \
                    sync_status = 'pending' \
                    OR (sync_status = 'syncing' AND updated_at < NOW() - INTERVAL '15 minutes') \
                  ) \
                ORDER BY updated_at ASC \
                FOR UPDATE SKIP LOCKED \
                LIMIT 1 \
             ) \
             RETURNING {INDEX_COLUMNS}"
        );
        let row = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    /// Atomically replace an index's documents + chunks and mark it synced.
    /// Guarded by `claimed_at`; a stale claim returns None and changes nothing.
    pub async fn complete_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: chrono::DateTime<chrono::Utc>,
        documents: Vec<CreateKnowledgeIndexDocumentWithChunks>,
        vector_dim: Option<i32>,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let mut tx = self.pool.begin().await?;

        let sql = format!(
            "UPDATE knowledge_indexes \
             SET sync_status = 'synced', \
                 last_synced_at = NOW(), \
                 last_sync_error = NULL, \
                 vector_dim = COALESCE($3, vector_dim), \
                 updated_at = NOW() \
             WHERE id = $1 AND updated_at = $2 AND sync_status = 'syncing' AND status = 'active' \
             RETURNING {INDEX_COLUMNS}"
        );
        let index = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(index_id)
            .bind(claimed_at)
            .bind(vector_dim)
            .fetch_optional(&mut *tx)
            .await?;

        if index.is_none() {
            tx.commit().await?;
            return Ok(None);
        }

        // ON DELETE CASCADE drops chunks when their document is removed.
        sqlx::query("DELETE FROM knowledge_index_documents WHERE index_id = $1")
            .bind(index_id)
            .execute(&mut *tx)
            .await?;

        for doc in documents {
            let chunk_count = doc.chunks.len() as i32;
            let doc_id: Uuid = sqlx::query_scalar(
                "INSERT INTO knowledge_index_documents \
                    (index_id, public_id, source_uri, title, mime_type, content_hash, \
                     size_bytes, chunk_count, last_seen_at) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, NOW()) RETURNING id",
            )
            .bind(index_id)
            .bind(&doc.public_id)
            .bind(&doc.source_uri)
            .bind(&doc.title)
            .bind(&doc.mime_type)
            .bind(&doc.content_hash)
            .bind(doc.size_bytes)
            .bind(chunk_count)
            .fetch_one(&mut *tx)
            .await?;

            for chunk in doc.chunks {
                sqlx::query(
                    "INSERT INTO knowledge_index_chunks \
                        (document_id, index_id, public_id, ordinal, text, location, token_count) \
                     VALUES ($1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(doc_id)
                .bind(index_id)
                .bind(&chunk.public_id)
                .bind(chunk.ordinal)
                .bind(&chunk.text)
                .bind(&chunk.location)
                .bind(chunk.token_count)
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(index)
    }

    /// Mark an index's sync as failed, preserving previously indexed content.
    /// Guarded by `claimed_at`; a stale claim returns None and changes nothing.
    pub async fn fail_knowledge_index_sync(
        &self,
        index_id: Uuid,
        claimed_at: chrono::DateTime<chrono::Utc>,
        error: &str,
    ) -> Result<Option<KnowledgeIndexRow>> {
        let sql = format!(
            "UPDATE knowledge_indexes \
             SET sync_status = 'failed', last_sync_error = $2, updated_at = NOW() \
             WHERE id = $1 AND updated_at = $3 AND sync_status = 'syncing' AND status = 'active' \
             RETURNING {INDEX_COLUMNS}"
        );
        let row = sqlx::query_as::<_, KnowledgeIndexRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(index_id)
            .bind(error)
            .bind(claimed_at)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }
}
