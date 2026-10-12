// PostgreSQL repository: MCP Servers
//
// Spec: knowledge/integrations/mcp.md (umbrella), knowledge/integrations/mcp-servers.md (detail)

pub(super) mod rows;
use rows::*;

use super::super::mcp_catalog::{McpServerAgentNamesRow, McpServerAgentUsageRow};
use super::super::mcp_tool_cache::*;
use super::Database;
use super::build_search_sql;
use anyhow::Result;
use everruns_contracts::typed_id::McpServerId;
use everruns_server_macros::sql;
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
            sql!(r#"
            INSERT INTO mcp_servers (org_id, name, description, url, transport_type, api_key_encrypted, api_key_set, headers, settings)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
            RETURNING {McpServerRow}
            "#),
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
            sql!(r#"
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
            RETURNING {McpServerRow}
            "#),
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

    /// Look up the owning org for an MCP server by its public id.
    pub async fn get_mcp_server_organization_id(&self, public_id: &str) -> Result<Option<i64>> {
        let Ok(id) = public_id.parse::<McpServerId>() else {
            return Ok(None);
        };
        let row: Option<(i64,)> =
            sqlx::query_as("SELECT org_id FROM mcp_servers WHERE id = $1 AND owner_virtual_user_id IS NULL LIMIT 1")
                .bind(id.uuid())
                .fetch_optional(&self.pool)
                .await?;
        Ok(row.map(|(org_id,)| org_id))
    }

    pub async fn get_mcp_server(&self, org_id: i64, id: Uuid) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(sql!(
            r#"
            SELECT {McpServerRow}
            FROM mcp_servers
            WHERE org_id = $1 AND id = $2 AND owner_virtual_user_id IS NULL
            "#
        ))
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
        let rows = sqlx::query_as::<_, McpServerRow>(sql!(
            r#"
            SELECT {McpServerRow}
            FROM mcp_servers
            WHERE org_id = $1 AND id = ANY($2) AND owner_virtual_user_id IS NULL
            "#
        ))
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
        let row = sqlx::query_as::<_, McpServerRow>(sql!(
            r#"
            SELECT {McpServerRow}
            FROM mcp_servers
            -- Live rows only. Archived and deleted rows release their name
            -- (EVE-964), so a name can now match a dead row and a live one;
            -- callers asking "which server is called X" mean the live one.
            WHERE org_id = $1 AND name = $2 AND status IN ('active', 'disabled')
              AND owner_virtual_user_id IS NULL
            "#
        ))
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
            r#"SELECT {columns}
                FROM mcp_servers
                WHERE org_id = $1 AND owner_virtual_user_id IS NULL{status_sql}{search_sql}
                ORDER BY created_at DESC"#,
            columns = McpServerRow::COLUMNS
        );
        let mut query =
            sqlx::query_as::<_, McpServerRow>(sqlx::AssertSqlSafe(sql.as_str())).bind(org_id);
        for pat in &patterns {
            query = query.bind(pat);
        }
        Ok(query.fetch_all(&self.pool).await?)
    }
    pub async fn list_mcp_server_catalog_page(
        &self,
        org_id: i64,
        cursor: Option<McpServerId>,
        limit: i64,
    ) -> Result<Vec<McpServerRow>> {
        Ok(sqlx::query_as::<_, McpServerRow>(sql!(
            r#"
            SELECT {McpServerRow}
            FROM mcp_servers
            WHERE org_id = $1
              AND owner_virtual_user_id IS NULL
              AND status != 'deleted'
              AND ($2::uuid IS NULL OR id < $2)
            ORDER BY id DESC
            LIMIT $3
            "#
        ))
        .bind(org_id)
        .bind(cursor.map(|id| id.uuid()))
        .bind(limit)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn list_mcp_server_agent_usage(
        &self,
        org_id: i64,
    ) -> Result<Vec<McpServerAgentUsageRow>> {
        Ok(sqlx::query_as::<_, McpServerAgentUsageRow>(
            r#"
            SELECT ms.id AS mcp_server_id,
                   COUNT(DISTINCT a.id) FILTER (WHERE a.id IS NOT NULL) AS used_by_agents
            FROM mcp_servers ms
            LEFT JOIN agents a
              ON a.org_id = ms.org_id
             AND a.status = 'active'
             AND a.archived_at IS NULL
             AND a.deleted_at IS NULL
             AND EXISTS (
                 SELECT 1
                 FROM jsonb_each(COALESCE(a.mcp_servers, '{}'::jsonb)) attachment
                 WHERE attachment.value->>'use' = 'catalog:' || ms.name
             )
            WHERE ms.org_id = $1
              AND ms.owner_virtual_user_id IS NULL
              AND ms.status != 'deleted'
            GROUP BY ms.id
            "#,
        )
        .bind(org_id)
        .fetch_all(&self.pool)
        .await?)
    }
    pub async fn list_mcp_server_agent_usage_for_ids(
        &self,
        org_id: i64,
        server_ids: &[Uuid],
    ) -> Result<Vec<McpServerAgentUsageRow>> {
        if server_ids.is_empty() {
            return Ok(Vec::new());
        }
        Ok(sqlx::query_as::<_, McpServerAgentUsageRow>(
            r#"
            SELECT ms.id AS mcp_server_id,
                   COUNT(DISTINCT a.id) FILTER (WHERE a.id IS NOT NULL) AS used_by_agents
            FROM mcp_servers ms
            LEFT JOIN agents a
              ON a.org_id = ms.org_id
             AND a.status = 'active'
             AND a.archived_at IS NULL
             AND a.deleted_at IS NULL
             AND EXISTS (
                 SELECT 1
                 FROM jsonb_each(COALESCE(a.mcp_servers, '{}'::jsonb)) attachment
                 WHERE attachment.value->>'use' = 'catalog:' || ms.name
             )
            WHERE ms.org_id = $1
              AND ms.id = ANY($2)
              AND ms.owner_virtual_user_id IS NULL
              AND ms.status != 'deleted'
            GROUP BY ms.id
            "#,
        )
        .bind(org_id)
        .bind(server_ids)
        .fetch_all(&self.pool)
        .await?)
    }

    pub async fn get_mcp_server_agent_names(
        &self,
        org_id: i64,
        server_id: McpServerId,
        limit: i64,
    ) -> Result<McpServerAgentNamesRow> {
        Ok(sqlx::query_as::<_, McpServerAgentNamesRow>(
            r#"
            WITH matching AS (
                SELECT DISTINCT a.id, COALESCE(NULLIF(a.display_name, ''), a.name) AS agent_name
                FROM mcp_servers ms
                JOIN agents a
                  ON a.org_id = ms.org_id
                 AND a.status = 'active'
                 AND a.archived_at IS NULL
                 AND a.deleted_at IS NULL
                 AND EXISTS (
                     SELECT 1
                     FROM jsonb_each(COALESCE(a.mcp_servers, '{}'::jsonb)) attachment
                     WHERE attachment.value->>'use' = 'catalog:' || ms.name
                 )
                WHERE ms.org_id = $1
                  AND ms.id = $2
                  AND ms.owner_virtual_user_id IS NULL
                  AND ms.status != 'deleted'
            )
            SELECT COALESCE(
                       ARRAY(
                           SELECT agent_name
                           FROM matching
                           ORDER BY LOWER(agent_name), agent_name
                           LIMIT $3
                       ),
                       ARRAY[]::text[]
                   ) AS agent_names,
                   (SELECT COUNT(*) FROM matching) AS total_count
            "#,
        )
        .bind(org_id)
        .bind(server_id)
        .bind(limit)
        .fetch_one(&self.pool)
        .await?)
    }

    /// List only active MCP servers (for capability listing)
    pub async fn list_active_mcp_servers(&self, org_id: i64) -> Result<Vec<McpServerRow>> {
        let rows = sqlx::query_as::<_, McpServerRow>(sql!(
            r#"
            SELECT {McpServerRow}
            FROM mcp_servers
            WHERE org_id = $1 AND status = 'active' AND owner_virtual_user_id IS NULL
            ORDER BY name ASC
            "#
        ))
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
        self.update_mcp_server_owned_by(org_id, None, id, input)
            .await
    }

    /// Update a row owned by `owner`, or a catalog row when `owner` is None.
    pub(super) async fn update_mcp_server_owned_by(
        &self,
        org_id: i64,
        owner: Option<Uuid>,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        let clear_api_key = input
            .api_key_encrypted
            .as_ref()
            .is_some_and(|bytes| bytes.is_empty());
        // Handle api_key_set: if we're updating the encrypted key, also update the flag
        let api_key_set = match input.api_key_encrypted.as_ref() {
            Some(bytes) if bytes.is_empty() => Some(false),
            Some(_) => Some(true),
            None => None,
        };
        let api_key_encrypted = match input.api_key_encrypted.as_ref() {
            Some(bytes) if bytes.is_empty() => None,
            Some(bytes) => Some(bytes.clone()),
            None => None,
        };

        let row = sqlx::query_as::<_, McpServerRow>(
            sql!(r#"
            UPDATE mcp_servers
            SET
                name = COALESCE($3, name),
                description = COALESCE($4, description),
                url = COALESCE($5, url),
                transport_type = COALESCE($6, transport_type),
                status = COALESCE($7, status),
                api_key_encrypted = CASE WHEN $12 THEN NULL ELSE COALESCE($8, api_key_encrypted) END,
                api_key_set = COALESCE($9, api_key_set),
                headers = COALESCE($10, headers),
                settings = COALESCE($11, settings)
            WHERE org_id = $1 AND id = $2 AND owner_virtual_user_id IS NOT DISTINCT FROM $13
            RETURNING {McpServerRow}
            "#),
        )
        .bind(org_id)
        .bind(id)
        .bind(&input.name)
        .bind(&input.description)
        .bind(&input.url)
        .bind(&input.transport_type)
        .bind(&input.status)
        .bind(&api_key_encrypted)
        .bind(api_key_set)
        .bind(&input.headers)
        .bind(&input.settings)
        .bind(clear_api_key)
        .bind(owner)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Store discovered display metadata. Does not touch operator name or description.
    pub async fn set_mcp_server_presentation(
        &self,
        org_id: i64,
        id: Uuid,
        presentation: serde_json::Value,
    ) -> Result<()> {
        sqlx::query("UPDATE mcp_servers SET presentation = $3 WHERE org_id = $1 AND id = $2")
            .bind(org_id)
            .bind(id)
            .bind(presentation)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Update cached tools for an MCP server
    pub async fn update_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateMcpServerTools,
    ) -> Result<Option<McpServerRow>> {
        let row = sqlx::query_as::<_, McpServerRow>(sql!(
            r#"
            UPDATE mcp_servers
            SET
                cached_tools = $3,
                tools_cached_at = NOW()
            WHERE org_id = $1 AND id = $2 AND owner_virtual_user_id IS NULL
            RETURNING {McpServerRow}
            "#
        ))
        .bind(org_id)
        .bind(id)
        .bind(&input.cached_tools)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    pub async fn clear_mcp_server_tools(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<McpServerRow>> {
        Ok(sqlx::query_as::<_, McpServerRow>(sql!(
            r#"
            UPDATE mcp_servers
            SET cached_tools = '[]'::jsonb, tools_cached_at = NULL
            WHERE org_id = $1 AND id = $2 AND owner_virtual_user_id IS NULL
            RETURNING {McpServerRow}
            "#
        ))
        .bind(org_id)
        .bind(id)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn get_mcp_service_tool_cache(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
        cache_scope: &str,
        credential_hash: &str,
    ) -> Result<Option<McpServiceToolCacheRow>> {
        Ok(sqlx::query_as::<_, McpServiceToolCacheRow>(sql!(
            r#"
            SELECT {McpServiceToolCacheRow}
            FROM mcp_service_tool_caches
            WHERE org_id = $1
              AND mcp_server_id = $2
              AND agent_id = $3
              AND cache_scope = $4
              AND credential_hash = $5
            "#
        ))
        .bind(org_id)
        .bind(mcp_server_id)
        .bind(agent_id)
        .bind(cache_scope)
        .bind(credential_hash)
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn upsert_mcp_service_tool_cache(
        &self,
        input: UpsertMcpServiceToolCache,
    ) -> Result<McpServiceToolCacheRow> {
        Ok(sqlx::query_as::<_, McpServiceToolCacheRow>(sql!(
            r#"
            INSERT INTO mcp_service_tool_caches (
                org_id, mcp_server_id, agent_id, cache_scope, credential_hash,
                cached_tools, ttl_ms
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            ON CONFLICT (org_id, mcp_server_id, agent_id, cache_scope, credential_hash)
            DO UPDATE SET
                cached_tools = EXCLUDED.cached_tools,
                ttl_ms = EXCLUDED.ttl_ms,
                tools_cached_at = NOW()
            RETURNING {McpServiceToolCacheRow}
            "#
        ))
        .bind(input.org_id)
        .bind(input.mcp_server_id)
        .bind(input.agent_id)
        .bind(&input.cache_scope)
        .bind(&input.credential_hash)
        .bind(&input.cached_tools)
        .bind(input.ttl_ms)
        .fetch_one(&self.pool)
        .await?)
    }

    pub async fn delete_mcp_service_tool_caches(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
    ) -> Result<u64> {
        Ok(sqlx::query(
            r#"
            DELETE FROM mcp_service_tool_caches
            WHERE org_id = $1 AND mcp_server_id = $2 AND agent_id = $3
            "#,
        )
        .bind(org_id)
        .bind(mcp_server_id)
        .bind(agent_id)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    pub async fn delete_mcp_service_tool_cache(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
        cache_scope: &str,
        credential_hash: &str,
    ) -> Result<u64> {
        Ok(sqlx::query(
            r#"
            DELETE FROM mcp_service_tool_caches
            WHERE org_id = $1
              AND mcp_server_id = $2
              AND agent_id = $3
              AND cache_scope = $4
              AND credential_hash = $5
            "#,
        )
        .bind(org_id)
        .bind(mcp_server_id)
        .bind(agent_id)
        .bind(cache_scope)
        .bind(credential_hash)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    pub async fn delete_obsolete_mcp_service_private_tool_caches(
        &self,
        org_id: i64,
        mcp_server_id: Uuid,
        agent_id: Uuid,
        current_credential_hash: &str,
    ) -> Result<u64> {
        Ok(sqlx::query(
            r#"
            DELETE FROM mcp_service_tool_caches
            WHERE org_id = $1
              AND mcp_server_id = $2
              AND agent_id = $3
              AND cache_scope = 'private'
              AND credential_hash <> $4
            "#,
        )
        .bind(org_id)
        .bind(mcp_server_id)
        .bind(agent_id)
        .bind(current_credential_hash)
        .execute(&self.pool)
        .await?
        .rows_affected())
    }

    pub async fn delete_mcp_server(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE mcp_servers
            SET status = 'archived', archived_at = COALESCE(archived_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND id = $2 AND status IN ('active', 'disabled')
              AND owner_virtual_user_id IS NULL
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
              AND owner_virtual_user_id IS NULL
            "#,
        )
        .bind(org_id)
        .bind(id)
        .execute(&self.pool)
        .await?;

        Ok(result.rows_affected() > 0)
    }
}
