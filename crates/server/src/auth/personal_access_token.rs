// Personal access token service for programmatic API access.
// Decision: personal access tokens are user-scoped (tied to a person, inheriting
// all of that user's org access), never org-scoped.
// Decision: tokens are prefixed with "evr_pat_" for identification — the prefix
// itself signals "personal access token", distinguishing them from JWTs and from
// unrelated LLM-provider/integration "api keys".
// Decision: full token is shown only once at creation, stored hashed in DB.

use chrono::{DateTime, Utc};
use rand::RngExt;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Personal access token prefix for identification
pub const PAT_PREFIX: &str = "evr_pat_";
const PAT_LENGTH: usize = 32; // 32 random bytes = 64 hex chars

/// Generated personal access token (full token shown only at creation)
#[derive(Debug)]
pub struct GeneratedPersonalAccessToken {
    /// Full token (`evr_pat_<random>`)
    pub token: String,
    /// SHA-256 hash for database storage
    pub token_hash: String,
    /// Prefix for display (e.g., "evr_pat_abc1...")
    pub token_prefix: String,
}

/// Generate a new personal access token
pub fn generate_personal_access_token() -> GeneratedPersonalAccessToken {
    // Generate random bytes
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..PAT_LENGTH).map(|_| rng.random()).collect();
    let random_hex = hex::encode(&random_bytes);

    // Full token with prefix
    let token = format!("{}{}", PAT_PREFIX, random_hex);

    // Hash for storage
    let token_hash = hash_personal_access_token(&token);

    // Prefix for display (first 8 chars after prefix)
    let token_prefix = format!("{}{}...", PAT_PREFIX, &random_hex[..8]);

    GeneratedPersonalAccessToken {
        token,
        token_hash,
        token_prefix,
    }
}

/// Hash a personal access token for database storage/lookup
pub fn hash_personal_access_token(token: &str) -> String {
    let hash = Sha256::digest(token.as_bytes());
    hex::encode(hash)
}

/// Validate personal access token format
pub fn is_valid_personal_access_token_format(token: &str) -> bool {
    if !token.starts_with(PAT_PREFIX) {
        return false;
    }

    let token_part = &token[PAT_PREFIX.len()..];

    // Check length (should be 64 hex chars for 32 bytes)
    if token_part.len() != PAT_LENGTH * 2 {
        return false;
    }

    // Check all characters are hex
    token_part.chars().all(|c| c.is_ascii_hexdigit())
}

/// Personal access token with user info (for validation responses)
#[derive(Debug, Clone)]
pub struct ValidatedPersonalAccessToken {
    pub token_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl ValidatedPersonalAccessToken {
    /// Check if the token is expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            expires_at < Utc::now()
        } else {
            false
        }
    }

    /// Check if the token has a specific scope
    #[allow(dead_code)]
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == "*" || s == scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_personal_access_token() {
        let token = generate_personal_access_token();

        // Check prefix
        assert!(token.token.starts_with(PAT_PREFIX));

        // Check format is valid
        assert!(is_valid_personal_access_token_format(&token.token));

        // Check hash is consistent
        assert_eq!(token.token_hash, hash_personal_access_token(&token.token));

        // Check prefix format
        assert!(token.token_prefix.starts_with(PAT_PREFIX));
        assert!(token.token_prefix.ends_with("..."));
    }

    #[test]
    fn test_different_tokens() {
        let token1 = generate_personal_access_token();
        let token2 = generate_personal_access_token();

        // Each token should be unique
        assert_ne!(token1.token, token2.token);
        assert_ne!(token1.token_hash, token2.token_hash);
    }

    #[test]
    fn test_is_valid_personal_access_token_format() {
        // Valid token
        let token = generate_personal_access_token();
        assert!(is_valid_personal_access_token_format(&token.token));

        // Invalid: wrong prefix
        assert!(!is_valid_personal_access_token_format(
            "sk_1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        ));

        // Invalid: old evr_ prefix (no longer accepted)
        assert!(!is_valid_personal_access_token_format(
            "evr_1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        ));

        // Invalid: too short
        assert!(!is_valid_personal_access_token_format("evr_pat_1234"));

        // Invalid: non-hex characters
        assert!(!is_valid_personal_access_token_format(
            "evr_pat_gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"
        ));

        // Invalid: no prefix
        assert!(!is_valid_personal_access_token_format(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        ));
    }

    #[test]
    fn token_hash_matches_sha256_known_vectors() {
        for (input, expected) in [
            (
                "",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
        ] {
            assert_eq!(hash_personal_access_token(input), expected);
        }
    }

    #[test]
    fn test_validated_personal_access_token_expiry() {
        use chrono::Duration;

        // Non-expired token
        let token = ValidatedPersonalAccessToken {
            token_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: Some(Utc::now() + Duration::days(1)),
        };
        assert!(!token.is_expired());

        // Expired token
        let expired_token = ValidatedPersonalAccessToken {
            token_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: Some(Utc::now() - Duration::days(1)),
        };
        assert!(expired_token.is_expired());

        // No expiry
        let no_expiry_token = ValidatedPersonalAccessToken {
            token_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: None,
        };
        assert!(!no_expiry_token.is_expired());
    }

    #[test]
    fn test_validated_personal_access_token_scopes() {
        let token = ValidatedPersonalAccessToken {
            token_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            expires_at: None,
        };

        assert!(token.has_scope("read"));
        assert!(token.has_scope("write"));
        assert!(!token.has_scope("admin"));

        // Wildcard scope
        let admin_token = ValidatedPersonalAccessToken {
            token_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "admin".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: None,
        };

        assert!(admin_token.has_scope("read"));
        assert!(admin_token.has_scope("write"));
        assert!(admin_token.has_scope("admin"));
        assert!(admin_token.has_scope("anything"));
    }
}
