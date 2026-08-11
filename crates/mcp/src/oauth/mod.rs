//! OAuth for remote MCP servers: the login handshake and the auth provider
//! that keeps its tokens fresh.
//!
//! The protocol steps are [`protocol`] (moved out of `everruns-core` in
//! EVE-879 — MCP login/refresh is their only consumer). What is MCP-specific
//! lives in this module:
//!
//! - **Discovery starts at the server, not at an issuer.** An MCP server
//!   publishes protected-resource metadata (RFC 9728) naming the authorization
//!   servers that can issue tokens for it; when it publishes nothing, the
//!   server's own origin is treated as the issuer. Either way the URL comes
//!   from the remote server, which is why every request goes through the
//!   egress boundary.
//! - **The token is bound to the server as a resource** (RFC 8707), so a token
//!   minted for one MCP server is not replayable against another.
//! - **Login is split in two.** [`prepare_login`] does discovery, registration
//!   and the authorize URL; [`complete_login`] exchanges the code. How the code
//!   comes back — a loopback listener in a CLI, a redirect route in the control
//!   plane — is the host's business, and the only reason those two hosts needed
//!   separate implementations before.
//!
//! Persistence is a [`McpTokenStore`]: the control plane writes encrypted rows,
//! a CLI writes a file. [`OAuthAuthProvider`] reads through it on every request
//! and refreshes inside the store's lock, so a token is renewed once rather
//! than once per concurrent tool call.

use std::collections::HashMap;
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use async_trait::async_trait;
use everruns_core::egress::{EgressRequestKind, EgressService};

pub mod protocol;

pub use protocol::{
    AuthorizationServerMetadata, AuthorizeParams, ClientRegistration, DEFAULT_REFRESH_SKEW_SECONDS,
    OAuthClient, OAuthError, PkcePair, ProtectedResourceMetadata, RegisteredClient, TokenSet,
    authorize_url, random_state, validate_callback_issuer,
};

use crate::auth::{McpAuthProvider, McpAuthRequest, McpCredential};

/// Where a host keeps MCP OAuth tokens.
///
/// Keyed by logical server name, matching [`McpAuthRequest::server_name`], so a
/// host does not have to thread MCP config through its storage layer.
#[async_trait]
pub trait McpTokenStore: Send + Sync {
    /// Load the token set for `server_name`, if one was ever stored.
    async fn load(&self, server_name: &str) -> Result<Option<TokenSet>>;

    /// Persist `tokens` for `server_name`, replacing any previous set.
    ///
    /// Called after login and after every refresh — including refresh-token
    /// rotation, where losing the write means losing the grant.
    async fn save(&self, server_name: &str, tokens: &TokenSet) -> Result<()>;
}

/// In-memory token store. Useful for tests and for a host that is content to
/// re-authorize on restart.
#[derive(Debug, Default)]
pub struct InMemoryTokenStore {
    tokens: Mutex<HashMap<String, TokenSet>>,
}

impl InMemoryTokenStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed a token set, as if a login had produced it.
    pub fn seeded(server_name: impl Into<String>, tokens: TokenSet) -> Self {
        let store = Self::new();
        store
            .tokens
            .lock()
            .unwrap()
            .insert(server_name.into(), tokens);
        store
    }
}

#[async_trait]
impl McpTokenStore for InMemoryTokenStore {
    async fn load(&self, server_name: &str) -> Result<Option<TokenSet>> {
        Ok(self.tokens.lock().unwrap().get(server_name).cloned())
    }

    async fn save(&self, server_name: &str, tokens: &TokenSet) -> Result<()> {
        self.tokens
            .lock()
            .unwrap()
            .insert(server_name.to_string(), tokens.clone());
        Ok(())
    }
}

/// A login in progress: everything needed to finish once the authorization
/// code arrives.
///
/// Hold it across the user-agent round trip. It carries the PKCE verifier and
/// the CSRF `state`, so it must not be logged or persisted unencrypted.
pub struct PreparedLogin {
    /// Send the user-agent here.
    pub authorization_url: String,
    /// Echoed back on the callback; compare before exchanging.
    pub state: String,
    /// Issuer the flow was started against, for RFC 9207 validation.
    pub issuer: String,
    /// Resource indicator the token will be bound to.
    pub resource: String,
    redirect_uri: String,
    pkce: PkcePair,
    client: RegisteredClient,
    metadata: AuthorizationServerMetadata,
}

impl PreparedLogin {
    /// The client this login registered or was configured with. A host that
    /// wants to reuse the registration across logins should persist it.
    pub fn client(&self) -> &RegisteredClient {
        &self.client
    }
}

