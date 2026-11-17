//! Application-owned MCP authentication and OAuth login. Stability: alpha.

use async_trait::async_trait;
use everruns_core::mcp::oauth::{self, OAuthAuthProvider};
use everruns_core::mcp::{McpAuthProvider, McpAuthRequest, McpCredential};
use std::{collections::BTreeMap, fmt, sync::Arc};

/// The application contract for loading and atomically saving rotating grants.
pub use everruns_core::mcp::oauth::McpTokenStore as TokenStore;
/// Registration for OAuth servers that do not support dynamic client registration.
pub use everruns_core::mcp::oauth::RegisteredClient as ClientRegistration;
/// An OAuth grant. Serialize only into an application-owned secret store.
pub use everruns_core::mcp::oauth::TokenSet as OAuthTokens;

/// MCP authentication failed without exposing credentials or upstream bodies.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("MCP authentication failed: {0}")]
pub struct AuthError(&'static str);

/// A cloneable, process-local token store. Applications supply durable secret storage.
#[derive(Clone, Default)]
pub struct MemoryTokenStore(Arc<oauth::InMemoryTokenStore>);

impl fmt::Debug for MemoryTokenStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("MemoryTokenStore { .. }")
    }
}

#[async_trait]
impl TokenStore for MemoryTokenStore {
    async fn load(&self, name: &str) -> anyhow::Result<Option<OAuthTokens>> {
        self.0.load(name).await
    }
    async fn save(&self, name: &str, tokens: &OAuthTokens) -> anyhow::Result<()> {
        self.0.save(name, tokens).await
    }
}

/// Credentials selected explicitly by the application, acquired on each MCP operation.
#[async_trait]
pub trait Credentials: Send + Sync {
    /// Full Authorization value (for example `Bearer ...`), or no credential.
    async fn authorization(&self) -> Result<Option<String>, String>;
}

struct ApplicationCredentials(Arc<dyn Credentials>);
#[async_trait]
impl McpAuthProvider for ApplicationCredentials {
    async fn authorization(&self, _: &McpAuthRequest<'_>) -> anyhow::Result<Option<McpCredential>> {
        self.0
            .authorization()
            .await
            .map(|value| value.map(McpCredential::authorization))
            .map_err(|_| anyhow::anyhow!("MCP credentials unavailable"))
    }
}

struct Bearer(String);
#[async_trait]
impl Credentials for Bearer {
    async fn authorization(&self) -> Result<Option<String>, String> {
        if self.0.trim().is_empty() {
            return Err("empty credential".into());
        }
        Ok(Some(format!("Bearer {}", self.0)))
    }
}

#[derive(Clone)]
struct Entry {
    url: String,
    provider: Arc<dyn McpAuthProvider>,
}

/// Named MCP credentials bound to the exact endpoint the application configured.
///
/// Credentials never enter an agent's serializable MCP configuration. Reusing a
/// name with another endpoint is rejected when the agent is built.
#[derive(Clone, Default)]
pub struct McpAuth {
    entries: BTreeMap<String, Entry>,
}

impl fmt::Debug for McpAuth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("McpAuth")
            .field("servers", &self.entries.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

impl McpAuth {
    /// Start an empty credential configuration.
    pub fn new() -> Self {
        Self::default()
    }
    /// Bind a bearer token to a configured server name and endpoint.
    pub fn bearer(
        self,
        name: impl Into<String>,
        url: impl Into<String>,
        token: impl Into<String>,
    ) -> Self {
        self.credentials(name, url, Bearer(token.into()))
    }
    /// Bind application-owned credential acquisition to one server.
    pub fn credentials(
        mut self,
        name: impl Into<String>,
        url: impl Into<String>,
        credentials: impl Credentials + 'static,
    ) -> Self {
        self.entries.insert(
            name.into(),
            Entry {
                url: url.into(),
                provider: Arc::new(ApplicationCredentials(Arc::new(credentials))),
            },
        );
        self
    }
    /// Bind an OAuth token store. Refresh and rotation use the shared MCP implementation.
    pub fn oauth(
        mut self,
        name: impl Into<String>,
        url: impl Into<String>,
        store: impl TokenStore + 'static,
    ) -> Self {
        let provider = OAuthAuthProvider::new(store, crate::batteries::runtime_egress_service());
        self.entries.insert(
            name.into(),
            Entry {
                url: url.into(),
                provider: Arc::new(provider),
            },
        );
        self
    }
    pub(crate) fn validate(&self, servers: &everruns_core::ScopedMcpServers) -> Result<(), String> {
        for (name, entry) in &self.entries {
            let Some(server) = servers.get(name) else {
                return Err(format!(
                    "MCP authentication names an unconfigured server: {name}"
                ));
            };
            if server.transport_type != everruns_core::McpServerTransportType::Http
                || server.url != entry.url
            {
                return Err(format!(
                    "MCP authentication endpoint does not match server: {name}"
                ));
            }
        }
        Ok(())
    }
}

#[async_trait]
impl McpAuthProvider for McpAuth {
    async fn authorization(
        &self,
        request: &McpAuthRequest<'_>,
    ) -> anyhow::Result<Option<McpCredential>> {
        // THREAT[TM-FW-001]: revalidate live attachments before resolving any grant.
        match self.entries.get(request.server_name) {
            Some(entry) if request.server_url == Some(entry.url.as_str()) => entry
                .provider
                .authorization(request)
                .await
                .map_err(|_| anyhow::anyhow!("MCP authorization unavailable")),
            Some(_) => Err(anyhow::anyhow!("MCP credential endpoint mismatch")),
            None => Ok(None),
        }
    }
}

/// A split OAuth login: the application owns browser navigation and its callback.
pub struct OAuthLogin {
    prepared: oauth::PreparedLogin,
}

impl fmt::Debug for OAuthLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("OAuthLogin { .. }")
    }
}

