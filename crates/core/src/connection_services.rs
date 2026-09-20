//! Neutral contracts for provider credentials and user connections.

use crate::error::Result;
use crate::typed_id::SessionId;
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
    /// Resolve default credentials for a provider type (for example `openai`).
    ///
    /// Implementations may apply environment fallbacks internally, but tools
    /// should never read provider env vars directly.
    async fn get_default_provider_credentials(
        &self,
        provider_type: &str,
    ) -> Result<Option<ProviderCredentials>>;
}

/// Resolves user connection tokens (e.g. GitHub) lazily at tool execution time.
///
/// Instead of eagerly injecting tokens at session creation, tools call this
/// resolver when they need a token. If the user hasn't connected, returns None.
#[async_trait]
pub trait UserConnectionResolver: Send + Sync {
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
    /// This is deliberately *not* `get_connection_token`: that one prefers an
    /// agent identity grant and falls back to the session owner's user grant,
    /// so the remote account a write lands under can change because session
    /// wiring changed. For MCP the acting identity is configuration, so each
    /// `actsAs` reads exactly one store and nothing else (EVE-1029, D2 of
    /// `knowledge/integrations/agent-mcp-attachments.md`):
    ///
    /// - [`crate::mcp_server::McpServerActsAs::None`] reads no connection store at all.
    /// - [`crate::mcp_server::McpServerActsAs::Service`] reads only the agent identity's grant.
    /// - [`crate::mcp_server::McpServerActsAs::User`] reads only the invoking user's grant, and
    ///   only when a human actually initiated the session.
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
        _acts_as: crate::mcp_server::McpServerActsAs,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    /// Invalidate an MCP credential after the remote server rejects it.
    ///
    /// Implementations that own persistent grants can remove the credential
    /// selected by `acts_as` and its credential-scoped tool cache. The default
    /// is a no-op.
    async fn invalidate_mcp_connection(
        &self,
        _session_id: SessionId,
        _provider: &str,
        _acts_as: crate::mcp_server::McpServerActsAs,
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
