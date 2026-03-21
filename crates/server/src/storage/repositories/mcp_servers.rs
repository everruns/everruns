// PostgreSQL repository: MCP Servers

use super::super::models::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use uuid::Uuid;

impl Database {
    // ============================================
    // MCP Servers
    // ============================================

    pub async fn create_mcp_server(
        &self,
        org_id: i64,
        input: CreateMcpServerRow,
    ) -> Result<McpServerRow> {
        let headers = input.headers.unwrap_or(serde_json::json!({}));
        let settings = input.settings.unwrap_or(serde_json::json!({}));
        let api_key_set = input.api_key_encrypted.is_some();

        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            INSERT INTO mcp_servers (org_id, name, description, url, transport_type, api_key_encrypted, api_key_set, headers, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.url)
        .bind(&input.transport_type)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&headers)
        .bind(&settings)
        .fetch_one(&self.pool)
        .await?;

        Ok(row)
    }

    /// Create MCP server with a specific ID (for seeding)
    /// Returns None if server already exists with this ID
    /// Create or update MCP server with a specific ID (for seeding).
    /// Returns Some(row) if created or updated, None if unchanged.
    pub async fn create_mcp_server_with_id(
        &self,
        org_id: i64,
        id: Uuid,
        input: CreateMcpServerRow,
    ) -> Result<Option<McpServerRow>> {
        let headers = input.headers.unwrap_or(serde_json::json!({}));
        let settings = input.settings.unwrap_or(serde_json::json!({}));
        let api_key_set = input.api_key_encrypted.is_some();

        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            INSERT INTO mcp_servers (id, org_id, name, description, url, transport_type, api_key_encrypted, api_key_set, headers, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
            ON CONFLICT (id) DO UPDATE SET
                name = EXCLUDED.name,
                description = EXCLUDED.description,
                url = EXCLUDED.url,
                transport_type = EXCLUDED.transport_type,
                updated_at = NOW()
            WHERE
                mcp_servers.name IS DISTINCT FROM EXCLUDED.name
                OR mcp_servers.description IS DISTINCT FROM EXCLUDED.description
                OR mcp_servers.url IS DISTINCT FROM EXCLUDED.url
                OR mcp_servers.transport_type IS DISTINCT FROM EXCLUDED.transport_type
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(id)
        .bind(org_id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.url)
        .bind(&input.transport_type)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&headers)
        .bind(&settings)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn get_mcp_server(&self, org_id: i64, id: Uuid) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            FROM mcp_servers
            WHERE org_id = $1 AND id = $2
            "#,
        )
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Batch fetch multiple MCP servers by IDs in a single query.
    pub async fn get_mcp_servers_batch(
        &self,
        org_id: i64,
        ids: &[Uuid],
    ) -> Result<Vec<McpServerRow>> {
        if ids.is_empty() {
            return Ok(vec![]);
        }
        let rows = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            FROM mcp_servers
            WHERE org_id = $1 AND id = ANY($2)
            "#,
        )
        .bind(org_id)
        .bind(ids)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn get_mcp_server_by_name(
        &self,
        org_id: i64,
        name: &str,
    ) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            FROM mcp_servers
            WHERE org_id = $1 AND name = $2
            "#,
        )
        .bind(org_id)
        .bind(name)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn list_mcp_servers(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<McpServerRow>> {
        let (search_sql, patterns) =
            build_search_sql(search, "LOWER(name || ' ' || COALESCE(description, ''))", 2);
        let status_sql = if include_archived {
            " AND status != 'deleted'"
        } else {
            " AND status NOT IN ('archived', 'deleted')"
        };
        let sql = format!(
            r#"SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
                FROM mcp_servers
                WHERE org_id = $1{status_sql}{search_sql}
                ORDER BY created_at DESC"#
        );
        let mut query = sqlx::query_as::<_, McpServerRow>(&sql).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }

    /// List only active MCP servers (for capability listing)
    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        let rows = sqlx::query_as::<_, McpServerRow>(
            r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            FROM mcp_servers
            WHERE org_id = $1 AND status = 'active'
            ORDER BY name ASC
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    pub async fn update_mcp_server(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        // Handle api_key_set: if we're updating the encrypted key, also update the flag
        let api_key_set = input.api_key_encrypted.as_ref().map(|_| true);

        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            UPDATE mcp_servers
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                url = COALESCE($5, url),
                transport_type = COALESCE($6, transport_type),
                status = COALESCE($7, status),
                api_key_encrypted = COALESCE($8, api_key_encrypted),
                api_key_set = COALESCE($9, api_key_set),
                headers = COALESCE($10, headers),
                settings = COALESCE($11, settings)
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.url)
        .bind(&input.transport_type)
        .bind(&input.status)
        .bind(&input.api_key_encrypted)
        .bind(api_key_set)
        .bind(&input.headers)
        .bind(&input.settings)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Update cached tools for an MCP server
    pub async fn update_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(
            r#"
            UPDATE mcp_servers
            SET
                cached_tools = $3,
                tools_cached_at = NOW()
            WHERE org_id = $1 AND id = $2
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at
            "#,
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.cached_tools)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn delete_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE mcp_servers
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status IN ('active', 'disabled')
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }

    pub async fn destroy_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE mcp_servers
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
