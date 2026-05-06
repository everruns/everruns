// PostgreSQL repository: org-scoped memory stores and memories.
// See specs/memory.md.

use super::super::models::*;
use super::{Database, build_search_sql};
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // Memory Stores
    // ============================================

    pub async fn create_memory_store(
        &self,
        org_id: i64,
        input: CreateMemoryStoreRow,
    ) -> Result<MemoryStoreDbRow> {
        let row = sqlx::query_as::<_, MemoryStoreDbRow>(
            r#"
            INSERT INTO memory_stores (org_id, public_id, name, is_default)
            VALUES ($1, $2, $3, $4)
            RETURNING id, org_id, public_id, name, is_default, created_at, updated_at
            "#,
        )
        .bind(org_id)
        .bind(&input.public_id)
        .bind(&input.name)
        .bind(input.is_default)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_memory_store_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<MemoryStoreDbRow>> {
        let row = sqlx::query_as::<_, MemoryStoreDbRow>(
            r#"
            SELECT id, org_id, public_id, name, is_default, created_at, updated_at
            FROM memory_stores
            WHERE org_id = $1 AND public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_default_memory_store(&self, org_id: i64) -> Result<Option<MemoryStoreDbRow>> {
        let row = sqlx::query_as::<_, MemoryStoreDbRow>(
            r#"
            SELECT id, org_id, public_id, name, is_default, created_at, updated_at
            FROM memory_stores
            WHERE org_id = $1 AND is_default
            LIMIT 1
            "#,
        )
        .bind(org_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_memory_stores(&self, org_id: i64) -> Result<Vec<MemoryStoreDbRow>> {
        let rows = sqlx::query_as::<_, MemoryStoreDbRow>(
            r#"
            SELECT id, org_id, public_id, name, is_default, created_at, updated_at
            FROM memory_stores
            WHERE org_id = $1
            ORDER BY is_default DESC, created_at ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// List memory stores joined with their active-memory counts. Single
    /// roundtrip — preferred over list + per-store count_active_memories.
    pub async fn list_memory_stores_with_counts(
        &self,
        org_id: i64,
    ) -> Result<Vec<MemoryStoreWithCount>> {
        let rows = sqlx::query_as::<_, MemoryStoreWithCount>(
            r#"
            SELECT s.id, s.org_id, s.public_id, s.name, s.is_default,
                   s.created_at, s.updated_at,
                   COALESCE(c.active_count, 0) AS active_count
            FROM memory_stores s
            LEFT JOIN (
                SELECT store_id, COUNT(*) AS active_count
                FROM memories
                WHERE active = TRUE
                GROUP BY store_id
            ) c ON c.store_id = s.id
            WHERE s.org_id = $1
            ORDER BY s.is_default DESC, s.created_at ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn count_memory_stores(&self, org_id: i64) -> Result<i64> {
        let row: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM memory_stores WHERE org_id = $1")
            .bind(org_id)
            .fetch_one(&self.pool)
            .await?;
        Ok(row.0)
    }

    // ============================================
    // Memories
    // ============================================

    /// Insert a memory with an atomic per-store capacity check.
    ///
    /// `SELECT … FOR UPDATE` on the store row serialises concurrent creates
    /// targeting the same store so the count and the insert observe a
    /// consistent snapshot; without this two callers can both pass a 9999 →
    /// 10000 transition and exceed `MAX_MEMORIES_PER_STORE`.
    ///
    /// Returns `Ok(None)` when the store is at or above `cap_limit`; the
    /// caller maps that to a tool-error message that is safe to surface to
    /// agents and other org members.
    pub async fn create_memory_with_cap(
        &self,
        input: CreateMemoryRow,
        cap_limit: i64,
    ) -> Result<Option<MemoryDbRow>> {
        let mut tx = self.pool.begin().await?;

        // Serialise creates for this store. The lock is released when `tx`
        // commits or rolls back, and the row's payload doesn't matter here.
        let _store_lock: (uuid::Uuid,) =
            sqlx::query_as("SELECT id FROM memory_stores WHERE id = $1 AND org_id = $2 FOR UPDATE")
                .bind(input.store_id)
                .bind(input.org_id)
                .fetch_one(&mut *tx)
                .await?;

        let count: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM memories WHERE store_id = $1 AND active = TRUE")
                .bind(input.store_id)
                .fetch_one(&mut *tx)
                .await?;

        if count.0 >= cap_limit {
            tx.rollback().await?;
            return Ok(None);
        }

        let row = sqlx::query_as::<_, MemoryDbRow>(
            r#"
            WITH inserted AS (
                INSERT INTO memories (
                    public_id, store_id, org_id, content, content_parts,
                    kind, importance, tags, active
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE)
                RETURNING *
            )
            SELECT i.id, i.public_id, i.store_id, s.public_id AS store_public_id,
                   i.org_id, i.content, i.content_parts, i.kind, i.importance,
                   i.tags, i.active, i.created_at, i.updated_at
            FROM inserted i
            JOIN memory_stores s ON s.id = i.store_id
            "#,
        )
        .bind(&input.public_id)
        .bind(input.store_id)
        .bind(input.org_id)
        .bind(&input.content)
        .bind(&input.content_parts)
        .bind(&input.kind)
        .bind(input.importance)
        .bind(&input.tags)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some(row))
    }

    pub async fn create_memory(&self, input: CreateMemoryRow) -> Result<MemoryDbRow> {
        let row = sqlx::query_as::<_, MemoryDbRow>(
            r#"
            WITH inserted AS (
                INSERT INTO memories (
                    public_id, store_id, org_id, content, content_parts,
                    kind, importance, tags, active
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, TRUE)
                RETURNING *
            )
            SELECT i.id, i.public_id, i.store_id, s.public_id AS store_public_id,
                   i.org_id, i.content, i.content_parts, i.kind, i.importance,
                   i.tags, i.active, i.created_at, i.updated_at
            FROM inserted i
            JOIN memory_stores s ON s.id = i.store_id
            "#,
        )
        .bind(&input.public_id)
        .bind(input.store_id)
        .bind(input.org_id)
        .bind(&input.content)
        .bind(&input.content_parts)
        .bind(&input.kind)
        .bind(input.importance)
        .bind(&input.tags)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_memory_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<MemoryDbRow>> {
        let row = sqlx::query_as::<_, MemoryDbRow>(
            r#"
            SELECT m.id, m.public_id, m.store_id, s.public_id AS store_public_id,
                   m.org_id, m.content, m.content_parts, m.kind, m.importance,
                   m.tags, m.active, m.created_at, m.updated_at
            FROM memories m
            JOIN memory_stores s ON s.id = m.store_id
            WHERE m.org_id = $1 AND m.public_id = $2
            "#,
        )
        .bind(org_id)
        .bind(public_id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_memories(
        &self,
        org_id: i64,
        store_id: Option<Uuid>,
        filter: ListMemoriesFilter,
    ) -> Result<(Vec<MemoryDbRow>, i64)> {
        let limit = if filter.limit == 0 {
            10
        } else {
            filter.limit.min(200)
        } as i64;

        let mut sql = String::from(
            r#"
            SELECT m.id, m.public_id, m.store_id, s.public_id AS store_public_id,
                   m.org_id, m.content, m.content_parts, m.kind, m.importance,
                   m.tags, m.active, m.created_at, m.updated_at,
                   COUNT(*) OVER() AS total_count
            FROM memories m
            JOIN memory_stores s ON s.id = m.store_id
            WHERE m.org_id = $1
            "#,
        );

        let mut idx = 2;
        if store_id.is_some() {
            sql.push_str(&format!(" AND m.store_id = ${idx}"));
            idx += 1;
        }
        if !filter.include_inactive {
            sql.push_str(" AND m.active = TRUE");
        }
        if filter.kind.is_some() {
            sql.push_str(&format!(" AND m.kind = ${idx}"));
            idx += 1;
        }
        if filter.tags.as_ref().is_some_and(|t| !t.is_empty()) {
            sql.push_str(&format!(" AND m.tags @> ${idx}"));
            idx += 1;
        }
        // Multi-token AND search using the shared escape/cap helper. Each
        // whitespace-separated token must match `LOWER(content)` with `%`/`_`
        // wildcards escaped (TM-API-014). Tokens beyond MAX_SEARCH_TOKENS are
        // ignored to bound query cost.
        let (search_sql, search_patterns) =
            build_search_sql(filter.query.as_deref(), "LOWER(m.content)", idx);
        sql.push_str(&search_sql);
        idx += search_patterns.len();
        sql.push_str(&format!(
            " ORDER BY m.importance DESC, m.created_at DESC LIMIT ${idx}"
        ));

        let mut q = sqlx::query_as::<_, MemoryWithTotalRow>(&sql).bind(org_id);
        if let Some(sid) = store_id {
            q = q.bind(sid);
        }
        if let Some(ref kind) = filter.kind {
            q = q.bind(kind);
        }
        if let Some(ref tags) = filter.tags
            && !tags.is_empty()
        {
            q = q.bind(tags);
        }
        for pattern in &search_patterns {
            q = q.bind(pattern);
        }
        q = q.bind(limit);

        let rows = q.fetch_all(&self.pool).await?;

        let total = rows.first().map(|r| r.total_count).unwrap_or(0);
        let memories = rows
            .into_iter()
            .map(|r| MemoryDbRow {
                id: r.id,
                public_id: r.public_id,
                store_id: r.store_id,
                store_public_id: r.store_public_id,
                org_id: r.org_id,
                content: r.content,
                content_parts: r.content_parts,
                kind: r.kind,
                importance: r.importance,
                tags: r.tags,
                active: r.active,
                created_at: r.created_at,
                updated_at: r.updated_at,
            })
            .collect();
        Ok((memories, total))
    }

    pub async fn count_active_memories(&self, store_id: Uuid) -> Result<i64> {
        let row: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM memories WHERE store_id = $1 AND active = TRUE")
                .bind(store_id)
                .fetch_one(&self.pool)
                .await?;
        Ok(row.0)
    }

    pub async fn deactivate_memory(
        &self,
        org_id: i64,
        store_id: Uuid,
        memory_public_id: &str,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE memories
            SET active = FALSE, updated_at = NOW()
            WHERE org_id = $1 AND store_id = $2 AND public_id = $3 AND active = TRUE
            "#,
        )
        .bind(org_id)
        .bind(store_id)
        .bind(memory_public_id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}

#[derive(sqlx::FromRow)]
struct MemoryWithTotalRow {
    id: Uuid,
    public_id: String,
    store_id: Uuid,
    store_public_id: String,
    org_id: i64,
    content: String,
    content_parts: serde_json::Value,
    kind: String,
    importance: i16,
    tags: Vec<String>,
    active: bool,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    total_count: i64,
}
