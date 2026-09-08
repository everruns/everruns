// Authentication for Microsoft MAI (Azure AI Foundry) providers.
//
// MAI deployments on Azure AI Foundry accept two auth schemes:
//
//   1. An Azure AI Foundry **API key**, sent via the `api-key` header.
//   2. A **Microsoft Entra ID** (OAuth 2.0) bearer token, sent via
//      `Authorization: Bearer <token>`. Tokens are minted with the
//      client-credentials grant and are short-lived, so they are cached and
//      refreshed before expiry.
//
// Both schemes implement the runtime provider's generic [`ProviderAuth`]
// contract, so the protocol driver never has to know
// which scheme is in use. New schemes (managed identity, workload identity
// federation, ...) can be added by implementing [`ProviderAuth`] without
// touching the driver.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use everruns_provider::driver_registry::DriverConfig;
use everruns_provider::error::{AgentLoopError, Result};
use everruns_provider::validate_safe_url;
use everruns_provider::{ProviderAuth, ProviderAuthRequest};
use serde::Deserialize;
use tokio::sync::Mutex;

/// Default Microsoft Entra ID authority host.
pub const DEFAULT_ENTRA_AUTHORITY: &str = "https://login.microsoftonline.com";

/// Default OAuth scope for Azure AI Foundry / Azure Cognitive Services.
pub const DEFAULT_ENTRA_SCOPE: &str = "https://cognitiveservices.azure.com/.default";

/// Refresh a cached Entra token this long before it actually expires, so an
/// in-flight request never races the expiry boundary.
const TOKEN_REFRESH_SKEW: Duration = Duration::seconds(120);

const ALLOWED_ENTRA_AUTHORITY_HOSTS: &[&str] = &[
    // Public, US Government, and China cloud Microsoft Entra authorities.
    "login.microsoftonline.com",
    "login.microsoftonline.us",
    "login.chinacloudapi.cn",
];

/// Authentication strategy for a Microsoft MAI provider.
///
/// Built from a [`DriverConfig`] via [`MaiAuth::from_driver_config`]: an Entra
/// OAuth config in `metadata.extra` selects OAuth; otherwise an `api_key`
/// selects API-key auth.
///
/// `Debug` redacts the key / client secret so credentials never leak via
/// `{:?}` formatting (logs, error chains).
#[derive(Clone)]
pub enum MaiAuth {
    /// Azure AI Foundry API key (`api-key` header).
    ApiKey(String),
    /// Microsoft Entra ID OAuth client-credentials flow (bearer token).
    EntraOAuth(EntraOAuthConfig),
}

impl std::fmt::Debug for MaiAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MaiAuth::ApiKey(_) => f.debug_tuple("ApiKey").field(&"[REDACTED]").finish(),
            MaiAuth::EntraOAuth(config) => f.debug_tuple("EntraOAuth").field(config).finish(),
        }
    }
}

impl MaiAuth {
    /// Resolve the auth strategy from a driver config.
    ///
    /// Precedence:
    /// 1. An Entra OAuth block in `metadata.extra` (the in-process / embedder
    ///    path, where credentials arrive as [`ProviderMetadata`]).
    /// 2. The driver's typed credential fields. Entra ID OAuth is declared as
    ///    discrete fields (`tenant_id`, `client_id`, `client_secret`, optional
    ///    `scope`/`authority`); a plain Azure AI Foundry key is the `api_key`
    ///    field. Server-stored providers carry these in the encrypted
    ///    credential document, which is parsed into typed fields once in
    ///    [`DriverConfig`], so OAuth flows through every existing path (chat
    ///    execution and model sync) and keeps the fail-closed contract.
    ///
    /// Returns an error when no credential is present so misconfiguration fails
    /// with a clear message rather than an opaque 401 at call time.
    pub fn from_driver_config(config: &DriverConfig) -> Result<Self> {
        if let Some(extra) = config.metadata.extra.as_ref()
            && let Some(oauth) = EntraOAuthConfig::from_extra(extra)?
        {
            return Ok(MaiAuth::EntraOAuth(oauth));
        }

        let oauth_fields = oauth_value_from_credentials(&config.credentials);
        if let Some(oauth) = EntraOAuthConfig::from_extra(&oauth_fields)? {
            return Ok(MaiAuth::EntraOAuth(oauth));
        }

        match config.credential("api_key") {
            Some(key) => Ok(MaiAuth::ApiKey(key.to_string())),
            None => Err(AgentLoopError::llm(
                "Microsoft MAI provider is not authenticated: configure an Azure AI \
                 Foundry API key, or Entra ID OAuth credentials (tenant_id, client_id, \
                 client_secret).",
            )),
        }
    }

