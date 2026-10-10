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

use super::Database;
use crate::storage::{CreateMcpServerRow, McpServerRow, UpdateMcpServer};
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

    /// Remove a user server and the owner's sign-in to it. User servers have
    /// no archive step: nothing else references them.
    ///
    /// Decision (knowledge/integrations/user-mcp-servers.md, D8): the list is
    /// where a person sees what they signed in to, so removing a server also
    /// signs them out. A catalog server's sign-in is the person's grant to the
    /// preset (`mcp_oauth_<preset uuid>`), shared with agent servers that act
    /// as them; it is deleted unless another of their listed servers still
    /// points at the same preset. Revoking the sign-in alone (the connection
    /// endpoint) keeps the server on the list as "Needs sign-in".
    pub async fn delete_user_mcp_server(&self, org_id: i64, owner: Uuid, id: Uuid) -> Result<bool> {
        let mut tx = self.pool.begin().await?;
        let removed: Option<(Option<Uuid>,)> = sqlx::query_as(
            r#"
            UPDATE mcp_servers
            SET status = 'deleted', deleted_at = COALESCE(deleted_at, NOW()), updated_at = NOW()
            WHERE org_id = $1 AND owner_virtual_user_id = $2 AND id = $3
              AND status IN ('active', 'disabled')
            RETURNING catalog_mcp_server_id
            "#,
        )
        .bind(org_id)
        .bind(owner)
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((catalog,)) = removed else {
            return Ok(false);
        };
        let still_listed = match catalog {
            Some(preset) => {
                sqlx::query_scalar::<_, bool>(
                    r#"
                SELECT EXISTS (
                    SELECT 1 FROM mcp_servers
                    WHERE org_id = $1 AND owner_virtual_user_id = $2
                      AND catalog_mcp_server_id = $3 AND status IN ('active', 'disabled')
                )
                "#,
                )
                .bind(org_id)
                .bind(owner)
                .bind(preset)
                .fetch_one(&mut *tx)
                .await?
            }
            None => false,
        };
        if !still_listed {
            sqlx::query(
                "DELETE FROM virtual_user_connections WHERE virtual_user_id = $1 AND provider = $2",
            )
            .bind(owner)
            .bind(everruns_core::mcp_oauth_provider_id_for_uuid(
                catalog.unwrap_or(id),
            ))
            .execute(&mut *tx)
            .await?;
        }
        tx.commit().await?;
        Ok(true)
    }

    /// Put a catalog preset on a person's list after they signed in to it.
    ///
    /// Decision (knowledge/integrations/user-mcp-servers.md, D8): a personal
    /// sign-in to a catalog server adds a user-owned row pointing at the
    /// preset, so an agent with the User MCP servers capability gets every
    /// server the person connected, wherever they connected it. Idempotent: a
    /// listed row for the preset (active or turned off) is kept as it is. A
    /// per-owner advisory lock serializes concurrent callbacks, so a repeated
    /// connect never adds a second row. Only active end-user virtual users and
    /// active catalog presets qualify, and a full list is left alone.
    ///
    /// Name rule, shared with migration 205's backfill: the preset's name, or
    /// `<name>-2`, `<name>-3`, ... when another of the person's servers already
    /// produces the same tool prefix.
    pub async fn list_catalog_server_for_owner(
        &self,
        org_id: i64,
        owner: Uuid,
        preset_id: Uuid,
        max_servers: usize,
    ) -> Result<CatalogListing> {
        let mut tx = self.pool.begin().await?;
        sqlx::query(
            "SELECT pg_advisory_xact_lock(hashtextextended('user_mcp_servers:' || $1::text, 0))",
        )
        .bind(owner)
        .execute(&mut *tx)
        .await?;
        let end_user = sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS (SELECT 1 FROM virtual_users WHERE org_id = $1 AND id = $2 AND usage = 'end_user' AND status = 'active')",
        )
        .bind(org_id)
        .bind(owner)
        .fetch_one(&mut *tx)
        .await?;
        if !end_user {
            return Ok(CatalogListing::Skipped("not an active end user"));
        }
        let preset: Option<(String, Option<String>, String, String)> = sqlx::query_as(
            r#"
            SELECT name, description, url, transport_type FROM mcp_servers
            WHERE org_id = $1 AND id = $2 AND owner_virtual_user_id IS NULL AND status = 'active'
            "#,
        )
        .bind(org_id)
        .bind(preset_id)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((name, description, url, transport_type)) = preset else {
            return Ok(CatalogListing::Skipped("not an active catalog preset"));
        };
        let listed: Vec<(Uuid, String, Option<Uuid>)> = sqlx::query_as(
            r#"
            SELECT id, name, catalog_mcp_server_id FROM mcp_servers
            WHERE org_id = $1 AND owner_virtual_user_id = $2 AND status IN ('active', 'disabled')
            ORDER BY id
            "#,
        )
        .bind(org_id)
        .bind(owner)
        .fetch_all(&mut *tx)
        .await?;
        if let Some((id, _, _)) = listed
            .iter()
            .find(|(_, _, catalog)| *catalog == Some(preset_id))
        {
            return Ok(CatalogListing::AlreadyListed { id: *id });
        }
        if listed.len() >= max_servers {
            return Ok(CatalogListing::Skipped("the list is full"));
        }
        let Some(name) = free_user_server_name(&name, listed.iter().map(|(_, n, _)| n.as_str()))
        else {
            return Ok(CatalogListing::Skipped("no free name"));
        };
        let id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO mcp_servers (org_id, owner_virtual_user_id, name, description, url, transport_type, api_key_set, headers, settings, catalog_mcp_server_id, deferred)
            VALUES ($1, $2, $3, $4, $5, $6, FALSE, '{}'::jsonb, '{"auth_mode": "none"}'::jsonb, $7, TRUE)
            RETURNING id
            "#,
        )
        .bind(org_id)
        .bind(owner)
        .bind(&name)
        .bind(&description)
        .bind(&url)
        .bind(&transport_type)
        .bind(preset_id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(CatalogListing::Added { id, name })
    }
}

