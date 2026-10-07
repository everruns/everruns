//! Neutral contracts for provider credentials and user connections.

use crate::runtime::error::Result;
use crate::runtime::typed_id::SessionId;
use async_trait::async_trait;
use uuid::Uuid;

/// Provider credentials resolved for tool-side API clients.
#[derive(Debug, Clone)]
pub struct ProviderCredentials {
    pub api_key: String,
    pub base_url: Option<String>,
}

#[async_trait]
pub trait ProviderCredentialStore: Send + Sync {
    /// Resolve a trusted catalog selection. Never falls back to another account.
    async fn get_decision_model(
        &self,
        _model_id: Option<&str>,
        _session_id: SessionId,
    ) -> Result<Option<DecisionModelBinding>> {
        Err(crate::runtime::error::AgentLoopError::store(
            "Decision model resolution is unavailable",
        ))
    }

    /// Who answers deployment-owned decision checks (guardrail `jev` checks,
    /// Slack relevance) for this session's org.
    ///
    /// `Deployment` keeps the deployment's decisions. `Organization` carries the
    /// org's decision default. An org that opted in but has no usable model is
    /// an error, never `Deployment`: callers fail open or stay silent instead of
    /// spending deployment keys (THREAT[TM-LLM-037]). Hosts without org settings
    /// keep the deployment.
    async fn get_system_decision_model(
        &self,
        _session_id: SessionId,
    ) -> Result<SystemDecisionModel> {
        Ok(SystemDecisionModel::Deployment)
    }

    /// Resolve default credentials for a provider type (for example `openai`).
    ///
    /// Implementations may apply environment fallbacks internally, but tools
    /// should never read provider env vars directly.
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>>;
}

/// An MCP credential and the concrete identity (`user` or `service`) whose
/// grant supplied it.
#[derive(Clone, PartialEq, Eq)]
pub struct McpResolvedCredential {
    /// Decrypted bearer token.
    pub token: String,
    /// Identity whose grant the token came from.
    pub acted_as: crate::runtime::mcp_server::McpServerActsAs,
}

impl std::fmt::Debug for McpResolvedCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpResolvedCredential")
            .field("token", &"<redacted>")
            .field("acted_as", &self.acted_as)
            .finish()
    }
}

/// The source an org picked for deployment-owned decision checks.
#[derive(Clone)]
pub enum SystemDecisionModel {
    /// The deployment's decisions service answers.
    Deployment,
    /// The org's decision default answers, through the host's model boundary.
    Organization(DecisionModelBinding),
}

/// Runs an org-selected decision model through the host's egress, budget, and
/// usage path, the same path the Jev tool takes.
#[async_trait]
pub trait DecisionModelExecutor: Send + Sync {
    async fn evaluate(
        &self,
        binding: DecisionModelBinding,
        request: crate::runtime::decisions::DecisionRequest,
        context: &crate::runtime::tool_context::ToolContext,
    ) -> Result<crate::runtime::decisions::DecisionOutcome>;
}

/// Tool-context extension carrying the host's [`DecisionModelExecutor`].
pub struct DecisionModelExecutorExt(pub std::sync::Arc<dyn DecisionModelExecutor>);

/// Resolves user connection tokens (e.g. GitHub) lazily at tool execution time.
///
/// Instead of eagerly injecting tokens at session creation, tools call this
/// resolver when they need a token. If the user hasn't connected, returns None.
#[async_trait]
pub trait UserConnectionResolver: Send + Sync {
    /// Bind credential resolution to a stored input-message invocation. Implementations
    /// without this capability remain service-only and fail closed for consumer grants.
    fn for_execution(
        &self,
        _input_message_id: Uuid,
    ) -> Option<std::sync::Arc<dyn UserConnectionResolver>> {
        None
    }

    /// Bind a configured MCP attachment at a remote execution boundary.
    fn for_mcp_operation(
        &self,
        _server_prefix: &str,
    ) -> Option<std::sync::Arc<dyn UserConnectionResolver>> {
        None
    }

    /// Get a decrypted connection token for the given provider.
    /// Returns None if the user has no connection for this provider.
    async fn get_connection_token(
        &self,
        session_id: SessionId,
        provider: &str,
    ) -> Result<Option<String>>;

