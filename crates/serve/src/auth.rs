//! Authentication for a serve app's agent routes.
//!
//! With no method configured the API stays open, as it always was: run it
//! locally or behind a host that authenticates. Once any method is
//! configured, every agent route needs `Authorization: Bearer <credential>`
//! that one of them accepts, and anything else gets `401` with
//! `WWW-Authenticate: Bearer`.
//!
//! Decisions:
//! - The same method list as an everruns API channel: static keys (serve's
//!   stand-in for agent keys) plus the channel auth configs the server
//!   stores, checked by the server's own verifier from
//!   `everruns::channel_auth`. OIDC/JWKS and OAuth 2.0 introspection, with
//!   their claim requirements, behave exactly as on a server channel.
//! - Methods add up from three places: [`ServerBuilder::auth`](crate::ServerBuilder::auth)
//!   in code, `[auth]` in `serve.toml`, and `SERVE_API_KEYS` (comma
//!   separated) in the environment, which is where real keys belong:
//!   `serve.toml` is embedded in the binary.
//! - Public without a credential: `/health`, each agent's card
//!   (`GET /v1/channels/{agent}`, which names the accepted methods), the A2A
//!   Agent Card, and the voice test page. Messaging channel webhooks
//!   (`POST /v1/channels/{channel}`) stay public too: Slack and webhook
//!   callers cannot send a serve credential, and each driver checks its own
//!   platform signature.
//! - Keys are kept as SHA-256 digests and compared in constant time, so
//!   neither a key's bytes nor its length leak through timing.
//! - Authentication only. The verified caller is put on the request
//!   ([`Caller`]), but sessions are not isolated per caller: every
//!   authenticated caller can reach every session, as an operator can.

use std::sync::Arc;

use anyhow::bail;
use axum::extract::{Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
/// The channel auth types a method is written in, and the verifier.
pub use everruns::channel_auth::{
    ChannelAuthConfig, ChannelAuthMode, ChannelAuthPrincipal, ChannelAuthProviderConfig,
    ChannelAuthRequirements, ChannelAuthVerifier,
};
use everruns::channel_auth::{
    ChannelAuthError, GOOGLE_ISSUER, LegacyChannelAuth, constant_time_eq, extract_bearer,
};
use everruns::execution_api::AgentCardAuth;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::host::Host;

/// The environment variable holding static API keys, comma separated.
pub const API_KEYS_ENV: &str = "SERVE_API_KEYS";

/// One way a caller may authenticate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AuthMethod {
    /// `Authorization: Bearer <key>` with this static key.
    ApiKey(String),
    /// `Authorization: Bearer <token>` checked by the channel verifier: an
    /// `oidc`, `google_oidc` or `oauth2_introspection` config, the shape an
    /// everruns channel stores.
    Token(Box<ChannelAuthConfig>),
}

impl AuthMethod {
    /// A static bearer key.
    pub fn api_key(key: impl Into<String>) -> Self {
        Self::ApiKey(key.into())
    }

    /// JWTs from the OpenID Connect provider `issuer`, verified against the
    /// keys its discovery document names, with one of `audiences` in `aud`.
    /// Tighten with [`AuthMethod::requirements`].
    pub fn oidc(
        issuer: impl Into<String>,
        audiences: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self::Token(Box::new(ChannelAuthConfig {
            mode: ChannelAuthMode::Oidc,
            provider: Some(ChannelAuthProviderConfig::Oidc {
                issuer: issuer.into(),
                jwks_url: None,
            }),
            requirements: ChannelAuthRequirements {
                audiences: audiences.into_iter().map(Into::into).collect(),
                ..Default::default()
            },
        }))
    }

    /// Opaque tokens checked at an OAuth 2.0 introspection endpoint
    /// (RFC 7662), authenticating to it as `client` (id, secret) when given.
    pub fn oauth2_introspection(
        introspection_url: impl Into<String>,
        client: Option<(String, String)>,
    ) -> Self {
        let (client_id, client_secret) = match client {
            Some((id, secret)) => (Some(id), Some(secret)),
            None => (None, None),
        };
        Self::Token(Box::new(ChannelAuthConfig {
            mode: ChannelAuthMode::OAuth2Introspection,
            provider: Some(ChannelAuthProviderConfig::OAuth2Introspection {
                introspection_url: introspection_url.into(),
                client_id,
                client_secret,
                client_secret_configured: false,
            }),
            requirements: ChannelAuthRequirements::default(),
        }))
    }

    /// Replace a token method's claim requirements (scopes, subjects, groups,
    /// domains, claims, and audiences). No effect on a static key.
    pub fn requirements(mut self, requirements: ChannelAuthRequirements) -> Self {
        if let Self::Token(config) = &mut self {
            config.requirements = requirements;
        }
        self
    }
}

/// `[auth]` in `serve.toml`.
///
/// ```toml
/// [auth]
/// api_keys = []                    # prefer SERVE_API_KEYS: serve.toml ships in the binary
///
/// [[auth.methods]]                 # an everruns channel auth config
/// mode = "oidc"
/// provider = { type = "oidc", issuer = "https://login.example.com" }
/// requirements = { audiences = ["support-agent"] }
/// ```
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Static bearer keys.
    #[serde(default, skip_serializing)]
    pub api_keys: Vec<String>,
    /// Token methods, in the channel auth config shape.
    #[serde(default, skip_serializing)]
    pub methods: Vec<ChannelAuthConfig>,
}