impl OAuthLogin {
    /// Discover the MCP server, register a public client, and prepare a PKCE login.
    pub async fn begin(
        server_url: &str,
        redirect_uri: &str,
        client_name: &str,
    ) -> Result<Self, AuthError> {
        Self::begin_with_client(server_url, redirect_uri, client_name, None, None).await
    }
    /// Prepare login with an existing client registration and optional scope.
    pub async fn begin_with_client(
        server_url: &str,
        redirect_uri: &str,
        client_name: &str,
        client: Option<ClientRegistration>,
        scope: Option<&str>,
    ) -> Result<Self, AuthError> {
        oauth::prepare_login(
            crate::batteries::runtime_egress_service().as_ref(),
            server_url,
            redirect_uri,
            client_name,
            client,
            scope,
        )
        .await
        .map(|prepared| Self { prepared })
        .map_err(|_| AuthError("could not prepare OAuth login"))
    }
    /// Open this URL in the application's user agent. It is not a diagnostic value.
    pub fn authorization_url(&self) -> &str {
        &self.prepared.authorization_url
    }
    /// Complete the callback. State, issuer, PKCE and resource binding are checked.
    pub async fn complete(
        self,
        code: &str,
        state: &str,
        issuer: Option<&str>,
    ) -> Result<OAuthTokens, AuthError> {
        oauth::complete_login(
            crate::batteries::runtime_egress_service().as_ref(),
            &self.prepared,
            code,
            state,
            issuer,
        )
        .await
        .map_err(|_| AuthError("OAuth callback rejected or exchange failed"))
    }
}

/// OAuth callback values. Debug deliberately omits the one-time code and state.
pub struct Callback {
    /// Authorization code; pass directly to OAuthLogin::complete.
    pub code: String,
    /// State to compare with the prepared login.
    pub state: String,
    /// Issuer returned by the authorization server.
    pub issuer: Option<String>,
}
impl fmt::Debug for Callback {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Callback { .. }")
    }
}
/// Parse a callback query without leaking provider error descriptions or credentials.
/// Duplicate parameters, OAuth errors, and missing code/state are rejected.
pub fn parse_callback_query(query: &str) -> Result<Callback, AuthError> {
    let mut params = BTreeMap::new();
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if params
            .insert(key.into_owned(), value.into_owned())
            .is_some()
        {
            return Err(AuthError("duplicate callback parameter"));
        }
    }
    if params.contains_key("error") {
        return Err(AuthError("authorization was rejected"));
    }
    let code = params
        .remove("code")
        .filter(|v| !v.is_empty())
        .ok_or(AuthError("callback code missing"))?;
    let state = params
        .remove("state")
        .filter(|v| !v.is_empty())
        .ok_or(AuthError("callback state missing"))?;
    Ok(Callback {
        code,
        state,
        issuer: params.remove("iss"),
    })
}

/// Result type for application token-store implementations.
pub type StoreResult<T> = anyhow::Result<T>;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct Rotating(Arc<AtomicUsize>);
    #[async_trait]
    impl Credentials for Rotating {
        async fn authorization(&self) -> Result<Option<String>, String> {
            let n = self.0.fetch_add(1, Ordering::SeqCst);
            Ok(Some(format!("Bearer rotating-{n}")))
        }
    }
    fn request<'a>(url: &'a str) -> McpAuthRequest<'a> {
        McpAuthRequest {
            server_name: "docs",
            server_url: Some(url),
            auth_mode: everruns_core::McpServerAuthMode::ApiKey,
            oauth_provider_id: None,
            session_id: None,
        }
    }
    #[tokio::test]
    async fn endpoint_is_checked_on_every_operation_before_acquiring_credentials() {
        let calls = Arc::new(AtomicUsize::new(0));
        let auth =
            McpAuth::new().credentials("docs", "https://docs.example/mcp", Rotating(calls.clone()));
        assert!(
            McpAuthProvider::authorization(&auth, &request("https://other.example/mcp"))
                .await
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        for n in 0..2 {
            let credential =
                McpAuthProvider::authorization(&auth, &request("https://docs.example/mcp"))
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(
                credential.authorization.as_deref(),
                Some(format!("Bearer rotating-{n}").as_str())
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
    #[test]
    fn callback_parsing_rejects_duplicates_errors_and_missing_fields_without_echoing_secrets() {
        assert!(parse_callback_query("code=sentinel&state=s&state=other").is_err());
        assert!(parse_callback_query("error=denied&error_description=sentinel").is_err());
        assert!(parse_callback_query("code=sentinel").is_err());
        let callback =
            parse_callback_query("code=a%2Bb&state=s&iss=https%3A%2F%2Fissuer.example").unwrap();
        assert_eq!(callback.code, "a+b");
        assert_eq!(callback.issuer.as_deref(), Some("https://issuer.example"));
        assert!(!format!("{callback:?}").contains("a+b"));
    }
}
