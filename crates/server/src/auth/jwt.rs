// JWT token service for authentication
// Decision: Use HS256 algorithm for simplicity (symmetric key)
// Decision: Access tokens are short-lived, refresh tokens are stored in DB

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::config::JwtConfig;

/// `token_type` claim value for browser/session/API access tokens.
pub const ACCESS_TOKEN_TYPE: &str = "access";

/// `token_type` claim value for MCP OAuth access tokens.
///
/// MCP tokens are minted distinctly from regular access tokens so the general
/// `/api/*` validation path can reject them and the `/mcp` endpoint can reject
/// regular session/access tokens. Together with the `aud` resource binding this
/// closes the OAuth confused-deputy / missing-audience gap (TM-MCP-006).
pub const MCP_ACCESS_TOKEN_TYPE: &str = "mcp_access";

/// `token_type` claim value for the short-lived MCP OAuth consent (anti-CSRF)
/// token embedded in the `/oauth/authorize` confirmation form.
///
/// The browser-facing consent step must guarantee that the POST which mints an
/// authorization code was driven by a form this server rendered for the
/// signed-in user — not auto-submitted by a malicious page. That was previously
/// enforced with a freshly-set `SameSite=Lax` cookie that had to round-trip
/// from the rendered form back to the POST. In real MCP-client browser contexts
/// (popup / embedded webview / partitioned third-party storage) that second
/// cookie is not reliably stored, while the pre-existing session cookie still
/// rides along — so the POST authenticated the user yet failed with "Missing
/// CSRF cookie". Binding the anti-CSRF token to the session instead (a signed
/// token whose `sub` must equal the POSTing user) removes the dependency on
/// that second cookie while keeping the CSRF guarantee: an attacker cannot mint
/// a token for the victim without the server secret.
pub const OAUTH_CONSENT_TOKEN_TYPE: &str = "mcp_oauth_consent";

/// Lifetime of the MCP OAuth consent token (30 minutes) — i.e. how long a
/// rendered `/oauth/authorize` confirmation page stays submittable.
const OAUTH_CONSENT_TOKEN_LIFETIME_SECS: i64 = 30 * 60;

/// Generate a random identifier string (32 hex characters)
fn generate_random_id() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    hex::encode(bytes)
}

/// JWT claims for access tokens
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccessTokenClaims {
    /// Subject (user ID)
    pub sub: String,
    /// User email
    pub email: String,
    /// User name
    pub name: String,
    /// User roles
    pub roles: Vec<String>,
    /// Token type. `"access"` for browser/session/API tokens, `"mcp_access"`
    /// for resource-bound MCP OAuth tokens.
    pub token_type: String,
    /// Audience / resource indicator. Set to the `/mcp` resource URL for MCP
    /// access tokens (RFC 8707 resource binding); `None` for regular access
    /// tokens. Skipped on the wire when absent so existing tokens are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aud: Option<String>,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
}

/// JWT claims for refresh tokens
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RefreshTokenClaims {
    /// Subject (user ID)
    pub sub: String,
    /// Token type
    pub token_type: String,
    /// Expiration time (Unix timestamp)
    pub exp: i64,
    /// Issued at (Unix timestamp)
    pub iat: i64,
    /// Unique token ID (for revocation)
    pub jti: String,
}

/// Claims for the short-lived MCP OAuth consent (anti-CSRF) token embedded in
/// the `/oauth/authorize` confirmation form. See [`OAUTH_CONSENT_TOKEN_TYPE`].
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OAuthConsentClaims {
    /// Subject (user ID) the consent form was rendered for. The POST handler
    /// must confirm this equals the authenticated session user.
    pub sub: String,
    /// Token type — always [`OAUTH_CONSENT_TOKEN_TYPE`].
    pub token_type: String,
    /// Expiration time (Unix timestamp).
    pub exp: i64,
    /// Issued at (Unix timestamp).
    pub iat: i64,
}

