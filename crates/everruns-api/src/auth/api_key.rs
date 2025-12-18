// API Key service for programmatic API access
// Decision: API keys are prefixed with "evr_" for identification
// Decision: Full key is shown only once at creation, stored hashed in DB

use chrono::{DateTime, Utc};
use rand::Rng;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// API key prefix for identification
pub const API_KEY_PREFIX: &str = "evr_";
const API_KEY_LENGTH: usize = 32; // 32 random bytes = 64 hex chars

/// Generated API key (full key shown only at creation)
#[derive(Debug)]
pub struct GeneratedApiKey {
    /// Full API key (evr_<random>)
    pub key: String,
    /// SHA-256 hash for database storage
    pub key_hash: String,
    /// Prefix for display (e.g., "evr_abc1...")
    pub key_prefix: String,
}

/// Generate a new API key
pub fn generate_api_key() -> GeneratedApiKey {
    // Generate random bytes
    let mut rng = rand::thread_rng();
    let random_bytes: Vec<u8> = (0..API_KEY_LENGTH).map(|_| rng.gen()).collect();
    let random_hex = hex::encode(&random_bytes);

    // Full key with prefix
    let key = format!("{}{}", API_KEY_PREFIX, random_hex);

    // Hash for storage
    let key_hash = hash_api_key(&key);

    // Prefix for display (first 8 chars after prefix)
    let key_prefix = format!("{}{}...", API_KEY_PREFIX, &random_hex[..8]);

    GeneratedApiKey {
        key,
        key_hash,
        key_prefix,
    }
}

/// Hash an API key for database storage/lookup
pub fn hash_api_key(key: &str) -> String {
    let hash = Sha256::digest(key.as_bytes());
    hex::encode(hash)
}

/// Validate API key format
pub fn is_valid_api_key_format(key: &str) -> bool {
    if !key.starts_with(API_KEY_PREFIX) {
        return false;
    }

    let key_part = &key[API_KEY_PREFIX.len()..];

    // Check length (should be 64 hex chars for 32 bytes)
    if key_part.len() != API_KEY_LENGTH * 2 {
        return false;
    }

    // Check all characters are hex
    key_part.chars().all(|c| c.is_ascii_hexdigit())
}

/// API key with user info (for validation responses)
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct ValidatedApiKey {
    pub key_id: Uuid,
    pub user_id: Uuid,
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

impl ValidatedApiKey {
    /// Check if the API key is expired
    pub fn is_expired(&self) -> bool {
        if let Some(expires_at) = self.expires_at {
            expires_at < Utc::now()
        } else {
            false
        }
    }

    /// Check if the API key has a specific scope
    #[allow(dead_code)]
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == "*" || s == scope)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_api_key() {
        let key = generate_api_key();

        // Check prefix
        assert!(key.key.starts_with(API_KEY_PREFIX));

        // Check format is valid
        assert!(is_valid_api_key_format(&key.key));

        // Check hash is consistent
        assert_eq!(key.key_hash, hash_api_key(&key.key));

        // Check prefix format
        assert!(key.key_prefix.starts_with(API_KEY_PREFIX));
        assert!(key.key_prefix.ends_with("..."));
    }

    #[test]
    fn test_different_keys() {
        let key1 = generate_api_key();
        let key2 = generate_api_key();

        // Each key should be unique
        assert_ne!(key1.key, key2.key);
        assert_ne!(key1.key_hash, key2.key_hash);
    }

    #[test]
    fn test_is_valid_api_key_format() {
        // Valid key
        let key = generate_api_key();
        assert!(is_valid_api_key_format(&key.key));

        // Invalid: wrong prefix
        assert!(!is_valid_api_key_format(
            "sk_1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        ));

        // Invalid: too short
        assert!(!is_valid_api_key_format("evr_1234"));

        // Invalid: non-hex characters
        assert!(!is_valid_api_key_format(
            "evr_gggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggggg"
        ));

        // Invalid: no prefix
        assert!(!is_valid_api_key_format(
            "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
        ));
    }

    #[test]
    fn test_hash_consistency() {
        let key = "evr_1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef";
        let hash1 = hash_api_key(key);
        let hash2 = hash_api_key(key);

        assert_eq!(hash1, hash2);
    }

    #[test]
    fn test_validated_api_key_expiry() {
        use chrono::Duration;

        // Non-expired key
        let key = ValidatedApiKey {
            key_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: Some(Utc::now() + Duration::days(1)),
        };
        assert!(!key.is_expired());

        // Expired key
        let expired_key = ValidatedApiKey {
            key_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: Some(Utc::now() - Duration::days(1)),
        };
        assert!(expired_key.is_expired());

        // No expiry
        let no_expiry_key = ValidatedApiKey {
            key_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: None,
        };
        assert!(!no_expiry_key.is_expired());
    }

    #[test]
    fn test_validated_api_key_scopes() {
        let key = ValidatedApiKey {
            key_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "test".to_string(),
            scopes: vec!["read".to_string(), "write".to_string()],
            expires_at: None,
        };

        assert!(key.has_scope("read"));
        assert!(key.has_scope("write"));
        assert!(!key.has_scope("admin"));

        // Wildcard scope
        let admin_key = ValidatedApiKey {
            key_id: Uuid::nil(),
            user_id: Uuid::nil(),
            name: "admin".to_string(),
            scopes: vec!["*".to_string()],
            expires_at: None,
        };

        assert!(admin_key.has_scope("read"));
        assert!(admin_key.has_scope("write"));
        assert!(admin_key.has_scope("admin"));
        assert!(admin_key.has_scope("anything"));
    }
}