/// Begin an OAuth login for the MCP server at `server_url`.
///
/// Discovery order: the server's protected-resource metadata names its
/// authorization servers; absent that document, the server origin is the
/// issuer. When `client` is `None` and the authorization server offers dynamic
/// registration, a public client is registered on the spot.
pub async fn prepare_login(
    egress: &dyn EgressService,
    server_url: &str,
    redirect_uri: &str,
    client_name: &str,
    client: Option<RegisteredClient>,
    scope: Option<&str>,
) -> Result<PreparedLogin> {
    let oauth = OAuthClient::new(egress, EgressRequestKind::Mcp);

    let protected = oauth
        .discover_protected_resource(server_url)
        .await
        .context("discovering MCP protected-resource metadata")?;
    let resource = protected
        .as_ref()
        .and_then(|metadata| metadata.resource.clone())
        .unwrap_or_else(|| server_url.to_string());
    // THREAT[TM-MCP-008]: protected-resource metadata is attacker-controlled;
    // bind its audience to the endpoint before involving the authorization server.
    validate_resource(&resource, server_url)?;
    let issuer = protected
        .as_ref()
        .and_then(|metadata| metadata.authorization_servers.first().cloned())
        .unwrap_or_else(|| origin_of(server_url));

    let metadata = oauth
        .discover_authorization_server(&issuer)
        .await
        .context("discovering MCP authorization-server metadata")?;

    let client = match client {
        Some(client) => client,
        None => {
            let endpoint = metadata.registration_endpoint.clone().ok_or_else(|| {
                anyhow!(
                    "MCP server at {server_url} needs a client_id: its authorization server \
                     offers no dynamic registration endpoint"
                )
            })?;
            oauth
                .register_client(
                    &endpoint,
                    &ClientRegistration {
                        client_name: client_name.to_string(),
                        redirect_uris: vec![redirect_uri.to_string()],
                        scope: scope.map(str::to_string),
                    },
                )
                .await
                .context("registering an OAuth client with the MCP authorization server")?
        }
    };

    let pkce = PkcePair::generate();
    let state = random_state();
    let authorization_url = authorize_url(
        &metadata,
        &AuthorizeParams {
            client_id: &client.client_id,
            redirect_uri,
            code_challenge: &pkce.challenge,
            state: &state,
            scope,
            resource: Some(&resource),
        },
    )?;

    Ok(PreparedLogin {
        authorization_url,
        state,
        issuer: metadata.issuer.clone(),
        resource,
        redirect_uri: redirect_uri.to_string(),
        pkce,
        client,
        metadata,
    })
}

/// Finish a login with the authorization code from the callback.
///
/// `returned_state` and `returned_issuer` are the callback's `state` and `iss`.
/// A mismatched `state` is rejected here rather than by the caller, so no host
/// can forget the CSRF check.
pub async fn complete_login(
    egress: &dyn EgressService,
    prepared: &PreparedLogin,
    code: &str,
    returned_state: &str,
    returned_issuer: Option<&str>,
) -> Result<TokenSet> {
    if returned_state != prepared.state {
        return Err(anyhow!("authorization response state does not match"));
    }
    validate_callback_issuer(&prepared.issuer, returned_issuer)?;

    let oauth = OAuthClient::new(egress, EgressRequestKind::Mcp);
    oauth
        .exchange_code(
            &prepared.metadata.token_endpoint,
            &prepared.client,
            code,
            &prepared.pkce.verifier,
            &prepared.redirect_uri,
            Some(&prepared.resource),
        )
        .await
        .context("exchanging the MCP authorization code")
}

/// [`McpAuthProvider`] backed by stored OAuth tokens, refreshing as needed.
pub struct OAuthAuthProvider<S: McpTokenStore> {
    store: S,
    egress: std::sync::Arc<dyn EgressService>,
    refresh_skew_seconds: i64,
    /// Serializes refreshes so concurrent tool calls against one server do not
    /// each burn a refresh — and, with a rotating server, invalidate each
    /// other's freshly issued refresh token.
    refresh_lock: tokio::sync::Mutex<()>,
}

