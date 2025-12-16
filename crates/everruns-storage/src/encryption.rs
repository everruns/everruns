// Envelope encryption with key rotation support for sensitive database fields.
// Uses AES-256-GCM with per-value DEKs wrapped by versioned KEKs.
// See specs/encryption.md for full specification.

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

const NONCE_SIZE: usize = 12;
const KEY_SIZE: usize = 32;
const DEK_SIZE: usize = 32;
const PAYLOAD_VERSION: u8 = 1;
const ALGORITHM: &str = "AES-256-GCM";

/// Encrypted payload stored in database
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedPayload {
    /// Payload format version
    pub version: u8,
    /// Encryption algorithm identifier
    pub alg: String,
    /// Key ID of the KEK used to wrap the DEK
    pub key_id: String,
    /// Base64-encoded wrapped DEK
    pub dek_wrapped: String,
    /// Base64-encoded nonce
    pub nonce: String,
    /// Base64-encoded ciphertext (includes auth tag)
    pub ciphertext: String,
}

/// Key Encryption Key with identifier
#[derive(Clone)]
struct VersionedKey {
    id: String,
    cipher: Aes256Gcm,
}

/// Encryption service supporting envelope encryption with key rotation.
/// Thread-safe and designed for concurrent use.
#[derive(Clone)]
pub struct EncryptionService {
    /// Primary key for new encryptions
    primary_key: Arc<VersionedKey>,
    /// Map of all available keys (including primary) for decryption
    keys: Arc<HashMap<String, Aes256Gcm>>,
}

impl EncryptionService {
    /// Create from versioned key strings in format "key_id:base64_key".
    /// The first key is used for new encryptions, all keys are available for decryption.
    pub fn new(primary_key: &str, previous_keys: &[&str]) -> Result<Self> {
        let (primary_id, primary_cipher) = Self::parse_versioned_key(primary_key)?;

        let mut keys = HashMap::new();
        keys.insert(primary_id.clone(), primary_cipher.clone());

        for key_str in previous_keys {
            let (id, cipher) = Self::parse_versioned_key(key_str)?;
            if keys.contains_key(&id) {
                anyhow::bail!("Duplicate key ID: {}", id);
            }
            keys.insert(id, cipher);
        }

        Ok(Self {
            primary_key: Arc::new(VersionedKey {
                id: primary_id,
                cipher: primary_cipher,
            }),
            keys: Arc::new(keys),
        })
    }

    /// Create from environment variables.
    /// - SECRETS_ENCRYPTION_KEY: Primary key (required)
    /// - SECRETS_ENCRYPTION_KEY_PREVIOUS: Previous key for rotation (optional)
    pub fn from_env() -> Result<Self> {
        let primary = std::env::var("SECRETS_ENCRYPTION_KEY")
            .context("SECRETS_ENCRYPTION_KEY environment variable not set")?;

        let previous_keys: Vec<String> = std::env::var("SECRETS_ENCRYPTION_KEY_PREVIOUS")
            .ok()
            .into_iter()
            .collect();

        let previous_refs: Vec<&str> = previous_keys.iter().map(|s| s.as_str()).collect();

        Self::new(&primary, &previous_refs)
    }

    /// Parse a versioned key string in format "key_id:base64_key"
    fn parse_versioned_key(key_str: &str) -> Result<(String, Aes256Gcm)> {
        let parts: Vec<&str> = key_str.splitn(2, ':').collect();
        if parts.len() != 2 {
            anyhow::bail!(
                "Invalid key format. Expected 'key_id:base64_key', got: {}",
                if key_str.len() > 20 {
                    format!("{}...", &key_str[..20])
                } else {
                    key_str.to_string()
                }
            );
        }

        let key_id = parts[0].to_string();
        let key_bytes = BASE64
            .decode(parts[1])
            .context("Failed to decode key from base64")?;

        if key_bytes.len() != KEY_SIZE {
            anyhow::bail!(
                "Key must be {} bytes, got {} bytes for key_id '{}'",
                KEY_SIZE,
                key_bytes.len(),
                key_id
            );
        }

        let cipher = Aes256Gcm::new_from_slice(&key_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to create cipher for key '{}': {}", key_id, e))?;

        Ok((key_id, cipher))
    }

    /// Encrypt plaintext using envelope encryption.
    /// Returns JSON-encoded EncryptedPayload.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>> {
        // Generate random DEK
        let mut dek_bytes = [0u8; DEK_SIZE];
        rand::thread_rng().fill_bytes(&mut dek_bytes);

        // Wrap DEK with primary KEK
        let mut dek_nonce_bytes = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut dek_nonce_bytes);
        let dek_nonce = Nonce::from_slice(&dek_nonce_bytes);

