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
// Both schemes are exposed through the generic [`AuthHeaderProvider`] hook on
// the core Chat Completions driver, so the protocol driver never has to know
// which scheme is in use. New schemes (managed identity, workload identity
// federation, ...) can be added by implementing [`AuthHeaderProvider`] without
// touching the driver.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::llm_driver_registry::DriverConfig;
use everruns_core::openai_protocol::AuthHeaderProvider;
use serde::Deserialize;
use tokio::sync::Mutex;

/// Default Microsoft Entra ID authority host.
pub const DEFAULT_ENTRA_AUTHORITY: &str = "https://login.microsoftonline.com";

/// Default OAuth scope for Azure AI Foundry / Azure Cognitive Services.
pub const DEFAULT_ENTRA_SCOPE: &str = "https://cognitiveservices.azure.com/.default";

/// Refresh a cached Entra token this long before it actually expires, so an
/// in-flight request never races the expiry boundary.
const TOKEN_REFRESH_SKEW: Duration = Duration::seconds(120);

/// Authentication strategy for a Microsoft MAI provider.
///
/// Built from a [`DriverConfig`] via [`MaiAuth::from_driver_config`]: an Entra
/// OAuth config in `metadata.extra` selects OAuth; otherwise an `api_key`
/// selects API-key auth.
#[derive(Clone, Debug)]
pub enum MaiAuth {
    /// Azure AI Foundry API key (`api-key` header).
    ApiKey(String),
    /// Microsoft Entra ID OAuth client-credentials flow (bearer token).
    EntraOAuth(EntraOAuthConfig),
}

impl MaiAuth {
    /// Resolve the auth strategy from a driver config.
    ///
    /// Precedence: an Entra OAuth block in `metadata.extra` (an object with at
    /// least `tenant_id`, `client_id`, and `client_secret`, optionally guarded
    /// by `"auth": "entra_id"`) wins; otherwise a configured `api_key` is used.
    /// Returns an error when neither is present so misconfiguration fails with a
    /// clear message rather than an opaque 401 at call time.
    pub fn from_driver_config(config: &DriverConfig) -> Result<Self> {
        if let Some(extra) = config.metadata.extra.as_ref()
            && let Some(oauth) = EntraOAuthConfig::from_extra(extra)?
        {
            return Ok(MaiAuth::EntraOAuth(oauth));
        }

        match config.api_key.as_deref() {
            Some(key) if !key.is_empty() => Ok(MaiAuth::ApiKey(key.to_string())),
            _ => Err(AgentLoopError::llm(
                "Microsoft MAI provider is not authenticated: configure an Azure AI \
                 Foundry API key, or Entra ID OAuth credentials (tenant_id, client_id, \
                 client_secret) in the provider metadata.",
            )),
        }
    }

    /// Build the [`AuthHeaderProvider`] this strategy resolves to.
    pub fn into_provider(self) -> Arc<dyn AuthHeaderProvider> {
        match self {
            MaiAuth::ApiKey(key) => Arc::new(ApiKeyAuth { key }),
            MaiAuth::EntraOAuth(config) => Arc::new(EntraOAuthProvider::new(config)),
        }
    }
}

/// Static Azure AI Foundry API-key auth: always emits the `api-key` header.
#[derive(Debug)]
struct ApiKeyAuth {
    key: String,
}

#[async_trait]
impl AuthHeaderProvider for ApiKeyAuth {
    async fn auth_header(&self) -> Result<(String, String)> {
        Ok(("api-key".to_string(), self.key.clone()))
    }
}

/// Microsoft Entra ID client-credentials configuration.
#[derive(Clone, Debug, Deserialize)]
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

        serde_json::from_value::<EntraOAuthConfig>(extra.clone())
            .map(Some)
            .map_err(|e| {
                AgentLoopError::llm(format!(
                    "Invalid Microsoft Entra ID OAuth config in provider metadata: {e}. \
                     Required fields: tenant_id, client_id, client_secret."
                ))
            })
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

/// [`AuthHeaderProvider`] that mints and caches Entra ID bearer tokens via the
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
            http: reqwest::Client::new(),
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
impl AuthHeaderProvider for EntraOAuthProvider {
    async fn auth_header(&self) -> Result<(String, String)> {
        let token = self.bearer_token().await?;
        Ok(("Authorization".to_string(), format!("Bearer {token}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::llm_driver_registry::{DriverId, ProviderMetadata};

    fn driver_config(api_key: Option<&str>, extra: Option<serde_json::Value>) -> DriverConfig {
        DriverConfig {
            provider_type: DriverId::Mai,
            api_key: api_key.map(str::to_string),
            base_url: Some("https://example.services.ai.azure.com".to_string()),
            metadata: ProviderMetadata {
                extra,
                ..Default::default()
            },
        }
    }

    #[test]
    fn api_key_selected_when_no_oauth() {
        let auth = MaiAuth::from_driver_config(&driver_config(Some("key-123"), None)).unwrap();
        assert!(matches!(auth, MaiAuth::ApiKey(k) if k == "key-123"));
    }

    #[test]
    fn oauth_selected_from_metadata_extra() {
        let extra = serde_json::json!({
            "tenant_id": "tenant",
            "client_id": "client",
            "client_secret": "secret",
        });
        let auth = MaiAuth::from_driver_config(&driver_config(Some("key"), Some(extra))).unwrap();
        match auth {
            MaiAuth::EntraOAuth(cfg) => {
                assert_eq!(cfg.tenant_id, "tenant");
                assert_eq!(cfg.scope, DEFAULT_ENTRA_SCOPE);
                assert_eq!(cfg.authority, DEFAULT_ENTRA_AUTHORITY);
                assert_eq!(
                    cfg.token_url(),
                    "https://login.microsoftonline.com/tenant/oauth2/v2.0/token"
                );
            }
            other => panic!("expected EntraOAuth, got {other:?}"),
        }
    }

    #[test]
    fn missing_auth_is_an_error() {
        let err = MaiAuth::from_driver_config(&driver_config(None, None)).unwrap_err();
        assert!(err.to_string().contains("not authenticated"));
    }

    #[test]
    fn partial_oauth_block_is_a_clear_error() {
        // Looks like Entra (has tenant_id) but is missing client_secret.
        let extra = serde_json::json!({ "tenant_id": "t", "client_id": "c" });
        let err = MaiAuth::from_driver_config(&driver_config(None, Some(extra))).unwrap_err();
        assert!(err.to_string().contains("Entra ID OAuth config"));
    }

    #[tokio::test]
    async fn api_key_provider_emits_api_key_header() {
        let provider = MaiAuth::ApiKey("secret-key".to_string()).into_provider();
        let (name, value) = provider.auth_header().await.unwrap();
        assert_eq!(name, "api-key");
        assert_eq!(value, "secret-key");
    }

    #[test]
    fn cached_token_freshness_accounts_for_skew() {
        let now = Utc::now();
        let almost_expired = CachedToken {
            token: "t".into(),
            expires_at: now + Duration::seconds(30),
        };
        assert!(!almost_expired.is_fresh(now));
        let fresh = CachedToken {
            token: "t".into(),
            expires_at: now + Duration::seconds(600),
        };
        assert!(fresh.is_fresh(now));
    }
}