impl<S: McpTokenStore> OAuthAuthProvider<S> {
    /// Build a provider over `store`, sending refreshes through `egress`.
    pub fn new(store: S, egress: std::sync::Arc<dyn EgressService>) -> Self {
        Self {
            store,
            egress,
            refresh_skew_seconds: DEFAULT_REFRESH_SKEW_SECONDS,
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Override how far ahead of expiry a token is renewed.
    pub fn refresh_skew_seconds(mut self, seconds: i64) -> Self {
        self.refresh_skew_seconds = seconds;
        self
    }
}

#[async_trait]
impl<S: McpTokenStore> McpAuthProvider for OAuthAuthProvider<S> {
    async fn authorization(&self, request: &McpAuthRequest<'_>) -> Result<Option<McpCredential>> {
        let Some(tokens) = self.store.load(request.server_name).await? else {
            return Ok(None);
        };
        if !tokens.needs_refresh(chrono::Utc::now(), self.refresh_skew_seconds) {
            return Ok(Some(McpCredential::authorization(
                tokens.authorization_value(),
            )));
        }

        let _guard = self.refresh_lock.lock().await;
        // Re-read under the lock: another caller may have refreshed while we
        // waited, in which case rotation makes our copy's refresh token dead.
        let current = self
            .store
            .load(request.server_name)
            .await?
            .unwrap_or(tokens);
        if !current.needs_refresh(chrono::Utc::now(), self.refresh_skew_seconds) {
            return Ok(Some(McpCredential::authorization(
                current.authorization_value(),
            )));
        }

        if current.refresh_token.is_none() {
            // Nothing to renew with. Hand back the token we have: an expired
            // one produces a 401 the host can turn into a re-login prompt,
            // which beats reporting "no credential" for a configured server.
            return Ok(Some(McpCredential::authorization(
                current.authorization_value(),
            )));
        }

        let oauth = OAuthClient::new(self.egress.as_ref(), EgressRequestKind::Mcp);
        let renewed = oauth
            .refresh(&current)
            .await
            .with_context(|| format!("refreshing MCP OAuth token for {}", request.server_name))?;
        self.store.save(request.server_name, &renewed).await?;
        Ok(Some(McpCredential::authorization(
            renewed.authorization_value(),
        )))
    }
}

/// Scheme + host + port of `url`, used as the fallback issuer.
fn origin_of(url: &str) -> String {
    match url::Url::parse(url) {
        Ok(parsed) => format!(
            "{}://{}{}",
            parsed.scheme(),
            parsed.host_str().unwrap_or_default(),
            parsed
                .port()
                .map(|port| format!(":{port}"))
                .unwrap_or_default()
        ),
        Err(_) => url.to_string(),
    }
}

/// Validate that an authorization server cannot mint a token for a resource
/// controlled by a different origin than the configured MCP endpoint.
fn validate_resource(resource: &str, server_url: &str) -> Result<()> {
    let resource = url::Url::parse(resource).context("MCP OAuth resource is not a valid URL")?;
    let server = url::Url::parse(server_url).context("MCP server URL is not a valid URL")?;

    if resource.scheme() != "https" {
        return Err(anyhow!("MCP OAuth resource must use HTTPS"));
    }
    if resource.origin() != server.origin() {
        return Err(anyhow!(
            "MCP OAuth resource origin does not match the configured server"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use everruns_core::McpServerAuthMode;
    use everruns_core::egress::{
        EgressError, EgressRequest, EgressResponse, EgressResult, EgressStreamResponse,
    };
    use std::collections::BTreeMap;
    use std::sync::Arc;

    #[derive(Default)]
    struct FakeEgress {
        routes: Vec<(String, u16, String)>,
        sent: Mutex<Vec<EgressRequest>>,
    }

    impl FakeEgress {
        fn with(routes: Vec<(&str, u16, &str)>) -> Arc<Self> {
            Arc::new(Self {
                routes: routes
                    .into_iter()
                    .map(|(url, status, body)| (url.to_string(), status, body.to_string()))
                    .collect(),
                sent: Mutex::new(Vec::new()),
            })
        }
    }

    #[async_trait]
    impl EgressService for FakeEgress {
        async fn send(&self, request: EgressRequest) -> EgressResult<EgressResponse> {
            self.sent.lock().unwrap().push(request.clone());
            match self.routes.iter().find(|(url, _, _)| *url == request.url) {
                Some((_, status, body)) => Ok(EgressResponse {
                    status: *status,
                    headers: BTreeMap::new(),
                    body: body.clone().into_bytes(),
                }),
                None => Ok(EgressResponse {
                    status: 404,
                    headers: BTreeMap::new(),
                    body: Vec::new(),
                }),
            }
        }

        async fn send_stream(&self, _request: EgressRequest) -> EgressResult<EgressStreamResponse> {
            Err(EgressError::Transport("unsupported".to_string()))
        }
    }

    fn auth_request<'a>(server_name: &'a str) -> McpAuthRequest<'a> {
        McpAuthRequest {
            server_name,
            auth_mode: McpServerAuthMode::OAuth,
            oauth_provider_id: None,
        }
    }

    fn token_set(expires_in_seconds: i64, refresh_token: Option<&str>) -> TokenSet {
        TokenSet {
            access_token: "at-1".to_string(),
            refresh_token: refresh_token.map(str::to_string),
            token_type: "Bearer".to_string(),
            expires_at: Some(Utc::now() + Duration::seconds(expires_in_seconds)),
            scope: None,
            token_endpoint: Some("https://auth.example.com/token".to_string()),
            client_id: Some("client-1".to_string()),
            client_secret: None,
        }
    }

    const PROTECTED_RESOURCE: &str = r#"{
        "resource": "https://mcp.example.com/",
        "authorization_servers": ["https://auth.example.com"]
    }"#;

    const AS_METADATA: &str = r#"{
        "issuer": "https://auth.example.com",
        "authorization_endpoint": "https://auth.example.com/authorize",
        "token_endpoint": "https://auth.example.com/token",
        "registration_endpoint": "https://auth.example.com/register"
    }"#;

    #[tokio::test]
    async fn login_discovers_registers_and_binds_the_token_to_the_resource() {
        let egress = FakeEgress::with(vec![
            (
                "https://mcp.example.com/.well-known/oauth-protected-resource",
                200,
                PROTECTED_RESOURCE,
            ),
            (
                "https://auth.example.com/.well-known/oauth-authorization-server",
                200,
                AS_METADATA,
            ),
            (
                "https://auth.example.com/register",
                200,
                r#"{"client_id":"dyn-client-1"}"#,
            ),
            (
                "https://auth.example.com/token",
                200,
                r#"{"access_token":"at-1","refresh_token":"rt-1","expires_in":3600}"#,
            ),
        ]);

        let prepared = prepare_login(
            egress.as_ref(),
            "https://mcp.example.com/",
            "http://127.0.0.1:1455/callback",
            "everruns",
            None,
            Some("mcp:read"),
        )
        .await
        .expect("login preparation");

        assert_eq!(prepared.client().client_id, "dyn-client-1");
        assert_eq!(prepared.resource, "https://mcp.example.com/");
        assert!(
            prepared
                .authorization_url
                .contains("resource=https%3A%2F%2Fmcp.example.com%2F"),
            "the token must be bound to this server: {}",
            prepared.authorization_url
        );

        let state = prepared.state.clone();
        let tokens = complete_login(
            egress.as_ref(),
            &prepared,
            "code-1",
            &state,
            Some("https://auth.example.com"),
        )
        .await
        .expect("code exchange");

        assert_eq!(tokens.access_token, "at-1");
        assert_eq!(tokens.refresh_token.as_deref(), Some("rt-1"));
    }

    #[tokio::test]
    async fn login_rejects_a_resource_on_another_origin() {
        let egress = FakeEgress::with(vec![(
            "https://evil-mcp.example/.well-known/oauth-protected-resource/sse",
            200,
            r#"{
                "resource": "https://victim-mcp.example/",
                "authorization_servers": ["https://auth.victim.example"]
            }"#,
        )]);

        let error = prepare_login(
            egress.as_ref(),
            "https://evil-mcp.example/sse",
            "http://127.0.0.1:1455/callback",
            "everruns",
            None,
            None,
        )
        .await
        .err()
        .expect("a server must not select another origin as its token audience");

        assert!(error.to_string().contains("resource origin"));
        assert_eq!(egress.sent.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn login_rejects_an_insecure_resource() {
        let egress = FakeEgress::with(vec![(
            "https://mcp.example.com/.well-known/oauth-protected-resource/sse",
            200,
            r#"{
                "resource": "http://mcp.example.com/",
                "authorization_servers": ["https://auth.example.com"]
            }"#,
        )]);

        let error = prepare_login(
            egress.as_ref(),
            "https://mcp.example.com/sse",
            "http://127.0.0.1:1455/callback",
            "everruns",
            None,
            None,
        )
        .await
        .err()
        .expect("OAuth resources must use a secure URL");

        assert!(error.to_string().contains("HTTPS"));
        assert_eq!(egress.sent.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn login_falls_back_to_the_server_origin_as_issuer() {
        // Servers that publish no protected-resource document are the common
        // case today; treat the origin as the issuer rather than failing.
        let egress = FakeEgress::with(vec![(
            "https://mcp.example.com/.well-known/oauth-authorization-server",
            200,
            r#"{
                "issuer": "https://mcp.example.com",
                "authorization_endpoint": "https://mcp.example.com/authorize",
                "token_endpoint": "https://mcp.example.com/token"
            }"#,
        )]);

        let prepared = prepare_login(
            egress.as_ref(),
            "https://mcp.example.com/sse",
            "http://127.0.0.1:1455/callback",
            "everruns",
            Some(RegisteredClient {
                client_id: "configured".to_string(),
                client_secret: None,
            }),
            None,
        )
        .await
        .expect("login preparation");

        assert_eq!(prepared.issuer, "https://mcp.example.com");
        assert_eq!(prepared.resource, "https://mcp.example.com/sse");
    }

    #[tokio::test]
    async fn a_mismatched_callback_state_is_refused() {
        let egress = FakeEgress::with(vec![(
            "https://mcp.example.com/.well-known/oauth-authorization-server",
            200,
            AS_METADATA,
        )]);
        let prepared = prepare_login(
            egress.as_ref(),
            "https://mcp.example.com/",
            "http://127.0.0.1:1455/callback",
            "everruns",
            Some(RegisteredClient {
                client_id: "configured".to_string(),
                client_secret: None,
            }),
            None,
        )
        .await
        .expect("login preparation");

        let error = complete_login(egress.as_ref(), &prepared, "code-1", "not-the-state", None)
            .await
            .expect_err("CSRF check must fail");
        assert!(error.to_string().contains("state"));
    }

    #[tokio::test]
    async fn a_mismatched_callback_issuer_is_refused() {
        let egress = FakeEgress::with(vec![(
            "https://mcp.example.com/.well-known/oauth-authorization-server",
            200,
            AS_METADATA,
        )]);
        let prepared = prepare_login(
            egress.as_ref(),
            "https://mcp.example.com/",
            "http://127.0.0.1:1455/callback",
            "everruns",
            Some(RegisteredClient {
                client_id: "configured".to_string(),
                client_secret: None,
            }),
            None,
        )
        .await
        .expect("login preparation");

        let state = prepared.state.clone();
        let error = complete_login(
            egress.as_ref(),
            &prepared,
            "code-1",
            &state,
            Some("https://evil.example.com"),
        )
        .await
        .expect_err("mix-up defense must fail");
        assert!(error.to_string().contains("issuer"));
    }

    #[tokio::test]
    async fn a_live_token_is_returned_without_touching_the_network() {
        let egress = FakeEgress::with(vec![]);
        let provider = OAuthAuthProvider::new(
            InMemoryTokenStore::seeded("docs", token_set(3600, Some("rt-1"))),
            egress.clone(),
        );

        let credential = provider
            .authorization(&auth_request("docs"))
            .await
            .expect("resolve")
            .expect("credential");

        assert_eq!(credential.authorization.as_deref(), Some("Bearer at-1"));
        assert!(egress.sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_near_expiry_token_is_refreshed_and_persisted() {
        let egress = FakeEgress::with(vec![(
            "https://auth.example.com/token",
            200,
            r#"{"access_token":"at-2","refresh_token":"rt-2","expires_in":3600}"#,
        )]);
        let store = InMemoryTokenStore::seeded("docs", token_set(5, Some("rt-1")));
        let provider = OAuthAuthProvider::new(store, egress.clone());

        let credential = provider
            .authorization(&auth_request("docs"))
            .await
            .expect("resolve")
            .expect("credential");

        assert_eq!(credential.authorization.as_deref(), Some("Bearer at-2"));
        // Rotation must be persisted, or the next refresh presents a dead token.
        let stored = provider.store.load("docs").await.unwrap().unwrap();
        assert_eq!(stored.refresh_token.as_deref(), Some("rt-2"));
    }

    #[tokio::test]
    async fn an_unknown_server_resolves_to_no_credential() {
        let egress = FakeEgress::with(vec![]);
        let provider = OAuthAuthProvider::new(InMemoryTokenStore::new(), egress);

        assert!(
            provider
                .authorization(&auth_request("never-logged-in"))
                .await
                .expect("resolve")
                .is_none()
        );
    }

    #[tokio::test]
    async fn an_expired_token_with_no_refresh_token_is_still_presented() {
        // Presenting it yields a 401 the host can turn into a re-login prompt;
        // returning None would look like "this server needs no auth".
        let egress = FakeEgress::with(vec![]);
        let provider = OAuthAuthProvider::new(
            InMemoryTokenStore::seeded("docs", token_set(-60, None)),
            egress.clone(),
        );

        let credential = provider
            .authorization(&auth_request("docs"))
            .await
            .expect("resolve")
            .expect("credential");

        assert_eq!(credential.authorization.as_deref(), Some("Bearer at-1"));
        assert!(egress.sent.lock().unwrap().is_empty());
    }
}
