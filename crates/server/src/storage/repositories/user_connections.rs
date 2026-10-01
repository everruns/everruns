//! Compatibility DTO adapters over the canonical virtual-user connection store.
use super::super::mcp_catalog::UserMcpConnectionRow;
use super::super::models::*;
use super::Database;
use anyhow::Result;
use everruns_provider::typed_id::{SessionId, VirtualUserId};
use uuid::Uuid;
impl Database {
    pub async fn upsert_user_connection(
        &self,
        input: CreateUserConnectionRow,
    ) -> Result<UserConnectionRow> {
        Ok(self
            .upsert_virtual_user_connection(CreateVirtualUserConnectionRow {
                virtual_user_id: VirtualUserId::from_uuid(input.user_id),
                provider: input.provider,
                connection_type: input.connection_type,
                provider_user_id: input.provider_user_id,
                provider_username: input.provider_username,
                access_token_encrypted: input.access_token_encrypted,
                refresh_token_encrypted: input.refresh_token_encrypted,
                scopes: input.scopes,
                expires_at: input.expires_at,
                installation_id: input.installation_id,
                provider_metadata: input.provider_metadata,
            })
            .await?
            .into())
    }
    pub async fn get_user_connection(
        &self,
        id: Uuid,
        provider: &str,
    ) -> Result<Option<UserConnectionRow>> {
        Ok(self
            .get_virtual_user_connection(VirtualUserId::from_uuid(id), provider)
            .await?
            .map(Into::into))
    }
    pub async fn list_user_connections(&self, id: Uuid) -> Result<Vec<UserConnectionRow>> {
        Ok(self
            .list_virtual_user_connections(VirtualUserId::from_uuid(id))
            .await?
            .into_iter()
            .map(Into::into)
            .collect())
    }
    pub async fn update_user_connection_oauth_tokens(
        &self,
        input: UpdateOAuthConnectionTokens,
    ) -> Result<Option<UserConnectionRow>> {
        Ok(self
            .update_virtual_user_connection_oauth_tokens(input)
            .await?
            .map(Into::into))
    }
    pub async fn delete_user_connection(&self, id: Uuid, provider: &str) -> Result<bool> {
        self.delete_virtual_user_connection(VirtualUserId::from_uuid(id), provider)
            .await
    }
    pub async fn list_user_mcp_connections(
        &self,
        org: i64,
        id: Uuid,
    ) -> Result<Vec<UserMcpConnectionRow>> {
        let mut rows = vec![];
        for c in self.list_user_connections(id).await? {
            if let Some(server_id) = c
                .provider
                .strip_prefix("mcp_oauth_")
                .and_then(|v| v.parse::<Uuid>().ok())
                && let Some(s) = self.get_mcp_server(org, server_id).await?
            {
                rows.push(UserMcpConnectionRow {
                    connection_id: c.id,
                    provider: c.provider,
                    provider_username: c.provider_username,
                    scopes: c.scopes,
                    connected_at: c.created_at,
                    server_id: s.id,
                    server_name: s.name,
                    server_url: s.url,
                    server_status: s.status,
                });
            }
        }
        rows.sort_by_key(|r| r.server_name.to_lowercase());
        Ok(rows)
    }
    pub async fn list_user_mcp_connections_page(
        &self,
        org: i64,
        id: Uuid,
        cursor: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<UserMcpConnectionRow>> {
        let mut rows = self.list_user_mcp_connections(org, id).await?;
        rows.retain(|r| cursor.is_none_or(|c| r.connection_id < c));
        rows.sort_by_key(|r| std::cmp::Reverse(r.connection_id));
        rows.truncate(limit.max(0) as usize);
        Ok(rows)
    }
    // These unbound session helpers serve service accounts only. Consumer credentials
    // require a recorded input-message subject in DbConnectionResolver.
    pub async fn get_virtual_user_connection_row_for_session(
        &self,
        session: SessionId,
        provider: &str,
    ) -> Result<Option<VirtualUserConnectionRow>> {
        let Some(s) = self.get_session_unscoped(session).await? else {
            return Ok(None);
        };
        let Some(id) = s.virtual_user_id else {
            return Ok(None);
        };
        let Some(v) = self.get_virtual_user(s.org_id, id).await? else {
            return Ok(None);
        };
        if v.usage != "service" || v.status != "active" {
            return Ok(None);
        };
        self.get_virtual_user_connection(id, provider).await
    }
    pub async fn get_virtual_user_connection_for_session(
        &self,
        s: SessionId,
        p: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_virtual_user_connection_row_for_session(s, p)
            .await?
            .and_then(|r| r.access_token_encrypted))
    }
    pub async fn get_connection_token_for_session(
        &self,
        s: SessionId,
        p: &str,
    ) -> Result<Option<Vec<u8>>> {
        self.get_virtual_user_connection_for_session(s, p).await
    }
    pub async fn get_connection_metadata_for_session(
        &self,
        s: SessionId,
        p: &str,
    ) -> Result<Option<serde_json::Value>> {
        Ok(self
            .get_virtual_user_connection_row_for_session(s, p)
            .await?
            .and_then(|r| r.provider_metadata))
    }
    pub async fn get_connection_user_for_session(
        &self,
        s: SessionId,
        p: &str,
    ) -> Result<Option<Uuid>> {
        Ok(self
            .get_virtual_user_connection_row_for_session(s, p)
            .await?
            .map(|r| r.virtual_user_id.uuid()))
    }
    pub async fn session_has_human_initiator(&self, _s: SessionId) -> Result<bool> {
        Ok(false)
    }
    pub async fn get_owner_user_connection_for_session(
        &self,
        _s: SessionId,
        _p: &str,
    ) -> Result<Option<UserConnectionRow>> {
        Ok(None)
    }
    pub async fn get_connection_token_for_user(
        &self,
        id: Uuid,
        p: &str,
    ) -> Result<Option<Vec<u8>>> {
        Ok(self
            .get_user_connection(id, p)
            .await?
            .and_then(|r| r.access_token_encrypted))
    }
    pub async fn get_installation_id_for_user(&self, id: Uuid, p: &str) -> Result<Option<i64>> {
        Ok(self
            .get_user_connection(id, p)
            .await?
            .and_then(|r| r.installation_id))
    }
    pub async fn get_installation_id_for_session(
        &self,
        s: SessionId,
        p: &str,
    ) -> Result<Option<i64>> {
        Ok(self
            .get_virtual_user_connection_row_for_session(s, p)
            .await?
            .and_then(|r| r.installation_id))
    }
    pub async fn get_user_id_by_installation_id(&self, p: &str, id: i64) -> Result<Option<Uuid>> {
        Ok(sqlx::query_scalar("SELECT virtual_user_id FROM virtual_user_connections WHERE provider=$1 AND installation_id=$2 ORDER BY created_at DESC LIMIT 1").bind(p).bind(id).fetch_optional(&self.pool).await?)
    }
}