impl AuthConfig {
    pub(crate) fn methods(&self) -> impl Iterator<Item = AuthMethod> + '_ {
        self.api_keys.iter().cloned().map(AuthMethod::ApiKey).chain(
            self.methods
                .iter()
                .map(|config| AuthMethod::Token(Box::new(config.clone()))),
        )
    }
}

/// Static keys from the value of [`API_KEYS_ENV`]: comma separated, blanks
/// ignored.
pub fn api_keys_from_env_value(value: &str) -> Vec<AuthMethod> {
    value
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(AuthMethod::api_key)
        .collect()
}

/// Who a request was let in as. Inserted into the request's extensions on
/// every authenticated request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Caller {
    /// A static key.
    ApiKey,
    /// A token, and the subject it proved.
    Token(ChannelAuthPrincipal),
}

/// The resolved method list a server enforces.
pub(crate) struct Auth {
    key_digests: Vec<[u8; 32]>,
    tokens: Vec<ChannelAuthConfig>,
    verifier: ChannelAuthVerifier,
}

impl Auth {
    /// `None` when no method is configured: the API stays open.
    pub(crate) fn new(
        methods: Vec<AuthMethod>,
        verifier: Option<ChannelAuthVerifier>,
    ) -> crate::Result<Option<Self>> {
        if methods.is_empty() {
            return Ok(None);
        }
        let mut key_digests = Vec::new();
        let mut tokens = Vec::new();
        for method in methods {
            match method {
                AuthMethod::ApiKey(key) => {
                    if key.trim().is_empty() {
                        bail!("auth: an API key is empty");
                    }
                    let digest: [u8; 32] = Sha256::digest(key.as_bytes()).into();
                    if !key_digests.contains(&digest) {
                        key_digests.push(digest);
                    }
                }
                AuthMethod::Token(config) => {
                    validate_token_method(&config)?;
                    tokens.push(*config);
                }
            }
        }
        Ok(Some(Self {
            key_digests,
            tokens,
            verifier: verifier.unwrap_or_default(),
        }))
    }

    /// The entries the agent card lists.
    pub(crate) fn card(&self) -> Vec<everruns::execution_api::AgentCardAuth> {
        let mut auth = Vec::new();
        if !self.key_digests.is_empty() {
            auth.push(AgentCardAuth::AgentKey);
        }
        for config in &self.tokens {
            let entry = match &config.provider {
                Some(ChannelAuthProviderConfig::Oidc { issuer, .. }) => AgentCardAuth::Oidc {
                    issuer: issuer.clone(),
                },
                Some(ChannelAuthProviderConfig::GoogleOidc { .. }) => AgentCardAuth::Oidc {
                    issuer: GOOGLE_ISSUER.to_string(),
                },
                _ => AgentCardAuth::OAuth2,
            };
            if !auth.contains(&entry) {
                auth.push(entry);
            }
        }
        auth
    }

    async fn check(&self, headers: &axum::http::HeaderMap) -> Result<Caller, ChannelAuthError> {
        let Some(token) = extract_bearer(headers) else {
            return Err(ChannelAuthError::Unauthorized);
        };
        let digest = Sha256::digest(token.trim().as_bytes());
        // Every key is compared, so the time taken does not say which matched.
        let key_matched = self.key_digests.iter().fold(false, |matched, key| {
            matched | constant_time_eq(key, &digest)
        });
        if key_matched {
            return Ok(Caller::ApiKey);
        }
        let mut error = ChannelAuthError::Unauthorized;
        for config in &self.tokens {
            match self
                .verifier
                .verify_principal(config, headers, LegacyChannelAuth::default())
                .await
            {
                Ok(Some(principal)) => return Ok(Caller::Token(principal)),
                Ok(None) | Err(ChannelAuthError::Unauthorized) => {}
                // One provider being down must not mask a credential another
                // one accepts, so keep trying and report it only at the end.
                Err(other) => error = other,
            }
        }
        Err(error)
    }
}

/// A token method this host can enforce, or why not.
fn validate_token_method(config: &ChannelAuthConfig) -> crate::Result {
    match (&config.mode, &config.provider) {
        (ChannelAuthMode::Oidc, Some(ChannelAuthProviderConfig::Oidc { issuer, .. })) => {
            if issuer.trim().is_empty() {
                bail!("auth: an oidc method has no issuer");
            }
            if config.requirements.audiences.is_empty() {
                bail!("auth: the oidc method for {issuer} needs at least one audience");
            }
        }
        (
            ChannelAuthMode::GoogleOidc,
            Some(ChannelAuthProviderConfig::GoogleOidc { client_id, .. }),
        ) => {
            if client_id.trim().is_empty() {
                bail!("auth: a google_oidc method has no client_id");
            }
        }
        (
            ChannelAuthMode::OAuth2Introspection,
            Some(ChannelAuthProviderConfig::OAuth2Introspection {
                introspection_url, ..
            }),
        ) => {
            if introspection_url.trim().is_empty() {
                bail!("auth: an oauth2_introspection method has no introspection_url");
            }
        }
        (mode, _) => bail!(
            "auth: unsupported method {mode:?}; serve takes static API keys and \
             oidc, google_oidc or oauth2_introspection with a matching provider"
        ),
    }
    Ok(())
}

