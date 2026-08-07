//! Pluggable MCP credential acquisition (knowledge/integrations/runtime-mcp.md D3).
//!
//! The current control-plane OAuth flow is browser/redirect based. Runtime
//! hosts are often CLI, so credential acquisition is an injectable trait: the
//! server implements it over its session-secret + connection-resolver web
//! OAuth, while a CLI host can supply static tokens or a device-code flow. The
//! transport layer stays stateless — token caching/refresh is owned by the
//! provider implementation.

use async_trait::async_trait;
use everruns_core::McpServerAuthMode;
use std::collections::HashMap;

/// A resolved credential to apply to an outbound MCP request.
#[derive(Debug, Clone, Default)]
pub struct McpCredential {
    /// Full `Authorization` header value, e.g. `"Bearer xyz"`.
    pub authorization: Option<String>,
    /// Additional headers to merge onto the request.
    pub headers: HashMap<String, String>,
}

impl McpCredential {
    /// Build a `Bearer` credential from a raw token.
    pub fn bearer(token: impl Into<String>) -> Self {
        Self {
            authorization: Some(format!("Bearer {}", token.into())),
            headers: HashMap::new(),
        }
    }

    /// Build a credential from a full `Authorization` header value.
    pub fn authorization(value: impl Into<String>) -> Self {
        Self {
            authorization: Some(value.into()),
            headers: HashMap::new(),
        }
    }
}

/// Identity of the logical server a provider is resolving credentials for.
///
/// Providers receive only this — never a host's connection-resolver internals
/// (knowledge/integrations/runtime-mcp.md, security considerations).
pub struct McpAuthRequest<'a> {
    pub server_name: &'a str,
    pub auth_mode: McpServerAuthMode,
    pub oauth_provider_id: Option<&'a str>,
}

/// Resolves (and, in stateful implementations, refreshes) MCP credentials.
#[async_trait]
pub trait McpAuthProvider: Send + Sync {
    async fn authorization(
        &self,
        request: &McpAuthRequest<'_>,
    ) -> anyhow::Result<Option<McpCredential>>;
}

/// Provider that never returns a credential. Used for `auth_mode = None`
/// servers or servers whose auth is carried as literal headers.
#[derive(Debug, Default, Clone)]
pub struct NoAuthProvider;

#[async_trait]
impl McpAuthProvider for NoAuthProvider {
    async fn authorization(
        &self,
        _request: &McpAuthRequest<'_>,
    ) -> anyhow::Result<Option<McpCredential>> {
        Ok(None)
    }
}

/// Static credential provider keyed by logical server name. Suitable for CLI
/// hosts that hold long-lived tokens (e.g. from `.mcp.json` or env vars).
#[derive(Debug, Default, Clone)]
pub struct StaticAuthProvider {
    tokens: HashMap<String, McpCredential>,
}

impl StaticAuthProvider {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_bearer(mut self, server_name: impl Into<String>, token: impl Into<String>) -> Self {
        self.tokens
            .insert(server_name.into(), McpCredential::bearer(token));
        self
    }

    pub fn with_credential(
        mut self,
        server_name: impl Into<String>,
        credential: McpCredential,
    ) -> Self {
        self.tokens.insert(server_name.into(), credential);
        self
    }
}

#[async_trait]
impl McpAuthProvider for StaticAuthProvider {
    async fn authorization(
        &self,
        request: &McpAuthRequest<'_>,
    ) -> anyhow::Result<Option<McpCredential>> {
        Ok(self.tokens.get(request.server_name).cloned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn static_provider_returns_bearer_by_name() {
        let provider = StaticAuthProvider::new().with_bearer("docs", "secret");
        let cred = provider
            .authorization(&McpAuthRequest {
                server_name: "docs",
                auth_mode: McpServerAuthMode::ApiKey,
                oauth_provider_id: None,
            })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cred.authorization.as_deref(), Some("Bearer secret"));
    }

    #[tokio::test]
    async fn static_provider_misses_unknown_server() {
        let provider = StaticAuthProvider::new().with_bearer("docs", "secret");
        let cred = provider
            .authorization(&McpAuthRequest {
                server_name: "other",
                auth_mode: McpServerAuthMode::ApiKey,
                oauth_provider_id: None,
            })
            .await
            .unwrap();
        assert!(cred.is_none());
    }
}
