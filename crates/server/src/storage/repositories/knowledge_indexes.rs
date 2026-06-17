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
                embedding_model_id = COALESCE($7, embedding_model_id), \
                status = COALESCE($8, status), \
                archived_at = CASE \
                    WHEN $8 = 'archived' THEN COALESCE(archived_at, NOW()) \
                    WHEN $8 = 'active' THEN NULL \
                    ELSE archived_at \
                END, \
                deleted_at = CASE \
                    WHEN $8 = 'deleted' THEN COALESCE(deleted_at, NOW()) \
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
}