    /// Build the refreshable provider-auth implementation for this strategy.
    pub fn into_provider(self) -> Arc<dyn ProviderAuth> {
        match self {
            MaiAuth::ApiKey(key) => Arc::new(ApiKeyAuth { key }),
            MaiAuth::EntraOAuth(config) => Arc::new(EntraOAuthProvider::new(config)),
        }
    }
}

/// Preserve a configuration failure inside the provider-owned auth layer.
///
/// Transitional descriptor factories cannot return a construction error, so
/// malformed credentials become an auth implementation that fails closed
/// before the protocol sends a request.
pub(crate) fn failing_provider(error: AgentLoopError) -> Arc<dyn ProviderAuth> {
    Arc::new(FailingAuth {
        message: error.to_string(),
    })
}

struct FailingAuth {
    message: String,
}

impl std::fmt::Debug for FailingAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FailingAuth").finish_non_exhaustive()
    }
}

#[async_trait]
impl ProviderAuth for FailingAuth {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        Err(AgentLoopError::llm(self.message.clone()))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Static Azure AI Foundry API-key auth: always emits the `api-key` header.
struct ApiKeyAuth {
    key: String,
}

impl std::fmt::Debug for ApiKeyAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ApiKeyAuth")
            .field("key", &"[REDACTED]")
            .finish()
    }
}

#[async_trait]
impl ProviderAuth for ApiKeyAuth {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        Ok(vec![("api-key".to_string(), self.key.clone())])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Collect the Entra OAuth credential fields into a JSON object so they can be
/// detected and validated by [`EntraOAuthConfig::from_extra`], the same code
/// path used for the embedder (`metadata.extra`) case. Empty fields are
/// omitted so unset optional values fall back to their defaults.
fn oauth_value_from_credentials(
    credentials: &std::collections::BTreeMap<String, String>,
) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    for key in [
        "tenant_id",
        "client_id",
        "client_secret",
        "scope",
        "authority",
    ] {
        if let Some(value) = credentials.get(key).filter(|v| !v.is_empty()) {
            obj.insert(key.to_string(), serde_json::Value::String(value.clone()));
        }
    }
    serde_json::Value::Object(obj)
}

/// Microsoft Entra ID client-credentials configuration.
///
/// `Debug` redacts `client_secret` so it never leaks via `{:?}` formatting.
#[derive(Clone, Deserialize)]
pub struct EntraOAuthConfig {
    /// Entra ID (Azure AD) tenant id.
    pub tenant_id: String,
    /// Application (client) id of the service principal.
    pub client_id: String,
    /// Client secret of the service principal.
    pub client_secret: String,
    /// OAuth scope. Defaults to [`DEFAULT_ENTRA_SCOPE`].
    #[serde(default = "default_scope")]
    pub scope: String,
    /// Authority host. Defaults to [`DEFAULT_ENTRA_AUTHORITY`].
    #[serde(default = "default_authority")]
    pub authority: String,
}

impl std::fmt::Debug for EntraOAuthConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntraOAuthConfig")
            .field("tenant_id", &self.tenant_id)
            .field("client_id", &self.client_id)
            .field("client_secret", &"[REDACTED]")
            .field("scope", &self.scope)
            .field("authority", &self.authority)
            .finish()
    }
}

fn default_scope() -> String {
    DEFAULT_ENTRA_SCOPE.to_string()
}

fn default_authority() -> String {
    DEFAULT_ENTRA_AUTHORITY.to_string()
}

