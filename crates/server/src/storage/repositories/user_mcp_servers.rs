// PostgreSQL repository: user MCP servers
//
// Spec: knowledge/integrations/user-mcp-servers.md
//
// Decision: a user MCP server is an `mcp_servers` row with
// `owner_virtual_user_id` set. Every catalog query in `mcp_servers.rs` filters
// owned rows out, so they never appear in the org catalog or resolve through
// `catalog:<name>`. The queries here are the only way to reach them, and each
// is scoped to the owning virtual user, except the two OAuth helpers that the
// connect flow and token refresh need for any row.

use super::super::models::*;
use super::Database;
use anyhow::Result;
use uuid::Uuid;

/// An MCP server row together with its owner, if a user owns it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct OwnedMcpServerRow {
    #[sqlx(flatten)]
    pub row: McpServerRow,
    pub owner_virtual_user_id: Option<Uuid>,
}

/// A user-owned MCP server row with the catalog preset it was added from.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserMcpServerRow {
    #[sqlx(flatten)]
    pub row: McpServerRow,
    pub catalog_mcp_server_id: Option<Uuid>,
    /// Whether the server loads on demand (user servers only).
    pub deferred: bool,
}

impl Database {
    /// Read a catalog or user-owned server with its owner. Only the OAuth
    /// connect flow and token refresh use this; they check the owner
    /// themselves before writing a credential.
    pub async fn get_mcp_server_with_owner(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<OwnedMcpServerRow>> {
        let sql = "SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at, owner_virtual_user_id FROM mcp_servers WHERE org_id = $1 AND id = $2";
        Ok(sqlx::query_as::<_, OwnedMcpServerRow>(sql)
            .bind(org_id)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    /// Persist OAuth client registration settings for a catalog or
    /// user-owned server. The connect flow registers a client before it
    /// redirects, whoever owns the server.
    pub async fn update_mcp_server_settings_any_owner(
        &self,
        org_id: i64,
        id: Uuid,
        settings: serde_json::Value,
    ) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE mcp_servers SET settings = $3, updated_at = NOW() WHERE org_id = $1 AND id = $2",
        )
        .bind(org_id)
        .bind(id)
        .bind(settings)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn create_user_mcp_server(
        &self,
        org_id: i64,
        owner: Uuid,
        catalog_mcp_server_id: Option<Uuid>,
        deferred: bool,
        input: CreateMcpServerRow,
    ) -> Result<UserMcpServerRow> {
        let headers = input.headers.unwrap_or(serde_json::json!({}));
        let settings = input.settings.unwrap_or(serde_json::json!({}));
        let api_key_set = input.api_key_encrypted.is_some();
        let sql = r#"
            INSERT INTO mcp_servers (org_id, owner_virtual_user_id, name, description, url, transport_type, api_key_encrypted, api_key_set, headers, settings, catalog_mcp_server_id, deferred)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
            RETURNING id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at, catalog_mcp_server_id, deferred
            "#;
        Ok(sqlx::query_as::<_, UserMcpServerRow>(sql)
            .bind(org_id)
            .bind(owner)
            .bind(&input.name)
            .bind(&input.description)
            .bind(&input.url)
            .bind(&input.transport_type)
            .bind(&input.api_key_encrypted)
            .bind(api_key_set)
            .bind(&headers)
            .bind(&settings)
            .bind(catalog_mcp_server_id)
            .bind(deferred)
            .fetch_one(&self.pool)
            .await?)
    }

    /// Live (active or disabled) servers a user owns, by name.
    pub async fn list_user_mcp_servers(
        &self,
        org_id: i64,
        owner: Uuid,
    ) -> Result<Vec<UserMcpServerRow>> {
        let sql = r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at, catalog_mcp_server_id, deferred FROM mcp_servers
            WHERE org_id = $1 AND owner_virtual_user_id = $2 AND status IN ('active', 'disabled')
            ORDER BY lower(name), id
            "#;
        Ok(sqlx::query_as::<_, UserMcpServerRow>(sql)
            .bind(org_id)
            .bind(owner)
            .fetch_all(&self.pool)
            .await?)
    }

    pub async fn get_user_mcp_server(
        &self,
        org_id: i64,
        owner: Uuid,
        id: Uuid,
    ) -> Result<Option<UserMcpServerRow>> {
        let sql = r#"
            SELECT id, org_id, name, description, url, transport_type, status, api_key_encrypted, api_key_set, headers, settings, cached_tools, tools_cached_at, created_at, updated_at, archived_at, deleted_at, catalog_mcp_server_id, deferred FROM mcp_servers
            WHERE org_id = $1 AND owner_virtual_user_id = $2 AND id = $3 AND status IN ('active', 'disabled')
            "#;
        Ok(sqlx::query_as::<_, UserMcpServerRow>(sql)
            .bind(org_id)
            .bind(owner)
            .bind(id)
            .fetch_optional(&self.pool)
            .await?)
    }

    pub async fn update_user_mcp_server(
        &self,
        org_id: i64,
        owner: Uuid,
        id: Uuid,
        input: UpdateMcpServer,
    ) -> Result<Option<McpServerRow>> {
        if self.get_user_mcp_server(org_id, owner, id).await?.is_none() {
            return Ok(None);
        }
        self.update_mcp_server_owned_by(org_id, Some(owner), id, input)
            .await
    }

    /// Choose whether a user server loads on demand.
    pub async fn set_user_mcp_server_deferred(
        &self,
        org_id: i64,
        owner: Uuid,
        id: Uuid,
        deferred: bool,
    ) -> Result<bool> {
        let result = sqlx::query(
            r#"
            UPDATE mcp_servers SET deferred = $4, updated_at = NOW()
            WHERE org_id = $1 AND owner_virtual_user_id = $2 AND id = $3
              AND status IN ('active', 'disabled')
            "#,
        )
        .bind(org_id)
        .bind(owner)
        .bind(id)
        .bind(deferred)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Remove a user server and the owner's credential for it. User servers
    /// have no archive step: nothing else references them.
    pub async fn delete_user_mcp_server(&self, org_id: i64, owner: Uuid, id: Uuid) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let result = sqlx::query(
            r#"
            UPDATE mcp_servers
            SET status = 'deleted', deleted_at = COALESCE(deleted_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND owner_virtual_user_id = $2 AND id = $3
              AND status IN ('active', 'disabled')
            "#,
        )
        .bind(org_id)
        .bind(owner)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if result.rows_affected() == 0 {
            return Ok(false);
        }
        sqlx::query(
            "DELETE FROM virtual_user_connections WHERE virtual_user_id = $1 AND provider = $2",
        )
        .bind(owner)
        .bind(everruns_core::mcp_oauth_provider_id_for_uuid(id))
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(true)
    }
}
