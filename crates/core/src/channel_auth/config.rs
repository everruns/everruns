//! Authentication config for one agent channel (or one serve app).
//!
//! The same shape is stored on an everruns server channel row and read from a
//! serve app's `serve.toml`, so a method list moves between hosts unchanged.

use serde::{Deserialize, Serialize};

fn is_false(value: &bool) -> bool {
    !*value
}

/// Agent channel authentication mode.
///
/// Stored on `AgentChannel.auth` so users can protect one endpoint without first
/// creating org-level identity-provider state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ChannelAuthMode {
    Anonymous,
    SharedSecret,
    ApiKey,
    GoogleOidc,
    Oidc,
    #[serde(rename = "oauth2_introspection", alias = "o_auth2_introspection")]
    OAuth2Introspection,
    HttpBasic,
    Mtls,
}

/// OIDC/OAuth/basic/mTLS provider details for one channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ChannelAuthProviderConfig {
    GoogleOidc {
        client_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_domains: Vec<String>,
    },
    Oidc {
        issuer: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        jwks_url: Option<String>,
    },
    #[serde(rename = "oauth2_introspection", alias = "o_auth2_introspection")]
    OAuth2Introspection {
        introspection_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        client_secret: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        client_secret_configured: bool,
    },
    HttpBasic {
        username: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        password_hash: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        password_configured: bool,
    },
    Mtls {
        header_name: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        allowed_values: Vec<String>,
        /// Header the trusted reverse proxy uses to prove its identity.
        /// Required. Configs without this field fail closed at verification time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proxy_secret_header: Option<String>,
        /// Shared secret the trusted proxy includes in `proxy_secret_header`.
        /// Write-only: redacted in GET responses. See TM-AUTH-021.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        proxy_secret: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        proxy_secret_configured: bool,
    },
}

/// Claim and credential requirements common to channel auth providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelAuthRequirements {
    /// JWT `aud` values to require on inbound tokens. Empty list disables audience checking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub audiences: Vec<String>,
    /// OAuth scope strings to require (space-delimited per scope entry). Empty list disables scope checking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scopes: Vec<String>,
    /// Arbitrary claim equality predicates. Empty map disables claim filtering.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub claims: serde_json::Map<String, serde_json::Value>,
    /// Allowlist of `sub` claim values. Empty list disables subject filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subjects: Vec<String>,
    /// Allowlist of group memberships (from `groups` claim). Empty list disables group filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<String>,
    /// Allowlist of email/identifier domains. Empty list disables domain filtering.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub domains: Vec<String>,
}

/// Authentication config for one channel/channel.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(
    feature = "openapi",
    schema(example = json!({"mode": "api_key", "requirements": {"audiences": ["everruns-api"], "scopes": ["app:invoke"]}}))
)]
pub struct ChannelAuthConfig {
    pub mode: ChannelAuthMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<ChannelAuthProviderConfig>,
    #[serde(default)]
    pub requirements: ChannelAuthRequirements,
}

/// Issuer of AgentID, AgentMail's OpenID Connect provider for AI agents.
pub const AGENTID_ISSUER: &str = "https://auth.agentid.com";

/// Binding provider for subjects proven by AgentID's own discovered keys.
pub const AGENTID_PROVIDER: &str = "agentid";

/// Binding provider for every other OIDC-verified subject.
pub const OIDC_PROVIDER: &str = "oidc";

/// Issuer of Google Sign-In ID tokens.
pub const GOOGLE_ISSUER: &str = "https://accounts.google.com";

impl ChannelAuthConfig {
    /// The AgentID channel preset.
    ///
    /// Decision: AgentID is ordinary `oidc` channel auth, not a verifier mode of
    /// its own. The preset pins the issuer, leaves the keys to AgentID discovery,
    /// requires the operator's registered client id as audience, and requires
    /// `actor_type = "agent"` (every AgentID subject is an agent inbox).
    pub fn agentid_preset(client_id: &str) -> Self {
        let mut claims = serde_json::Map::new();
        claims.insert(
            "actor_type".to_string(),
            serde_json::Value::String("agent".to_string()),
        );
        Self {
            mode: ChannelAuthMode::Oidc,
            provider: Some(ChannelAuthProviderConfig::Oidc {
                issuer: AGENTID_ISSUER.to_string(),
                jwks_url: None,
            }),
            requirements: ChannelAuthRequirements {
                audiences: vec![client_id.trim().to_string()],
                claims,
                ..Default::default()
            },
        }
    }

    /// Whether this config verifies AgentID tokens against AgentID's own
    /// discovered keys. A config that names its own JWKS URL is not AgentID,
    /// whatever issuer it claims.
    pub fn is_agentid(&self) -> bool {
        self.mode == ChannelAuthMode::Oidc
            && matches!(
                self.provider.as_ref(),
                Some(ChannelAuthProviderConfig::Oidc { issuer, jwks_url: None })
                    if issuer.trim().trim_end_matches('/') == AGENTID_ISSUER
            )
    }
}
