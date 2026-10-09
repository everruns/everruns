// PostgreSQL repository: Virtual User CRUD

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_contracts::typed_id::VirtualUserId;

impl Database {
    // ============================================
    // Virtual User CRUD
    // ============================================

    pub async fn create_virtual_user(&self, input: CreateVirtualUserRow) -> Result<VirtualUserRow> {
        let row = sqlx::query_as::<_, VirtualUserRow>(
            r#"
            INSERT INTO virtual_users (org_id, id, name, description, avatar_url, locale, timezone, status, usage)
            VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', $8)
            ON CONFLICT DO NOTHING
            RETURNING id, org_id, usage, name, description, avatar_url, locale, timezone, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(input.org_id)
        .bind(input.id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.avatar_url)
        .bind(&input.locale)
        .bind(&input.timezone)
        .bind(&input.usage)
        .fetch_optional(&self.pool)
        .await?;

        // Deterministic ids make concurrent creates of the same user normal
        // (runtime identity first use). A targeted `ON CONFLICT (id)` only
        // arbitrates the primary key, so a racing insert could still trip the
        // `(org_id, id)` unique index; `DO NOTHING` covers every unique index
        // and the existing row is read back instead.
        match row {
            Some(row) => Ok(row),
            None => self
                .get_virtual_user(input.org_id, input.id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("virtual user id already exists in another org")),
        }
    }

    /// Look up the owning org for a virtual user by its public id.
    pub async fn get_virtual_user_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let Ok(id) = public_id.parse::<VirtualUserId>() else {
            return Ok(None);
        };
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM virtual_users WHERE id = $1 LIMIT 1")
                .bind(id)
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(org_id,)| org_id))
    }

    pub async fn get_virtual_user(
        &self,
        org_id: i64,
        id: VirtualUserId,
    ) -> Result<Option<VirtualUserRow>> {
        let row = sqlx::query_as::<_, VirtualUserRow>(
            r#"
            SELECT id, org_id, usage, name, description, avatar_url, locale, timezone, status, created_at, updated_at, archived_at, deleted_at
            FROM virtual_users
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_virtual_users(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
        usage: Option<&str>,
        pagination: crate::common_dto::Pagination,
    ) -> Result<(Vec<VirtualUserRow>, u32)> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 3);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let predicate = format!(
            "org_id=$1 AND usage <> 'organization' AND ($2::text IS NULL OR usage=$2){status_sql}{search_sql}"
        );
        let count_sql = format!("SELECT count(*) FROM virtual_users WHERE {predicate}");
        let mut count = sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(count_sql.as_str()))
            .bind(org_id)
            .bind(usage);
        for pattern in &patterns {
            count = count.bind(pattern);
        }
        let total = count.fetch_one(&self.pool).await? as u32;
        let limit_index = 3 + patterns.len();
        let sql = format!(
            "SELECT * FROM virtual_users WHERE {predicate} ORDER BY created_at DESC,id DESC LIMIT ${limit_index} OFFSET ${}",
            limit_index + 1
        );
        let mut query = sqlx::query_as::<_, VirtualUserRow>(sqlx::AssertSqlSafe(sql.as_str()))
            .bind(org_id)
            .bind(usage);
        for pattern in &patterns {
            query = query.bind(pattern);
        }
        let rows = query
            .bind(i64::from(pagination.limit))
            .bind(i64::from(pagination.offset))
            .fetch_all(&self.pool)
            .await?;
        Ok((rows, total))
    }

    pub async fn update_virtual_user(
        &self,
        org_id: i64,
        id: VirtualUserId,
        input: UpdateVirtualUser,
    ) -> Result<Option<VirtualUserRow>> {
        let row = sqlx::query_as::<_, VirtualUserRow>(
            r#"
            UPDATE virtual_users
            SET
                name = COALESCE($3, name),
                description = CASE WHEN $4 THEN $5 ELSE description END,
                avatar_url = CASE WHEN $6 THEN $7 ELSE avatar_url END,
                locale = CASE WHEN $8 THEN $9 ELSE locale END,
                timezone = CASE WHEN $10 THEN $11 ELSE timezone END,
                status = COALESCE($12, status),
                updated_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, usage, name, description, avatar_url, locale, timezone, status, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(input.description.is_changed())
        .bind(input.description.into_value())
        .bind(input.avatar_url.is_changed())
        .bind(input.avatar_url.into_value())
        .bind(input.locale.is_changed())
        .bind(input.locale.into_value())
        .bind(input.timezone.is_changed())
        .bind(input.timezone.into_value())
        .bind(&input.status)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_virtual_user(&self, org_id: i64, id: VirtualUserId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE virtual_users
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

    pub async fn destroy_virtual_user(&self, org_id: i64, id: VirtualUserId) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE virtual_users
            SET status = 'deleted', deleted_at = COALESCE(deleted_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status = 'archived'
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
