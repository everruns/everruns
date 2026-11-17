//! An agent's service MCP grants handed to its org's data plane (data plane
//! v2 step 11.2).
//!
//! A v2 org's turns run on its data plane, which refreshes the grant there
//! (`RefreshOwner::DataPlane`). The control plane hands over the grant as a
//! self-contained OAuth token set, and stores each refresh the data plane
//! reports, so the platform's copy never holds a spent refresh token.
//!
//! Decisions:
//! - Only a catalog server's own OAuth grant goes over. A connection-backed
//!   preset (GitHub and the like) and a person's grant stay on the platform
//!   for now (step 11.6).
//! - The refresh token is the decoded grant, bound to the server's current
//!   OAuth configuration: a grant written for an older configuration does
//!   not go over (it would not refresh either).
//! - A data plane refresh is stored only over the access token the
//!   platform last handed over or stored: a grant the person authorized
//!   again in the meantime wins.

use everruns_contracts::typed_id::{AgentId, VirtualUserId};
use everruns_core::mcp::oauth::TokenSet;
use uuid::Uuid;

use super::{
    AgentLoopError, DbConnectionResolver, McpRefreshGrant, OAuthScope, Result,
    UpdateOAuthConnectionTokens,
};

/// An agent's service grant for one MCP server, as its data plane runs it.
#[derive(Debug, Clone)]
pub struct DataPlaneGrant {
    /// The platform's connection row.
    pub connection_id: Uuid,
    pub virtual_user_id: VirtualUserId,
    /// The `mcp_oauth_<server>` provider the row is stored under.
    pub provider: String,
    /// The server's OAuth configuration the grant is bound to.
    pub config_hash: String,
    pub tokens: TokenSet,
}

impl DbConnectionResolver {
    /// The service grant `agent_id` holds for the catalog MCP server
    /// `provider` names, if it has a usable one.
    pub async fn agent_service_mcp_grant(
        &self,
        org_id: i64,
        agent_id: AgentId,
        provider: &str,
    ) -> Result<Option<DataPlaneGrant>> {
        let Some(server) = Self::parse_mcp_oauth_provider(provider) else {
            return Ok(None);
        };
        let store = |e: anyhow::Error| AgentLoopError::store(e.to_string());
        let Some(agent) = self
            .db
            .get_agent_config(org_id, agent_id)
            .await
            .map_err(store)?
        else {
            return Ok(None);
        };
        let Some(identity_id) = agent.virtual_user_id.filter(|_| agent.status == "active") else {
            return Ok(None);
        };
        let identity = self
            .db
            .get_virtual_user(org_id, identity_id)
            .await
            .map_err(store)?
            .filter(|v| v.status == "active" && v.usage == "service");
        if identity.is_none() || self.connection_backing(org_id, server).await?.is_some() {
            return Ok(None);
        }
        let Some(row) = self
            .db
            .get_virtual_user_connection(identity_id, provider)
            .await
            .map_err(store)?
        else {
            return Ok(None);
        };
        if row.connection_type != "oauth"
            || !self
                .authorize_connection(OAuthScope::Org(org_id), &row, None)
                .await?
        {
            return Ok(None);
        }
        let Some(access_token) = row.access_token_encrypted.as_deref() else {
            return Ok(None);
        };
        let access_token = self.decrypt(access_token, "OAuth access token")?;
        let Some(config) = self.oauth_client_config_in_org(org_id, server).await? else {
            return Ok(None);
        };
        let grant = row
            .refresh_token_encrypted
            .as_deref()
            .map(|value| self.decrypt(value, "OAuth refresh token"))
            .transpose()?
            .and_then(|value| McpRefreshGrant::decode(&value, &config.config_hash));
        Ok(Some(DataPlaneGrant {
            connection_id: row.id,
            virtual_user_id: identity_id,
            provider: provider.to_owned(),
            config_hash: config.config_hash,
            tokens: TokenSet {
                access_token,
                refresh_token: grant.as_ref().map(|g| g.refresh_token.clone()),
                token_type: "Bearer".to_owned(),
                expires_at: row.expires_at,
                scope: row.scopes,
                token_endpoint: Some(config.token_endpoint),
                resource: grant.map(|g| g.resource),
                client_id: Some(config.client_id),
                client_secret: config.client_secret,
            },
        }))
    }

    /// Store the token set the data plane refreshed `grant` to. False when
    /// the platform's row moved on (removed, or authorized again).
    pub async fn store_data_plane_refresh(
        &self,
        grant: &DataPlaneGrant,
        expected_access_token: &str,
    ) -> Result<bool> {
        let store = |e: anyhow::Error| AgentLoopError::store(e.to_string());
        let Some(row) = self
            .db
            .get_virtual_user_connection_by_id(
                grant.virtual_user_id,
                grant.connection_id,
                &grant.provider,
            )
            .await
            .map_err(store)?
        else {
            return Ok(false);
        };
        let Some(current) = row.access_token_encrypted.as_deref() else {
            return Ok(false);
        };
        if self.decrypt(current, "OAuth access token")? != expected_access_token {
            return Ok(false);
        }
        let encrypt = |value: &str| {
            self.encryption
                .encrypt_string(value)
                .map_err(|e| AgentLoopError::store(format!("Failed to encrypt OAuth token: {e}")))
        };
        let refresh_token_encrypted = match (&grant.tokens.refresh_token, &grant.tokens.resource) {
            (Some(refresh_token), Some(resource)) => encrypt(
                &McpRefreshGrant {
                    refresh_token: refresh_token.clone(),
                    resource: resource.clone(),
                    config_hash: grant.config_hash.clone(),
                }
                .encode(),
            )?,
            _ => row.refresh_token_encrypted.clone().unwrap_or_default(),
        };
        let updated = self
            .db
            .rotate_runtime_connection(
                current,
                UpdateOAuthConnectionTokens {
                    connection_id: row.id,
                    access_token_encrypted: encrypt(&grant.tokens.access_token)?,
                    refresh_token_encrypted,
                    expires_at: grant.tokens.expires_at,
                    scopes: grant.tokens.scope.clone(),
                },
            )
            .await
            .map_err(store)?;
        Ok(updated.is_some())
    }
}
