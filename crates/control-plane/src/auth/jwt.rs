// JWT token service for authentication
// Decision: Use HS256 algorithm for simplicity (symmetric key)
// Decision: Access tokens are short-lived, refresh tokens are stored in DB

use anyhow::{Context, Result};
use chrono::{Duration, Utc};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use rand::Rng;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::config::JwtConfig;

/// Generate a random identifier string (32 hex characters)
fn generate_random_id() -> String {
    let mut rng = rand::thread_rng();
    let bytes: [u8; 16] = rng.gen();
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
    /// Token type
    pub token_type: String,
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
            token_type: "access".to_string(),
            exp: exp.timestamp(),
            iat: now.timestamp(),
        };

        encode(&Header::default(), &claims, &self.encoding_key)
            .context("Failed to encode access token")
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

    /// Validate and decode an access token
    pub fn validate_access_token(&self, token: &str) -> Result<AccessTokenClaims> {
        let mut validation = Validation::default();
        validation.validate_exp = true;

        let token_data = decode::<AccessTokenClaims>(token, &self.decoding_key, &validation)
            .context("Invalid access token")?;

        if token_data.claims.token_type != "access" {
            anyhow::bail!("Invalid token type");
        }

        Ok(token_data.claims)
    }

    /// Validate and decode a refresh token
    pub fn validate_refresh_token(&self, token: &str) -> Result<RefreshTokenClaims> {
        let mut validation = Validation::default();
        validation.validate_exp = true;

        let token_data = decode::<RefreshTokenClaims>(token, &self.decoding_key, &validation)
            .context("Invalid refresh token")?;

        if token_data.claims.token_type != "refresh" {
            anyhow::bail!("Invalid token type");
        }

        Ok(token_data.claims)
    }

    /// Get access token lifetime in seconds
    #[allow(dead_code)]
    pub fn access_token_lifetime_secs(&self) -> i64 {
        self.config.access_token_lifetime.as_secs() as i64
    }

    /// Get refresh token lifetime in seconds
    pub fn refresh_token_lifetime_secs(&self) -> i64 {
        self.config.refresh_token_lifetime.as_secs() as i64
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
