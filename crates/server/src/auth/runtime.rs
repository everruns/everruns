//! Consumer self-service authority, separate from management authentication.
use super::middleware::{AuthError, AuthUser};
use super::{AuthState, ResolvedOrg};
use axum::{
    extract::{FromRef, FromRequestParts},
    http::{header, request::Parts},
};
use base64::Engine;
use everruns_contracts::typed_id::VirtualUserId;

#[derive(Debug, Clone)]
pub struct RuntimeAccount {
    pub org_id: i64,
    pub id: VirtualUserId,
    pub management: Option<ResolvedOrg>,
    pub channel_id: Option<String>,
}
impl RuntimeAccount {
    pub async fn from_token(auth: &AuthState, token: &str) -> Result<Self, AuthError> {
        let claims = super::jwt::JwtService::new(auth.config.jwt.clone())
            .validate_runtime_token(token)
            .map_err(|_| AuthError::unauthorized("Invalid runtime credential"))?;
        let db = auth
            .db
            .as_ref()
            .ok_or_else(|| AuthError::internal("Runtime identity unavailable"))?;
        let id = claims
            .sub
            .parse()
            .map_err(|_| AuthError::unauthorized("Invalid runtime credential"))?;
        let user = db
            .get_virtual_user(claims.org_id, id)
            .await
            .map_err(|_| AuthError::internal("Runtime identity unavailable"))?
            .ok_or_else(|| AuthError::unauthorized("Runtime account unavailable"))?;
        if user.status != "active" || user.usage != "end_user" {
            return Err(AuthError::unauthorized("Runtime account unavailable"));
        }
        let channel = db
            .get_ingress_channel_by_public_id(&claims.channel_id)
            .await
            .map_err(|_| AuthError::internal("Channel unavailable"))?
            .ok_or_else(|| AuthError::unauthorized("Runtime channel unavailable"))?;
        if channel.org_id != claims.org_id
            || !channel.enabled
            || channel.channel_status != "live"
            || channel.agent_status != "active"
            || channel.exposures_suspended
        {
            return Err(AuthError::unauthorized("Runtime channel unavailable"));
        }
        if !db
            .list_virtual_user_bindings(claims.org_id, id)
            .await
            .map_err(|_| AuthError::internal("Runtime identity unavailable"))?
            .iter()
            .any(|b| b.id == claims.binding_id)
        {
            return Err(AuthError::unauthorized("Runtime identity binding revoked"));
        }
        Ok(Self {
            org_id: claims.org_id,
            id,
            management: None,
            channel_id: Some(claims.channel_id),
        })
    }
    pub async fn connection_target(
        &self,
        db: &crate::storage::StorageBackend,
        resolver: &dyn everruns_core::PermissionResolver,
        raw: &str,
    ) -> Result<VirtualUserId, crate::domains::common::CommandError> {
        if let Some(org) = &self.management {
            return crate::domains::virtual_users::connection_target(
                db,
                resolver,
                &everruns_core::Caller::from(org),
                raw,
            )
            .await;
        }
        if !self.permits_self(raw) {
            return Err(crate::domains::common::CommandError::forbidden(
                "Runtime credentials permit self-service only",
            ));
        }
        Ok(self.id)
    }
    /// Consumer setup is bounded by the channel's effective user MCP attachments.
    pub async fn allowed_mcp_providers(
        &self,
        db: &crate::storage::StorageBackend,
    ) -> anyhow::Result<Option<std::collections::HashSet<String>>> {
        let Some(channel_id) = &self.channel_id else {
            return Ok(None);
        };
        let channel = db
            .get_ingress_channel_by_public_id(channel_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("Channel unavailable"))?;
        let agent =
            crate::domains::agents::queries::resolve(db, self.org_id, &channel.agent_public_id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("Agent unavailable"))?;
        let harness = crate::domains::harnesses::queries::resolve_effective(
            db,
            self.org_id,
            channel.harness_id.into(),
        )
        .await?
        .ok_or_else(|| anyhow::anyhow!("Harness unavailable"))?;
        let registry = crate::platform::oss_capability_registry();
        let mut capabilities = harness.capabilities.clone();
        capabilities.extend(agent.capabilities.clone());
        let expanded =
            everruns_core::capabilities::resolve_capability_configs(&capabilities, &registry)?;
        let contributed =
            everruns_core::capabilities::collect_capability_mcp_servers(&expanded, &registry);
        let merged = everruns_core::merge_scoped_mcp_servers(
            &contributed,
            &everruns_core::merge_scoped_mcp_servers(&harness.mcp_servers, &agent.mcp_servers),
        );
        let mut providers = std::collections::HashSet::new();
        for server in merged
            .values()
            .filter(|s| s.acts_as == everruns_core::McpServerActsAs::User)
        {
            if let Some(preset) = &server.preset {
                if let Some(row) = db
                    .get_mcp_server_by_name(self.org_id, preset.catalog_name())
                    .await?
                    && row.status == "active"
                {
                    providers.insert(everruns_core::mcp_oauth_provider_id_for_uuid(row.id.uuid()));
                }
            } else if let Some(provider) = &server.oauth_provider_id {
                providers.insert(provider.clone());
            }
        }
        // An agent that uses the person's own MCP servers lets them sign in to
        // those servers: a custom server signs in under its own id, a
        // catalog-sourced one under its preset's.
        let uses_user_servers = expanded.iter().any(|config| {
            config.capability_id() == everruns_capabilities::capabilities::USER_MCP_CAPABILITY_ID
                && everruns_capabilities::capabilities::user_mcp_use_enabled(config.config_value())
        });
        if uses_user_servers {
            for server in db
                .list_user_mcp_servers(self.org_id, self.id.uuid())
                .await?
            {
                let id = server.catalog_mcp_server_id.unwrap_or(server.row.id.uuid());
                providers.insert(everruns_core::mcp_oauth_provider_id_for_uuid(id));
            }
        }
        Ok(Some(providers))
    }
    pub async fn permits_provider(
        &self,
        db: &crate::storage::StorageBackend,
        provider: &str,
    ) -> anyhow::Result<bool> {
        if !provider.starts_with("mcp_oauth_") {
            return Ok(true);
        }
        Ok(self
            .allowed_mcp_providers(db)
            .await?
            .is_none_or(|allowed| allowed.contains(provider)))
    }
    pub fn permits_self(&self, raw: &str) -> bool {
        raw == "me" || raw.parse::<VirtualUserId>().ok() == Some(self.id)
    }
}
impl<S> FromRequestParts<S> for RuntimeAccount
where
    S: Send + Sync,
    AuthState: FromRef<S>,
{
    type Rejection = AuthError;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, AuthError> {
        let auth = AuthState::from_ref(state);
        if let Some(token) = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.split_once(' '))
            .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
            .map(|(_, t)| t)
            && (super::jwt::JwtService::new(auth.config.jwt.clone())
                .validate_runtime_token(token)
                .is_ok()
                || token
                    .split('.')
                    .nth(1)
                    .and_then(|p| {
                        base64::engine::general_purpose::URL_SAFE_NO_PAD
                            .decode(p)
                            .ok()
                    })
                    .and_then(|p| serde_json::from_slice::<serde_json::Value>(&p).ok())
                    .is_some_and(|p| p["token_type"] == "runtime_access"))
        {
            return Self::from_token(&auth, token).await;
        }
        let management = AuthUser::from_request_parts(parts, state).await?;
        let org = ResolvedOrg::from_request_parts(parts, state).await?;
        let db = auth
            .db
            .as_ref()
            .ok_or_else(|| AuthError::internal("Runtime identity unavailable"))?;
        let user = db
            .default_virtual_user(org.org_id, management.id)
            .await
            .map_err(|_| AuthError::internal("Runtime identity unavailable"))?;
        Ok(Self {
            org_id: org.org_id,
            id: user.id,
            management: Some(org),
            channel_id: None,
        })
    }
}