/// Result of [`Database::list_catalog_server_for_owner`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CatalogListing {
    /// A row was added under this name.
    Added { id: Uuid, name: String },
    /// The person already lists the preset; nothing changed.
    AlreadyListed { id: Uuid },
    /// Nothing was written, for this reason.
    Skipped(&'static str),
}

/// Longest user server name (`validate_name` in the user servers domain).
const MAX_NAME_LEN: usize = 64;

/// The first free name for a catalog server on a person's list: `base`, then
/// `base-2`, `base-3`, ... A name is taken when it produces the same tool
/// prefix as one already listed. Migration 205 applies the same rule in SQL.
pub fn free_user_server_name<'a>(
    base: &str,
    listed: impl IntoIterator<Item = &'a str>,
) -> Option<String> {
    use everruns_core::mcp_server::{is_valid_mcp_server_name, sanitize_mcp_server_name};
    let taken: std::collections::HashSet<String> =
        listed.into_iter().map(sanitize_mcp_server_name).collect();
    let usable = |name: &str| {
        name.len() <= MAX_NAME_LEN
            && is_valid_mcp_server_name(name)
            && !taken.contains(&sanitize_mcp_server_name(name))
    };
    if usable(base) {
        return Some(base.to_string());
    }
    (2..=crate::domains::mcp_servers::user_servers::MAX_USER_MCP_SERVERS + 1).find_map(|n| {
        let suffix = format!("-{n}");
        let stem: String = base
            .chars()
            .take(MAX_NAME_LEN.saturating_sub(suffix.len()))
            .collect();
        let candidate = format!("{}{suffix}", stem.trim_end_matches(['-', '_']));
        usable(&candidate).then_some(candidate)
    })
}