/// Routes reachable without a credential once auth is on.
fn is_public(method: &Method, path: &str) -> bool {
    let segments: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    let read = method == Method::GET || method == Method::HEAD;
    match segments.as_slice() {
        ["health"] => true,
        // GET is the agent card; POST is a messaging channel's webhook,
        // verified by its driver.
        ["v1", "channels", _] => true,
        [
            "v1",
            "channels" | "e",
            _,
            "a2a",
            ".well-known",
            "agent-card.json",
        ] => read,
        ["v1", "channels", _, "voice"] => read,
        _ => false,
    }
}

fn problem(status: StatusCode, detail: &str) -> Response {
    let body = serde_json::json!({
        "title": status.canonical_reason().unwrap_or("Error"),
        "status": status.as_u16(),
        "detail": detail,
    });
    let mut response = (status, axum::Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/problem+json"),
    );
    response
}

/// The middleware in front of every route.
pub(crate) async fn require(
    State(host): State<Arc<Host>>,
    mut request: Request,
    next: Next,
) -> Response {
    let Some(auth) = host.auth() else {
        return next.run(request).await;
    };
    if is_public(request.method(), request.uri().path()) {
        return next.run(request).await;
    }
    match auth.check(request.headers()).await {
        Ok(caller) => {
            request.extensions_mut().insert(caller);
            next.run(request).await
        }
        Err(ChannelAuthError::ProviderUnavailable) => problem(
            StatusCode::SERVICE_UNAVAILABLE,
            "the identity provider could not be reached",
        ),
        Err(_) => {
            let mut response = problem(
                StatusCode::UNAUTHORIZED,
                "a valid bearer credential is required",
            );
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
            response
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_keys_split_on_commas_and_skip_blanks() {
        assert_eq!(
            api_keys_from_env_value(" one, two ,,three,"),
            vec![
                AuthMethod::api_key("one"),
                AuthMethod::api_key("two"),
                AuthMethod::api_key("three"),
            ]
        );
        assert!(api_keys_from_env_value(" , ").is_empty());
    }

    #[test]
    fn no_method_means_no_auth() {
        assert!(Auth::new(Vec::new(), None).unwrap().is_none());
    }

    #[test]
    fn unenforceable_methods_fail_at_boot() {
        let anonymous = ChannelAuthConfig {
            mode: ChannelAuthMode::Anonymous,
            provider: None,
            requirements: ChannelAuthRequirements::default(),
        };
        for method in [
            AuthMethod::api_key(" "),
            AuthMethod::Token(Box::new(anonymous)),
            AuthMethod::oidc("https://login.example.com", Vec::<String>::new()),
            AuthMethod::oauth2_introspection("", None),
        ] {
            assert!(Auth::new(vec![method.clone()], None).is_err(), "{method:?}");
        }
    }

    #[test]
    fn the_card_names_each_kind_once() {
        let auth = Auth::new(
            vec![
                AuthMethod::api_key("a"),
                AuthMethod::api_key("b"),
                AuthMethod::oidc("https://login.example.com", ["serve"]),
                AuthMethod::oauth2_introspection("https://login.example.com/introspect", None),
            ],
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(
            auth.card(),
            vec![
                AgentCardAuth::AgentKey,
                AgentCardAuth::Oidc {
                    issuer: "https://login.example.com".to_string()
                },
                AgentCardAuth::OAuth2,
            ]
        );
    }

    #[test]
    fn only_discovery_routes_are_public() {
        let get = Method::GET;
        let post = Method::POST;
        assert!(is_public(&get, "/health"));
        assert!(is_public(&get, "/v1/channels/support"));
        assert!(is_public(&post, "/v1/channels/slack"));
        assert!(is_public(
            &get,
            "/v1/channels/support/a2a/.well-known/agent-card.json"
        ));
        assert!(is_public(
            &get,
            "/v1/e/support/a2a/.well-known/agent-card.json"
        ));
        assert!(is_public(&get, "/v1/channels/support/voice"));
        for (method, path) in [
            (&get, "/v1/agent"),
            (&post, "/v1/sessions"),
            (&get, "/v1/sessions/s1/sse"),
            (&post, "/v1/channels/support/sessions"),
            (&get, "/v1/channels/support/sessions/s1/events"),
            (&post, "/v1/channels/support/ag-ui"),
            (&post, "/v1/e/support/ag-ui"),
            (&post, "/v1/channels/support/a2a"),
            (&post, "/v1/channels/support/voice/calls"),
            (&post, "/v1/channels/support/voice"),
            (&post, "/dev/schedules/weekly"),
        ] {
            assert!(!is_public(method, path), "{method} {path}");
        }
    }
}