/// Token pair returned after successful authentication
#[derive(Debug, Serialize, Deserialize)]
pub struct TokenPair {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: i64,
}

/// JWT service for token generation and validation
#[derive(Clone)]
pub struct JwtService {
    config: JwtConfig,
    encoding_key: EncodingKey,
    decoding_key: DecodingKey,
}

impl JwtService {
    pub fn new(config: JwtConfig) -> Self {
        let encoding_key = EncodingKey::from_secret(config.secret.as_bytes());
        let decoding_key = DecodingKey::from_secret(config.secret.as_bytes());

        Self {
            config,
            encoding_key,
            decoding_key,
        }
    }

    /// Generate access token for a user
    pub fn generate_access_token(
        &self,
        user_id: Uuid,
        email: &str,
        name: &str,
        roles: &[String],
    ) -> Result<String> {
        let now = Utc::now();
        let exp = now + Duration::from_std(self.config.access_token_lifetime)?;

        let claims = AccessTokenClaims {
            sub: user_id.to_string(),
            email: email.to_string(),
            name: name.to_string(),
            roles: roles.to_vec(),
            token_type: ACCESS_TOKEN_TYPE.to_string(),
            aud: None,
            exp: exp.timestamp(),
            iat: now.timestamp(),
        };

        encode(&Header::default(), &claims, &self.encoding_key)
            .context("Failed to encode access token")
    }

    /// Generate a resource-bound MCP access token for a user.
    ///
    /// Distinct from [`generate_access_token`](Self::generate_access_token):
    /// `token_type` is `"mcp_access"` and `aud` is bound to the MCP resource
    /// (e.g. `https://app.example.com/mcp`). This lets the `/mcp` endpoint accept
    /// only these tokens while the general `/api/*` validate path rejects them —
    /// preventing a token minted for `/mcp` from acting as a full user token on
    /// the entire REST API (TM-MCP-006).
    pub fn generate_mcp_access_token(
        &self,
        user_id: Uuid,
        email: &str,
        name: &str,
        roles: &[String],
        resource: &str,
    ) -> Result<String> {
        let now = Utc::now();
        let exp = now + Duration::from_std(self.config.access_token_lifetime)?;

        let claims = AccessTokenClaims {
            sub: user_id.to_string(),
            email: email.to_string(),
            name: name.to_string(),
            roles: roles.to_vec(),
            token_type: MCP_ACCESS_TOKEN_TYPE.to_string(),
            aud: Some(resource.to_string()),
            exp: exp.timestamp(),
            iat: now.timestamp(),
        };

        encode(&Header::default(), &claims, &self.encoding_key)
            .context("Failed to encode MCP access token")
    }