        let wrapped_dek = self
            .primary_key
            .cipher
            .encrypt(dek_nonce, dek_bytes.as_ref())
            .map_err(|e| anyhow::anyhow!("Failed to wrap DEK: {}", e))?;

        // Encrypt plaintext with DEK
        let dek_cipher = Aes256Gcm::new_from_slice(&dek_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to create DEK cipher: {}", e))?;

        let mut data_nonce_bytes = [0u8; NONCE_SIZE];
        rand::thread_rng().fill_bytes(&mut data_nonce_bytes);
        let data_nonce = Nonce::from_slice(&data_nonce_bytes);

        let ciphertext = dek_cipher
            .encrypt(data_nonce, plaintext)
            .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

        // Build payload (combine DEK nonce + wrapped DEK for dek_wrapped field)
        let mut dek_wrapped_bytes = Vec::with_capacity(NONCE_SIZE + wrapped_dek.len());
        dek_wrapped_bytes.extend_from_slice(&dek_nonce_bytes);
        dek_wrapped_bytes.extend_from_slice(&wrapped_dek);

        let payload = EncryptedPayload {
            version: PAYLOAD_VERSION,
            alg: ALGORITHM.to_string(),
            key_id: self.primary_key.id.clone(),
            dek_wrapped: BASE64.encode(&dek_wrapped_bytes),
            nonce: BASE64.encode(data_nonce_bytes),
            ciphertext: BASE64.encode(&ciphertext),
        };

        serde_json::to_vec(&payload).context("Failed to serialize encrypted payload")
    }

    /// Decrypt data using the key referenced in the payload.
    pub fn decrypt(&self, data: &[u8]) -> Result<Vec<u8>> {
        let payload: EncryptedPayload =
            serde_json::from_slice(data).context("Failed to parse encrypted payload")?;

        if payload.version != PAYLOAD_VERSION {
            anyhow::bail!(
                "Unsupported payload version: {} (expected {})",
                payload.version,
                PAYLOAD_VERSION
            );
        }

        if payload.alg != ALGORITHM {
            anyhow::bail!(
                "Unsupported algorithm: {} (expected {})",
                payload.alg,
                ALGORITHM
            );
        }

        // Get the KEK for this payload
        let kek_cipher = self.keys.get(&payload.key_id).ok_or_else(|| {
            anyhow::anyhow!(
                "Unknown key_id '{}'. Available keys: {:?}",
                payload.key_id,
                self.keys.keys().collect::<Vec<_>>()
            )
        })?;

        // Decode wrapped DEK (nonce + wrapped_dek)
        let dek_wrapped_bytes = BASE64
            .decode(&payload.dek_wrapped)
            .context("Failed to decode wrapped DEK")?;

        if dek_wrapped_bytes.len() < NONCE_SIZE {
            anyhow::bail!("Wrapped DEK too short");
        }

        let (dek_nonce_bytes, wrapped_dek) = dek_wrapped_bytes.split_at(NONCE_SIZE);
        let dek_nonce = Nonce::from_slice(dek_nonce_bytes);

        // Unwrap DEK
        let dek_bytes = kek_cipher
            .decrypt(dek_nonce, wrapped_dek)
            .map_err(|e| anyhow::anyhow!("Failed to unwrap DEK: {}", e))?;

        if dek_bytes.len() != DEK_SIZE {
            anyhow::bail!("Invalid DEK size after unwrap");
        }

        // Decrypt data with DEK
        let dek_cipher = Aes256Gcm::new_from_slice(&dek_bytes)
            .map_err(|e| anyhow::anyhow!("Failed to create DEK cipher: {}", e))?;

        let data_nonce_bytes = BASE64
            .decode(&payload.nonce)
            .context("Failed to decode nonce")?;
        let data_nonce = Nonce::from_slice(&data_nonce_bytes);

        let ciphertext = BASE64
            .decode(&payload.ciphertext)
            .context("Failed to decode ciphertext")?;

        let plaintext = dek_cipher
            .decrypt(data_nonce, ciphertext.as_ref())
            .map_err(|e| anyhow::anyhow!("Decryption failed (data may be corrupted): {}", e))?;

        Ok(plaintext)
    }