impl EntraOAuthConfig {
    /// Parse an Entra OAuth block from provider `metadata.extra`.
    ///
    /// Returns `Ok(None)` when the JSON is clearly not an Entra config (no
    /// OAuth fields present), `Ok(Some(_))` when a full config is present, and
    /// `Err` when it looks like an Entra config but is missing required fields.
    fn from_extra(extra: &serde_json::Value) -> Result<Option<Self>> {
        let Some(obj) = extra.as_object() else {
            return Ok(None);
        };

        let tagged_entra = obj
            .get("auth")
            .and_then(|v| v.as_str())
            .is_some_and(|s| matches!(s, "entra" | "entra_id" | "oauth" | "entra_oauth"));
        let has_oauth_field = obj.contains_key("tenant_id")
            || obj.contains_key("client_id")
            || obj.contains_key("client_secret");

        if !tagged_entra && !has_oauth_field {
            return Ok(None);
        }

        let config = serde_json::from_value::<EntraOAuthConfig>(extra.clone()).map_err(|e| {
            AgentLoopError::llm(format!(
                "Invalid Microsoft Entra ID OAuth config in provider metadata: {e}. \
                 Required fields: tenant_id, client_id, client_secret."
            ))
        })?;
        config.validate_authority()?;
        Ok(Some(config))
    }

    fn validate_authority(&self) -> Result<()> {
        let url = validate_safe_url(&self.authority).map_err(|e| {
            AgentLoopError::llm(format!(
                "Invalid Microsoft Entra ID OAuth authority URL: {e}"
            ))
        })?;

        if url.scheme() != "https" {
            return Err(AgentLoopError::llm(
                "Invalid Microsoft Entra ID OAuth authority URL: authority must use https",
            ));
        }

        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if !ALLOWED_ENTRA_AUTHORITY_HOSTS.contains(&host.as_str()) {
            return Err(AgentLoopError::llm(format!(
                "Invalid Microsoft Entra ID OAuth authority URL: unsupported authority host {host}"
            )));
        }

        if url.port().is_some()
            || url.path() != "/"
            || url.query().is_some()
            || url.fragment().is_some()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(AgentLoopError::llm(
                "Invalid Microsoft Entra ID OAuth authority URL: authority must be an https origin without port, path, query, fragment, or credentials",
            ));
        }

        Ok(())
    }

    /// The token endpoint URL for this tenant.
    fn token_url(&self) -> String {
        format!(
            "{}/{}/oauth2/v2.0/token",
            self.authority.trim_end_matches('/'),
            self.tenant_id
        )
    }
}

/// A cached Entra bearer token with its expiry instant.
#[derive(Clone)]
struct CachedToken {
    token: String,
    expires_at: DateTime<Utc>,
}

impl CachedToken {
    /// Whether the token is still usable accounting for the refresh skew.
    fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now + TOKEN_REFRESH_SKEW < self.expires_at
    }
}

/// Token response from the Entra ID `/oauth2/v2.0/token` endpoint.
#[derive(Deserialize)]
struct EntraTokenResponse {
    access_token: String,
    /// Token lifetime in seconds.
    expires_in: i64,
}

/// [`ProviderAuth`] implementation that mints and caches Entra ID bearer tokens via the
/// client-credentials grant.
pub struct EntraOAuthProvider {
    config: EntraOAuthConfig,
    http: reqwest::Client,
    cache: Mutex<Option<CachedToken>>,
}

impl EntraOAuthProvider {
    /// Create a new provider with a fresh HTTP client.
    pub fn new(config: EntraOAuthConfig) -> Self {
        Self {
            config,
            http: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("MAI OAuth HTTP client configuration is valid"),
            cache: Mutex::new(None),
        }
    }

    /// Return a valid bearer token, minting a new one if the cache is empty or
    /// near expiry. Serializes concurrent refreshes behind the cache mutex.
    async fn bearer_token(&self) -> Result<String> {
        let mut cache = self.cache.lock().await;
        let now = Utc::now();
        if let Some(cached) = cache.as_ref()
            && cached.is_fresh(now)
        {
            return Ok(cached.token.clone());
        }

        let minted = self.mint_token().await?;
        let token = minted.token.clone();
        *cache = Some(minted);
        Ok(token)
    }

    /// Perform the client-credentials token request against Entra ID.
    async fn mint_token(&self) -> Result<CachedToken> {
        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", self.config.client_id.as_str()),
            ("client_secret", self.config.client_secret.as_str()),
            ("scope", self.config.scope.as_str()),
        ];