    /// Resolve a decrypted MCP connection token as a pure function of the
    /// attachment's `actsAs`.
    ///
    /// Acting identity is explicit configuration over one credential store:
    /// none reads no grant; service reads the responding agent's service virtual
    /// user; user reads the current invocation's end user. Neither identity
    /// falls back to the other, and session ownership never selects a grant.
    ///
    /// `Ok(None)` means "no credential", which callers surface as
    /// `connection_required` rather than an unauthenticated request.
    ///
    /// THREAT[TM-TOOL-041]: the default implementation is fail-closed on
    /// purpose. A resolver that has not opted in must never silently fall back
    /// to the identity-preferring lookup, because that is the substitution this
    /// method exists to remove.
    async fn get_mcp_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
        _acts_as: crate::runtime::mcp_server::McpServerActsAs,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// Resolve an MCP credential together with the identity that supplied it.
    ///
    /// `user_or_service` tries the invoking user's grant first and falls back
    /// to the agent's service grant; every other value reads exactly one store,
    /// as [`Self::get_mcp_connection_token`] does. An unattended run has no
    /// invoking user, so its user lookup is empty and it uses the service grant.
    /// The returned `acted_as` is always concrete (`user` or `service`), so a
    /// caller can record which account a call ran as.
    async fn get_mcp_connection_credential(
        &self,
        session_id: SessionId,
        provider: &str,
        acts_as: crate::runtime::mcp_server::McpServerActsAs,
    ) -> Result<Option<McpResolvedCredential>> {
        for identity in acts_as.resolution_order() {
            if let Some(token) = self
                .get_mcp_connection_token(session_id, provider, *identity)
                .await?
            {
                return Ok(Some(McpResolvedCredential {
                    token,
                    acted_as: *identity,
                }));
            }
        }
        Ok(None)
    }

    /// Resolve the responding agent's own service-account API-key connection
    /// for `provider`: its key and the provider metadata stored with it.
    ///
    /// Reads only the agent's service virtual user. It never reads the
    /// invoking end user's connections and never falls back to them, or to a
    /// management user's. `Ok(None)` means "no service connection".
    ///
    /// THREAT[TM-TOOL-041]: fail-closed by default, like
    /// [`Self::get_mcp_connection_token`]; a resolver that has not opted in
    /// must not substitute the identity-preferring lookup.
    async fn get_service_api_key_connection(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<ServiceApiKeyConnection>> {
        Ok(None)
    }

    /// Invalidate an MCP credential after the remote server rejects it.
    ///
    /// Implementations that own persistent grants can remove the credential
    /// selected by `acts_as` and its credential-scoped tool cache. The default
    /// is a no-op. Implementations must preserve a replacement grant when its
    /// fingerprint differs from the credential the remote server rejected.
    async fn invalidate_mcp_connection(
        &self,
        _session_id: SessionId,
        _provider: &str,
        _acts_as: crate::runtime::mcp_server::McpServerActsAs,
        _rejected_credential_fingerprint: &str,
    ) -> Result<()> {
        Ok(())
    }

    /// Resolve the user ID of the connection used for a session/provider pair.
    ///
    /// This is used by leased resources to bind cleanup to the same provider
    /// identity that created the remote resource.
    async fn get_connection_user(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<Uuid>> {
        Ok(None)
    }

    /// Resolve a provider token for a specific user.
    ///
    /// Cleanup workers use this to avoid "first org member wins" behavior when
    /// cleaning resources created by a specific provider connection owner.
    async fn get_connection_token_for_user(
        &self,
        _user_id: Uuid,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// Resolve one exact connection owned by the given virtual user. Sandbox
    /// lifecycle uses this for organization accounts so provisioning and later
    /// cleanup cannot drift to a different account for the same provider.
    async fn get_connection_token_for_connection(
        &self,
        _connection_id: Uuid,
        _virtual_user_id: Uuid,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// Get provider-specific metadata stored alongside the connection.
    /// Returns None if no metadata is stored or no connection exists.
    async fn get_connection_metadata(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<serde_json::Value>> {
        Ok(None)
    }
}

/// An API key held by an agent's own service account, with the provider
/// metadata stored beside it (for example an AgentMail inbox id).
#[derive(Clone)]
pub struct ServiceApiKeyConnection {
    pub api_key: String,
    pub metadata: Option<serde_json::Value>,
}

impl std::fmt::Debug for ServiceApiKeyConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceApiKeyConnection")
            .field("api_key", &"[redacted]")
            .field("metadata", &self.metadata)
            .finish()
    }
}

/// Resolved account material stays internal to the host boundary.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct DecisionModelBinding {
    pub model_id: String,
    pub provider_id: String,
    pub provider_type: String,
    pub model: String,
    pub profile_key: String,
    pub api_key: String,
    pub base_url: Option<String>,
    pub headers: std::collections::BTreeMap<String, String>,
}