    /// Encrypt a string, returns bytes for database storage.
    pub fn encrypt_string(&self, plaintext: &str) -> Result<Vec<u8>> {
        self.encrypt(plaintext.as_bytes())
    }

    /// Decrypt bytes to a string.
    pub fn decrypt_to_string(&self, data: &[u8]) -> Result<String> {
        let plaintext = self.decrypt(data)?;
        String::from_utf8(plaintext).context("Decrypted data is not valid UTF-8")
    }

    /// Get the key_id from encrypted data without decrypting.
    /// Useful for identifying which records need re-encryption.
    pub fn get_key_id(data: &[u8]) -> Result<String> {
        let payload: EncryptedPayload =
            serde_json::from_slice(data).context("Failed to parse encrypted payload")?;
        Ok(payload.key_id)
    }

    /// Check if data is encrypted with the current primary key.
    pub fn is_current_key(&self, data: &[u8]) -> Result<bool> {
        let key_id = Self::get_key_id(data)?;
        Ok(key_id == self.primary_key.id)
    }

    /// Re-encrypt data with the current primary key.
    /// Returns None if already encrypted with current key.
    pub fn reencrypt(&self, data: &[u8]) -> Result<Option<Vec<u8>>> {
        if self.is_current_key(data)? {
            return Ok(None);
        }

        let plaintext = self.decrypt(data)?;
        let new_ciphertext = self.encrypt(&plaintext)?;
        Ok(Some(new_ciphertext))
    }

    /// Get the primary key ID.
    pub fn primary_key_id(&self) -> &str {
        &self.primary_key.id
    }

    /// Get all available key IDs.
    pub fn available_key_ids(&self) -> Vec<&str> {
        self.keys.keys().map(|s| s.as_str()).collect()
    }
}

