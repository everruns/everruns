// PostgreSQL repository: Knowledge Base + Entry CRUD

use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

const KB_COLUMNS: &str = "id, org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at";
const ENTRY_COLUMNS: &str = "id, kb_id, public_id, title, body, kind, tags, created_at, updated_at";

impl Database {
    // ------------- knowledge_bases -------------

    pub async fn create_knowledge_base(
        &self,
        org_id: i64,
        input: CreateKnowledgeBaseRow,
    ) -> Result<KnowledgeBaseRow> {
        let row = sqlx::query_as::<_, KnowledgeBaseRow>(
            r#"
            INSERT INTO knowledge_bases
                (org_id, public_id, name, description, owner_principal_id, resolved_owner_user_id)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, org_id, public_id, name, description, owner_principal_id,
                      resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.owner_principal_id)
        .bind(input.resolved_owner_user_id)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_knowledge_base_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<KnowledgeBaseRow>> {
        let sql = format!(
            "SELECT {KB_COLUMNS} FROM knowledge_bases \
             WHERE org_id = $1 AND public_id = $2 AND status != 'deleted'"
        );
        let row = sqlx::query_as::<_, KnowledgeBaseRow>(&sql)
            .bind(org_id)
            .bind(public_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn get_knowledge_base_by_id(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<KnowledgeBaseRow>> {
        let sql = format!(
            "SELECT {KB_COLUMNS} FROM knowledge_bases \
             WHERE org_id = $1 AND id = $2 AND status != 'deleted'"
        );
        let row = sqlx::query_as::<_, KnowledgeBaseRow>(&sql)
            .bind(org_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn list_knowledge_bases(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<KnowledgeBaseRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status = 'active'"
        };
        let sql = format!(
            "SELECT {KB_COLUMNS} FROM knowledge_bases \
             WHERE org_id = $1{status_sql}{search_sql} ORDER BY created_at DESC"
        );
        let mut query = sqlx::query_as::<_, KnowledgeBaseRow>(&sql).bind(org_id);
        for pattern in &patterns {
            query = query.bind(pattern);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn update_knowledge_base(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateKnowledgeBase,
    ) -> Result<Option<KnowledgeBaseRow>> {
        let row = sqlx::query_as::<_, KnowledgeBaseRow>(
            r#"
            UPDATE knowledge_bases
            SET
                name = COALESCE($3, name),
                description = CASE WHEN $4 THEN $5 ELSE description END,
                status = COALESCE($6, status),
                archived_at = CASE
                    WHEN $6 = 'archived' THEN COALESCE(archived_at, NOW())
                    WHEN $6 = 'active' THEN NULL
                    ELSE archived_at
                END,
                deleted_at = CASE
                    WHEN $6 = 'deleted' THEN COALESCE(deleted_at, NOW())
                    ELSE deleted_at
                END,
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status != 'deleted'
            RETURNING id, org_id, public_id, name, description, owner_principal_id,
                      resolved_owner_user_id, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(input.description.is_some())
        .bind(input.description.flatten())
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn archive_knowledge_base(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE knowledge_bases
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

    // ------------- knowledge_entries -------------

    pub async fn create_knowledge_entry(
        &self,
        kb_id: Uuid,
        input: CreateKnowledgeEntryRow,
    ) -> Result<KnowledgeEntryRow> {
        let row = sqlx::query_as::<_, KnowledgeEntryRow>(
            r#"
            INSERT INTO knowledge_entries (kb_id, public_id, title, body, kind, tags)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id, kb_id, public_id, title, body, kind, tags, created_at, updated_at
            "#,
        )
        .bind(kb_id)
        .bind(&input.public_id)
        .bind(&input.title)
        .bind(&input.body)
        .bind(&input.kind)
        .bind(&input.tags)
        .fetch_one(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn get_knowledge_entry_by_public_id(
        &self,
        kb_id: Uuid,
        public_id: &str,
    ) -> Result<Option<KnowledgeEntryRow>> {
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM knowledge_entries \
             WHERE kb_id = $1 AND public_id = $2"
        );
        let row = sqlx::query_as::<_, KnowledgeEntryRow>(&sql)
            .bind(kb_id)
            .bind(public_id)
            .fetch_optional(&self.pool)
            .await?;
        Ok(row)
    }

    pub async fn list_knowledge_entries(
        &self,
        kb_id: Uuid,
        search: Option<&str>,
        kind: Option<&str>,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        let (search_sql, patterns) = build_search_sql(search, "LOWER(title || ' ' || body)", 3);
        let kind_sql = if kind.is_some() { " AND kind = $2" } else { "" };
        let sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM knowledge_entries \
             WHERE kb_id = $1{kind_sql}{search_sql} ORDER BY created_at DESC"
        );
        let mut query = sqlx::query_as::<_, KnowledgeEntryRow>(&sql).bind(kb_id);
        if let Some(k) = kind {
            query = query.bind(k);
        }
        for pattern in &patterns {
            query = query.bind(pattern);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    pub async fn search_knowledge_entries(
        &self,
        kb_ids: &[Uuid],
        query: &str,
        kind: Option<&str>,
        tags: &[String],
        limit: usize,
    ) -> Result<Vec<KnowledgeEntryRow>> {
        if kb_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut sql = format!(
            "SELECT {ENTRY_COLUMNS} FROM knowledge_entries WHERE kb_id = ANY($1) AND \
             to_tsvector('english', coalesce(title,'') || ' ' || coalesce(body,'')) @@ plainto_tsquery('english', $2)"
        );
        if kind.is_some() {
            sql.push_str(" AND kind = $3");
        }
        if !tags.is_empty() {
            // tags are already lowercased by the caller; KB-stored tags are
            // lowercased in the domain layer too, so direct equality is safe.
            let param_idx = if kind.is_some() { 4 } else { 3 };
            sql.push_str(&format!(" AND tags @> ${param_idx}"));
        }
        sql.push_str(" ORDER BY ts_rank(to_tsvector('english', title || ' ' || body), plainto_tsquery('english', $2)) DESC");
        sql.push_str(&format!(" LIMIT {}", limit.clamp(1, 100)));

        let mut q = sqlx::query_as::<_, KnowledgeEntryRow>(&sql)
            .bind(kb_ids)
            .bind(query);
        if let Some(k) = kind {
            q = q.bind(k);
        }
        if !tags.is_empty() {
            q = q.bind(tags);
        }
        Ok(q.fetch_all(&self.pool).await?)
    }

    pub async fn update_knowledge_entry(
        &self,
        kb_id: Uuid,
        id: Uuid,
        input: UpdateKnowledgeEntry,
    ) -> Result<Option<KnowledgeEntryRow>> {
        let row = sqlx::query_as::<_, KnowledgeEntryRow>(
            r#"
            UPDATE knowledge_entries
            SET
                title = COALESCE($3, title),
                body = COALESCE($4, body),
                kind = COALESCE($5, kind),
                tags = COALESCE($6, tags),
                updated_at = NOW()
            WHERE kb_id = $1 AND id = $2
            RETURNING id, kb_id, public_id, title, body, kind, tags, created_at, updated_at
            "#,
        )
        .bind(kb_id)
        .bind(id)
        .bind(&input.title)
        .bind(&input.body)
        .bind(&input.kind)
        .bind(&input.tags)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row)
    }

    pub async fn delete_knowledge_entry(&self, kb_id: Uuid, id: Uuid) -> Result<bool> {
        let result = sqlx::query("DELETE FROM knowledge_entries WHERE kb_id = $1 AND id = $2")
            .bind(kb_id)
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }
}