        let response = self
            .http
            .post(self.config.token_url())
            .form(&params)
            .send()
            .await
            .map_err(|e| AgentLoopError::llm(format!("Failed to request Entra ID token: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            // Entra error bodies can include client_secret echoes only on
            // request, never response; the body here is the AADSTS error which
            // is safe (and useful) to surface for diagnosis.
            let body = response.text().await.unwrap_or_default();
            return Err(AgentLoopError::llm(format!(
                "Entra ID token request failed ({status}): {body}"
            )));
        }

        let token: EntraTokenResponse = response.json().await.map_err(|e| {
            AgentLoopError::llm(format!("Failed to parse Entra ID token response: {e}"))
        })?;

        Ok(CachedToken {
            token: token.access_token,
            expires_at: Utc::now() + Duration::seconds(token.expires_in.max(0)),
        })
    }
}

#[async_trait]
impl ProviderAuth for EntraOAuthProvider {
    async fn headers(&self, _request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
        let token = self.bearer_token().await?;
        Ok(vec![(
            "authorization".to_string(),
            format!("Bearer {token}"),
        )])
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_provider::driver_registry::{DriverId, ProviderMetadata};

    /// Build a config the way the server does: the stored credential document
    /// (`api_key`) is parsed into the typed credential map, mirroring
    /// `DriverConfig::from_provider_config`.
    fn driver_config(api_key: Option<&str>, extra: Option<serde_json::Value>) -> DriverConfig {
        DriverConfig {
            provider: everruns_provider::ProviderKey::new("mai"),
            provider_type: DriverId::Mai,
            credentials: everruns_provider::credential_schema::parse_credential_document(api_key),
            api_key: api_key.map(str::to_string),
            base_url: Some("https://example.services.ai.azure.com".to_string()),
            metadata: ProviderMetadata {
                extra,
                ..Default::default()
            },
        }
    }

    fn oauth(tenant: &str) -> EntraOAuthConfig {
        EntraOAuthConfig {
            tenant_id: tenant.into(),
            client_id: "client-marker".into(),
            client_secret: "secret-marker".into(),
            scope: "https://cognitiveservices.azure.com/.default".into(),
            authority: "https://login.microsoftonline.com".into(),
        }
    }
    fn auth_request() -> ProviderAuthRequest<'static> {
        ProviderAuthRequest {
            method: "POST",
            url: "https://resource.services.ai.azure.com/openai/v1/chat/completions",
            headers: &[],
            body: b"{}",
        }
    }
    #[test]
    fn debug_preserves_diagnostics_while_redacting_secrets() {
        assert_eq!(
            format!("{:?}", MaiAuth::ApiKey("key-marker".into())),
            "ApiKey(\"[REDACTED]\")"
        );
        assert_eq!(
            format!("{:?}", MaiAuth::EntraOAuth(oauth("tenant-marker"))),
            "EntraOAuth(EntraOAuthConfig { tenant_id: \"tenant-marker\", client_id: \"client-marker\", client_secret: \"[REDACTED]\", scope: \"https://cognitiveservices.azure.com/.default\", authority: \"https://login.microsoftonline.com\" })"
        );
        assert_eq!(
            format!(
                "{:?}",
                ApiKeyAuth {
                    key: "key-marker".into()
                }
            ),
            "ApiKeyAuth { key: \"[REDACTED]\" }"
        );
        assert_eq!(
            format!(
                "{:?}",
                FailingAuth {
                    message: "private-error-marker".into()
                }
            ),
            "FailingAuth { .. }"
        );
    }
    #[tokio::test]
    async fn credential_and_metadata_precedence_reach_complete_auth_headers() {
        use serde_json::json;
        for (document, extra, key) in [
            ("plain-key", None, "plain-key"),
            (
                r#"{"api_key":"json-key"}"#,
                Some(json!({"unrelated":true})),
                "json-key",
            ),
        ] {
            let provider = MaiAuth::from_driver_config(&driver_config(Some(document), extra))
                .unwrap()
                .into_provider();
            assert_eq!(
                provider.headers(auth_request()).await.unwrap(),
                vec![("api-key".into(), key.into())]
            );
        }
        let typed=json!({"tenant_id":"typed-tenant","client_id":"typed-client","client_secret":"typed-secret","scope":"https://custom/.default","authority":"https://login.microsoftonline.us"}).to_string();
        for (document, extra, expected) in [
            (
                typed.as_str(),
                None,
                (
                    "typed-tenant",
                    "typed-client",
                    "typed-secret",
                    "https://custom/.default",
                    "https://login.microsoftonline.us",
                ),
            ),
            (
                typed.as_str(),
                Some(
                    json!({"auth":"entra","tenant_id":"metadata-tenant","client_id":"metadata-client","client_secret":"metadata-secret"}),
                ),
                (
                    "metadata-tenant",
                    "metadata-client",
                    "metadata-secret",
                    "https://cognitiveservices.azure.com/.default",
                    "https://login.microsoftonline.com",
                ),
            ),
            (
                r#"{"api_key":"fallback-key","tenant_id":"typed-tenant","client_id":"typed-client","client_secret":"typed-secret","scope":"","authority":""}"#,
                None,
                (
                    "typed-tenant",
                    "typed-client",
                    "typed-secret",
                    "https://cognitiveservices.azure.com/.default",
                    "https://login.microsoftonline.com",
                ),
            ),
        ] {
            let provider = MaiAuth::from_driver_config(&driver_config(Some(document), extra))
                .unwrap()
                .into_provider();
            let oauth = provider
                .as_any()
                .downcast_ref::<EntraOAuthProvider>()
                .expect("OAuth strategy");
            let cfg = &oauth.config;
            assert_eq!(
                (
                    cfg.tenant_id.as_str(),
                    cfg.client_id.as_str(),
                    cfg.client_secret.as_str(),
                    cfg.scope.as_str(),
                    cfg.authority.as_str()
                ),
                expected
            );
            assert_eq!(
                cfg.token_url(),
                format!("{}/{}/oauth2/v2.0/token", expected.4, expected.0)
            );
            *oauth.cache.lock().await = Some(CachedToken {
                token: "cached-bearer".into(),
                expires_at: Utc::now() + Duration::seconds(600),
            });
            assert_eq!(
                provider.headers(auth_request()).await.unwrap(),
                vec![("authorization".into(), "Bearer cached-bearer".into())]
            );
        }
    }
    #[test]
    fn partial_oauth_never_falls_back_to_a_valid_api_key() {
        use serde_json::json;
        for missing in ["tenant_id", "client_id", "client_secret"] {
            let mut partial = json!({"tenant_id":"tenant-marker","client_id":"client-marker","client_secret":"secret-marker"});
            partial.as_object_mut().unwrap().remove(missing);
            for from_metadata in [false, true] {
                let mut document = partial.clone();
                document["api_key"] = json!("valid-fallback-key");
                let config = if from_metadata {
                    driver_config(Some("valid-fallback-key"), Some(partial.clone()))
                } else {
                    driver_config(Some(&document.to_string()), None)
                };
                let error = MaiAuth::from_driver_config(&config)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.starts_with("LLM error: Invalid Microsoft Entra ID OAuth config"),
                    "{error}"
                );
                assert!(
                    error.contains(&format!("missing field `{missing}`")),
                    "{error}"
                );
                for secret in ["secret-marker", "valid-fallback-key"] {
                    assert!(!error.contains(secret));
                }
            }
        }
        for key in [None, Some("")] {
            assert_eq!(
                MaiAuth::from_driver_config(&driver_config(key, None))
                    .unwrap_err()
                    .to_string(),
                "LLM error: Microsoft MAI provider is not authenticated: configure an Azure AI Foundry API key, or Entra ID OAuth credentials (tenant_id, client_id, client_secret)."
            );
        }
    }
    #[test]
    fn authority_validation_covers_every_supported_cloud_and_forbidden_component() {
        use serde_json::json;
        for authority in [
            "https://login.microsoftonline.com",
            "https://login.microsoftonline.us",
            "https://login.chinacloudapi.cn",
        ] {
            let document = json!({"tenant_id":"tenant","client_id":"client","client_secret":"secret","authority":authority});
            for from_metadata in [false, true] {
                let config = if from_metadata {
                    driver_config(None, Some(document.clone()))
                } else {
                    driver_config(Some(&document.to_string()), None)
                };
                let MaiAuth::EntraOAuth(auth) = MaiAuth::from_driver_config(&config).unwrap()
                else {
                    panic!("OAuth expected")
                };
                assert_eq!(
                    auth.token_url(),
                    format!("{authority}/tenant/oauth2/v2.0/token")
                );
            }
        }
        for authority in [
            "http://127.0.0.1:39991",
            "https://127.0.0.1",
            "https://localhost",
            "https://169.254.169.254",
            "https://10.0.0.5",
            "https://192.168.1.10",
            "https://example.com",
            "https://login.microsoftonline.com:444",
            "https://login.microsoftonline.com/path",
            "https://login.microsoftonline.com?query=1",
            "https://login.microsoftonline.com#fragment",
            "https://user:pass@login.microsoftonline.com",
            "https://user@login.microsoftonline.com",
            "https://login.microsoftonline.com.evil.example",
            "https://login.microsoftonline.com@evil.example",
        ] {
            let document = json!({"tenant_id":"tenant","client_id":"client","client_secret":"secret","authority":authority});
            for from_metadata in [false, true] {
                let config = if from_metadata {
                    driver_config(None, Some(document.clone()))
                } else {
                    driver_config(Some(&document.to_string()), None)
                };
                let error = MaiAuth::from_driver_config(&config)
                    .unwrap_err()
                    .to_string();
                assert!(
                    error.contains("OAuth authority URL"),
                    "{authority}: {error}"
                );
            }
        }
    }
    #[tokio::test]
    async fn token_minting_cache_and_refresh_preserve_exact_client_credentials() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::builder().start().await;
        let count = Arc::new(AtomicUsize::new(0));
        let counter = count.clone();
        Mock::given(wiremock::matchers::method("POST"))
            .respond_with(move |_: &wiremock::Request| {
                let number = counter.fetch_add(1, Ordering::SeqCst) + 1;
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"access_token":format!("minted-{number}"),"expires_in":600}),
                )
            })
            .mount(&server)
            .await;
        // Direct construction is a trusted embedder path, also used by chat_wire's local token fixture.
        let mut config = oauth("tenant");
        config.authority = server.uri();
        config.client_id = "client+marker".into();
        config.client_secret = "secret&marker".into();
        let provider = EntraOAuthProvider::new(config);
        let results =
            futures::future::join_all((0..5).map(|_| provider.headers(auth_request()))).await;
        for result in results {
            assert_eq!(
                result.unwrap(),
                vec![("authorization".into(), "Bearer minted-1".into())]
            );
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        provider.cache.lock().await.as_mut().unwrap().expires_at =
            Utc::now() + Duration::seconds(119);
        assert_eq!(
            provider.headers(auth_request()).await.unwrap(),
            vec![("authorization".into(), "Bearer minted-2".into())]
        );
        assert_eq!(count.load(Ordering::SeqCst), 2);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests {
            assert_eq!(request.url.path(), "/tenant/oauth2/v2.0/token");
            assert_eq!(
                request.headers["content-type"],
                "application/x-www-form-urlencoded"
            );
            assert_eq!(
                String::from_utf8(request.body).unwrap(),
                "grant_type=client_credentials&client_id=client%2Bmarker&client_secret=secret%26marker&scope=https%3A%2F%2Fcognitiveservices.azure.com%2F.default"
            );
        }
    }
    #[tokio::test]
    async fn failed_refresh_never_reuses_expired_tokens_or_follows_redirects() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let redirect_target = MockServer::builder().start().await;
        for status in [302, 401, 200] {
            let server = MockServer::builder().start().await;
            Mock::given(wiremock::matchers::method("POST"))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("location", redirect_target.uri())
                        .set_body_json(serde_json::json!({"error":"token unavailable"})),
                )
                .expect(1)
                .mount(&server)
                .await;
            let mut config = oauth("tenant");
            config.authority = server.uri();
            let provider = EntraOAuthProvider::new(config);
            *provider.cache.lock().await = Some(CachedToken {
                token: "expired-token".into(),
                expires_at: Utc::now() - Duration::seconds(1),
            });
            let error = provider
                .headers(auth_request())
                .await
                .unwrap_err()
                .to_string();
            assert!(
                error.contains(if status == 200 {
                    "Failed to parse Entra ID token response"
                } else {
                    "Entra ID token request failed"
                }),
                "{error}"
            );
            assert!(!error.contains("expired-token"));
        }
        assert!(
            redirect_target
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
    }
    #[test]
    fn cached_token_refresh_boundary_is_strict_and_deterministic() {
        let now = DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        for (seconds, fresh) in [
            (-1, false),
            (0, false),
            (119, false),
            (120, false),
            (121, true),
            (600, true),
        ] {
            let token = CachedToken {
                token: "cached".into(),
                expires_at: now + Duration::seconds(seconds),
            };
            assert_eq!(token.is_fresh(now), fresh, "expiry offset {seconds}");
        }
    }
}