/// Generate a new random encryption key in versioned format.
/// Returns format: "key_id:base64_key"
pub fn generate_encryption_key(key_id: &str) -> String {
    let mut key = [0u8; KEY_SIZE];
    rand::thread_rng().fill_bytes(&mut key);
    format!("{}:{}", key_id, BASE64.encode(key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_key(id: &str) -> String {
        generate_encryption_key(id)
    }

    #[test]
    fn test_encrypt_decrypt() {
        let key = test_key("kek-v1");
        let service = EncryptionService::new(&key, &[]).unwrap();

        let plaintext = "sk-test-api-key-12345";
        let encrypted = service.encrypt_string(plaintext).unwrap();
        let decrypted = service.decrypt_to_string(&encrypted).unwrap();

        assert_eq!(plaintext, decrypted);
    }

    #[test]
    fn test_different_ciphertext() {
        let key = test_key("kek-v1");
        let service = EncryptionService::new(&key, &[]).unwrap();

        let plaintext = "same-plaintext";
        let encrypted1 = service.encrypt_string(plaintext).unwrap();
        let encrypted2 = service.encrypt_string(plaintext).unwrap();

        // Same plaintext should produce different ciphertext (different DEKs and nonces)
        assert_ne!(encrypted1, encrypted2);

        // But both should decrypt to the same plaintext
        assert_eq!(plaintext, service.decrypt_to_string(&encrypted1).unwrap());
        assert_eq!(plaintext, service.decrypt_to_string(&encrypted2).unwrap());
    }

    #[test]
    fn test_key_rotation() {
        let key_v1 = test_key("kek-v1");
        let key_v2 = test_key("kek-v2");

        // Encrypt with v1
        let service_v1 = EncryptionService::new(&key_v1, &[]).unwrap();
        let plaintext = "secret-data";
        let encrypted_v1 = service_v1.encrypt_string(plaintext).unwrap();

        // Create service with v2 as primary and v1 as previous
        let service_v2 = EncryptionService::new(&key_v2, &[&key_v1]).unwrap();

        // Should still decrypt v1 data
        let decrypted = service_v2.decrypt_to_string(&encrypted_v1).unwrap();
        assert_eq!(plaintext, decrypted);

        // New encryption should use v2
        let encrypted_v2 = service_v2.encrypt_string(plaintext).unwrap();
        assert_eq!(
            EncryptionService::get_key_id(&encrypted_v2).unwrap(),
            "kek-v2"
        );

        // Old encryption should show v1
        assert_eq!(
            EncryptionService::get_key_id(&encrypted_v1).unwrap(),
            "kek-v1"
        );
    }

    #[test]
    fn test_reencrypt() {
        let key_v1 = test_key("kek-v1");
        let key_v2 = test_key("kek-v2");

        // Encrypt with v1
        let service_v1 = EncryptionService::new(&key_v1, &[]).unwrap();
        let plaintext = "secret-data";
        let encrypted_v1 = service_v1.encrypt_string(plaintext).unwrap();

        // Create service with v2 as primary
        let service_v2 = EncryptionService::new(&key_v2, &[&key_v1]).unwrap();

        // Re-encrypt to v2
        let encrypted_v2 = service_v2.reencrypt(&encrypted_v1).unwrap().unwrap();
        assert_eq!(
            EncryptionService::get_key_id(&encrypted_v2).unwrap(),
            "kek-v2"
        );

        // Verify decryption
        let decrypted = service_v2.decrypt_to_string(&encrypted_v2).unwrap();
        assert_eq!(plaintext, decrypted);

        // Re-encrypt again should return None (already current)
        assert!(service_v2.reencrypt(&encrypted_v2).unwrap().is_none());
    }

    #[test]
    fn test_is_current_key() {
        let key_v1 = test_key("kek-v1");
        let key_v2 = test_key("kek-v2");

        let service_v1 = EncryptionService::new(&key_v1, &[]).unwrap();
        let encrypted_v1 = service_v1.encrypt_string("test").unwrap();

        let service_v2 = EncryptionService::new(&key_v2, &[&key_v1]).unwrap();
        let encrypted_v2 = service_v2.encrypt_string("test").unwrap();

        // v1 data is not current in v2 service
        assert!(!service_v2.is_current_key(&encrypted_v1).unwrap());
        // v2 data is current
        assert!(service_v2.is_current_key(&encrypted_v2).unwrap());
    }

    #[test]
    fn test_invalid_key_format() {
        // Missing colon
        let result = EncryptionService::new("no-colon-here", &[]);
        assert!(result.is_err());

        // Invalid base64
        let result = EncryptionService::new("kek-v1:not-valid-base64!!!", &[]);
        assert!(result.is_err());

        // Wrong key length
        let short_key = format!("kek-v1:{}", BASE64.encode([0u8; 16]));
        let result = EncryptionService::new(&short_key, &[]);
        assert!(result.is_err());
    }

    #[test]
    fn test_duplicate_key_id() {
        let key1 = test_key("kek-v1");
        let key2 = test_key("kek-v1"); // Same ID, different key

        let result = EncryptionService::new(&key1, &[&key2]);
        assert!(result.is_err());
    }

    #[test]
    fn test_unknown_key_id() {
        let key_v1 = test_key("kek-v1");
        let key_v2 = test_key("kek-v2");

        // Encrypt with v1
        let service_v1 = EncryptionService::new(&key_v1, &[]).unwrap();
        let encrypted = service_v1.encrypt_string("test").unwrap();

        // Try to decrypt with only v2 (v1 not available)
        let service_v2 = EncryptionService::new(&key_v2, &[]).unwrap();
        let result = service_v2.decrypt(&encrypted);
        assert!(result.is_err());
    }

    #[test]
    fn test_payload_format() {
        let key = test_key("kek-v1");
        let service = EncryptionService::new(&key, &[]).unwrap();

        let encrypted = service.encrypt_string("test").unwrap();
        let payload: EncryptedPayload = serde_json::from_slice(&encrypted).unwrap();

        assert_eq!(payload.version, 1);
        assert_eq!(payload.alg, "AES-256-GCM");
        assert_eq!(payload.key_id, "kek-v1");
        assert!(!payload.dek_wrapped.is_empty());
        assert!(!payload.nonce.is_empty());
        assert!(!payload.ciphertext.is_empty());
    }

    #[test]
    fn test_generate_key() {
        let key1 = generate_encryption_key("kek-v1");
        let key2 = generate_encryption_key("kek-v1");

        // Keys should be different
        assert_ne!(key1, key2);

        // Both should be valid
        assert!(EncryptionService::new(&key1, &[]).is_ok());
        assert!(EncryptionService::new(&key2, &[]).is_ok());
    }
}