    /// Generate refresh token for a user
    pub fn generate_refresh_token(&self, user_id: Uuid) -> Result<(String, String)> {
        let now = Utc::now();
        let exp = now + Duration::from_std(self.config.refresh_token_lifetime)?;
        let jti = generate_random_id();

        let claims = RefreshTokenClaims {
            sub: user_id.to_string(),
            token_type: "refresh".to_string(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
            jti: jti.clone(),
        };

        let token = encode(&Header::default(), &claims, &self.encoding_key)
            .context("Failed to encode refresh token")?;

        Ok((token, jti))
    }

    /// Generate both access and refresh tokens
    pub fn generate_token_pair(
        &self,
        user_id: Uuid,
        email: &str,
        name: &str,
        roles: &[String],
    ) -> Result<(TokenPair, String)> {
        let access_token = self.generate_access_token(user_id, email, name, roles)?;
        let (refresh_token, jti) = self.generate_refresh_token(user_id)?;

        let token_pair = TokenPair {
            access_token,
            refresh_token,
            token_type: "Bearer".to_string(),
            expires_in: self.config.access_token_lifetime.as_secs() as i64,
        };

        Ok((token_pair, jti))
    }

    /// Validate and decode a regular access token (browser/session/API).
    ///
    /// Rejects MCP access tokens (`token_type == "mcp_access"`): they must only
    /// be accepted by the `/mcp` endpoint via [`validate_mcp_access_token`].
    pub fn validate_access_token(&self, token: &str) -> Result<AccessTokenClaims> {
        // The `aud` claim is only set on MCP tokens. Disable the library's
        // audience check here so a stray/legacy `aud` never gates regular
        // tokens — the `token_type` gate below is the real resource boundary.
        let validation = Validation {
            validate_exp: true,
            validate_aud: false,
            ..Default::default()
        };

        let token_data = decode::<AccessTokenClaims>(token, &self.decoding_key, &validation)
            .context("Invalid access token")?;

        if token_data.claims.token_type != ACCESS_TOKEN_TYPE {
            anyhow::bail!("Invalid token type");
        }

        Ok(token_data.claims)
    }

    /// Validate and decode an MCP access token, enforcing the resource binding.
    ///
    /// Accepts only tokens minted by [`generate_mcp_access_token`]: the
    /// `token_type` must be `"mcp_access"` and the `aud` claim must equal the
    /// expected `/mcp` resource. Regular session/access tokens are rejected.
    pub fn validate_mcp_access_token(
        &self,
        token: &str,
        expected_resource: &str,
    ) -> Result<AccessTokenClaims> {
        // We compare `aud` explicitly below so we can return a precise error and
        // avoid depending on the library's set-membership semantics.
        let validation = Validation {
            validate_exp: true,
            validate_aud: false,
            ..Default::default()
        };

        let token_data = decode::<AccessTokenClaims>(token, &self.decoding_key, &validation)
            .context("Invalid MCP access token")?;

        if token_data.claims.token_type != MCP_ACCESS_TOKEN_TYPE {
            anyhow::bail!("Invalid token type for MCP resource");
        }

        match token_data.claims.aud.as_deref() {
            Some(aud) if aud == expected_resource => Ok(token_data.claims),
            _ => anyhow::bail!("MCP access token audience mismatch"),
        }
    }

    /// Generate a short-lived signed consent token binding the
    /// `/oauth/authorize` confirmation form to `user_id`.
    ///
    /// Embedded as a hidden form field and re-validated on POST against the
    /// authenticated session user (anti-CSRF). Replaces the previous
    /// freshly-set cookie that did not reliably round-trip from real MCP-client
    /// browser contexts — see [`OAUTH_CONSENT_TOKEN_TYPE`].
    pub fn generate_oauth_consent_token(&self, user_id: Uuid) -> Result<String> {
        let now = Utc::now();
        let exp = now + Duration::seconds(OAUTH_CONSENT_TOKEN_LIFETIME_SECS);

        let claims = OAuthConsentClaims {
            sub: user_id.to_string(),
            token_type: OAUTH_CONSENT_TOKEN_TYPE.to_string(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
        };

        encode(&Header::default(), &claims, &self.encoding_key)
            .context("Failed to encode OAuth consent token")
    }

    /// Validate and decode an OAuth consent token minted by
    /// [`generate_oauth_consent_token`].
    ///
    /// Checks the signature, expiry, and token type. The caller MUST additionally
    /// confirm the returned `sub` matches the authenticated session user — the
    /// signature alone only proves this server issued the token, not for whom.
    pub fn validate_oauth_consent_token(&self, token: &str) -> Result<OAuthConsentClaims> {
        let validation = Validation {
            validate_exp: true,
            validate_aud: false,
            ..Default::default()
        };

        let token_data = decode::<OAuthConsentClaims>(token, &self.decoding_key, &validation)
            .context("Invalid OAuth consent token")?;

        if token_data.claims.token_type != OAUTH_CONSENT_TOKEN_TYPE {
            anyhow::bail!("Invalid token type for OAuth consent");
        }

        Ok(token_data.claims)
    }

    /// Validate and decode a refresh token
    pub fn validate_refresh_token(&self, token: &str) -> Result<RefreshTokenClaims> {
        let validation = Validation {
            validate_exp: true,
            ..Default::default()
        };

        let token_data = decode::<RefreshTokenClaims>(token, &self.decoding_key, &validation)
            .context("Invalid refresh token")?;

        if token_data.claims.token_type != "refresh" {
            anyhow::bail!("Invalid token type");
        }

        Ok(token_data.claims)
    }

    /// Get refresh token lifetime in seconds
    pub fn refresh_token_lifetime_secs(&self) -> i64 {
        self.config.refresh_token_lifetime.as_secs() as i64
    }

    /// Get access token lifetime in seconds.
    pub fn access_token_lifetime_secs(&self) -> i64 {
        self.config.access_token_lifetime.as_secs() as i64
    }
}

/// Hash a token for database storage (using SHA-256)
pub fn hash_token(token: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(token.as_bytes());
    hex::encode(hash)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration as StdDuration;

    fn test_config() -> JwtConfig {
        JwtConfig {
            secret: "test-secret-key-for-testing".to_string(),
            access_token_lifetime: StdDuration::from_secs(900), // 15 minutes
            refresh_token_lifetime: StdDuration::from_secs(86400), // 1 day
        }
    }

    #[test]
    fn test_generate_access_token() {
        let service = JwtService::new(test_config());
        let user_id = Uuid::nil(); // Use nil UUID for testing
        let token = service
            .generate_access_token(
                user_id,
                "test@example.com",
                "Test User",
                &["user".to_string()],
            )
            .unwrap();

        assert!(!token.is_empty());

        // Validate the token
        let claims = service.validate_access_token(&token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.email, "test@example.com");
        assert_eq!(claims.name, "Test User");
        assert_eq!(claims.roles, vec!["user".to_string()]);
        assert_eq!(claims.token_type, "access");
    }

    #[test]
    fn test_generate_refresh_token() {
        let service = JwtService::new(test_config());
        let user_id = Uuid::nil(); // Use nil UUID for testing
        let (token, jti) = service.generate_refresh_token(user_id).unwrap();

        assert!(!token.is_empty());
        assert!(!jti.is_empty());

        // Validate the token
        let claims = service.validate_refresh_token(&token).unwrap();
        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.token_type, "refresh");
        assert_eq!(claims.jti, jti);
    }

    #[test]
    fn test_generate_token_pair() {
        let service = JwtService::new(test_config());
        let user_id = Uuid::nil(); // Use nil UUID for testing
        let (pair, jti) = service
            .generate_token_pair(user_id, "test@example.com", "Test", &["user".to_string()])
            .unwrap();

        assert_eq!(pair.token_type, "Bearer");
        assert!(!pair.access_token.is_empty());
        assert!(!pair.refresh_token.is_empty());
        assert!(!jti.is_empty());
    }

    #[test]
    fn test_invalid_token() {
        let service = JwtService::new(test_config());
        let result = service.validate_access_token("invalid-token");
        assert!(result.is_err());
    }

    #[test]
    fn test_wrong_token_type() {
        let service = JwtService::new(test_config());
        let user_id = Uuid::nil(); // Use nil UUID for testing

        // Generate refresh token
        let (refresh_token, _) = service.generate_refresh_token(user_id).unwrap();

        // Try to validate as access token
        let result = service.validate_access_token(&refresh_token);
        assert!(result.is_err());
    }

    const MCP_RESOURCE: &str = "https://app.example.com/mcp";

    #[test]
    fn test_mcp_token_carries_distinguishing_claims() {
        let service = JwtService::new(test_config());
        let user_id = Uuid::nil();
        let token = service
            .generate_mcp_access_token(
                user_id,
                "test@example.com",
                "Test User",
                &["user".to_string()],
                MCP_RESOURCE,
            )
            .unwrap();

        // The MCP token must validate on the MCP path and carry the
        // distinguishing token_type + audience claims.
        let claims = service
            .validate_mcp_access_token(&token, MCP_RESOURCE)
            .unwrap();
        assert_eq!(claims.token_type, MCP_ACCESS_TOKEN_TYPE);
        assert_eq!(claims.aud.as_deref(), Some(MCP_RESOURCE));
    }

    #[test]
    fn test_mcp_token_rejected_by_general_access_validation() {
        // (a) An mcp_access token must be rejected by the general /api/* path.
        let service = JwtService::new(test_config());
        let token = service
            .generate_mcp_access_token(
                Uuid::nil(),
                "test@example.com",
                "Test User",
                &["user".to_string()],
                MCP_RESOURCE,
            )
            .unwrap();

        assert!(
            service.validate_access_token(&token).is_err(),
            "mcp_access token must not validate on the general access path"
        );
    }

    #[test]
    fn test_regular_access_token_rejected_at_mcp_path() {
        // (b) A normal session/access token must be rejected at the /mcp path.
        let service = JwtService::new(test_config());
        let token = service
            .generate_access_token(
                Uuid::nil(),
                "test@example.com",
                "Test User",
                &["user".to_string()],
            )
            .unwrap();

        assert!(
            service
                .validate_mcp_access_token(&token, MCP_RESOURCE)
                .is_err(),
            "regular access token must not validate on the MCP path"
        );
    }

    #[test]
    fn test_mcp_token_audience_must_match() {
        let service = JwtService::new(test_config());
        let token = service
            .generate_mcp_access_token(
                Uuid::nil(),
                "test@example.com",
                "Test User",
                &["user".to_string()],
                MCP_RESOURCE,
            )
            .unwrap();

        // A token bound to one resource must not validate against another.
        assert!(
            service
                .validate_mcp_access_token(&token, "https://evil.example.com/mcp")
                .is_err(),
            "MCP token must be bound to its audience"
        );
    }

    #[test]
    fn test_oauth_consent_token_roundtrips_and_binds_user() {
        let service = JwtService::new(test_config());
        let user_id = Uuid::from_u128(0x1234_5678);

        let token = service.generate_oauth_consent_token(user_id).unwrap();
        let claims = service.validate_oauth_consent_token(&token).unwrap();

        assert_eq!(claims.sub, user_id.to_string());
        assert_eq!(claims.token_type, OAUTH_CONSENT_TOKEN_TYPE);
    }

    #[test]
    fn test_oauth_consent_token_rejects_other_token_types() {
        // A regular access token must not be accepted as a consent token, and a
        // consent token must not be accepted on the general access path.
        let service = JwtService::new(test_config());
        let access = service
            .generate_access_token(Uuid::nil(), "u@example.com", "U", &["user".to_string()])
            .unwrap();
        assert!(service.validate_oauth_consent_token(&access).is_err());

        let consent = service.generate_oauth_consent_token(Uuid::nil()).unwrap();
        assert!(service.validate_access_token(&consent).is_err());
    }

    #[test]
    fn test_oauth_consent_token_rejected_under_wrong_secret() {
        let issuer = JwtService::new(test_config());
        let token = issuer.generate_oauth_consent_token(Uuid::nil()).unwrap();

        let attacker = JwtService::new(JwtConfig {
            secret: "a-totally-different-secret".to_string(),
            ..test_config()
        });
        assert!(
            attacker.validate_oauth_consent_token(&token).is_err(),
            "consent token must not validate under a different signing secret"
        );
    }

    #[test]
    fn test_hash_token() {
        let token = "test-token-123";
        let hash1 = hash_token(token);
        let hash2 = hash_token(token);

        // Same input produces same hash
        assert_eq!(hash1, hash2);

        // Hash is a valid hex string
        assert!(hash1.chars().all(|c| c.is_ascii_hexdigit()));

        // SHA-256 produces 64 hex characters
        assert_eq!(hash1.len(), 64);
    }
}
