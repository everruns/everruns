//! Standard Webhooks signing, shared by both halves of MCP Events (EVE-1121).
//!
//! Outbound (`domains::mcp_servers::events`) signs the deliveries Everruns sends to MCP
//! clients; inbound (`domains::agent_triggers::mcp_event`) verifies the
//! deliveries other MCP servers send to an agent's trigger. Both use the wire
//! contract: a `whsec_` secret
//! of 24 to 64 key bytes, `webhook-id` / `webhook-timestamp` /
//! `webhook-signature` headers, and `v1,` + base64 HMAC-SHA256 over
//! `"{id}.{timestamp}.{body}"`.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use sha2::Sha256;

/// The webhook body cap from the MCP Events spec.
pub const MAX_BODY_BYTES: usize = 256 * 1024;
/// How far a `webhook-timestamp` may be from now, either way, before a
/// delivery is rejected as stale (the Standard Webhooks recommendation).
pub const TIMESTAMP_TOLERANCE_SECS: i64 = 5 * 60;

pub const HEADER_ID: &str = "webhook-id";
pub const HEADER_TIMESTAMP: &str = "webhook-timestamp";
pub const HEADER_SIGNATURE: &str = "webhook-signature";
pub const HEADER_SUBSCRIPTION_ID: &str = "x-mcp-subscription-id";

/// Bytes of key material in a secret Everruns generates.
const GENERATED_KEY_BYTES: usize = 32;

type HmacSha256 = Hmac<Sha256>;

/// `whsec_` + base64 of 24 to 64 key bytes.
pub fn decode_secret(secret: &str) -> Result<Vec<u8>, &'static str> {
    let encoded = secret
        .strip_prefix("whsec_")
        .ok_or("secret must start with whsec_")?;
    let key = BASE64
        .decode(encoded)
        .map_err(|_| "secret must be base64 after whsec_")?;
    if !(24..=64).contains(&key.len()) {
        return Err("secret must decode to 24 to 64 bytes");
    }
    Ok(key)
}

/// A fresh `whsec_` secret with 32 random key bytes.
pub fn generate_secret() -> String {
    let mut rng = rand::rng();
    let key: [u8; GENERATED_KEY_BYTES] = rng.random();
    format!("whsec_{}", BASE64.encode(key))
}

/// Standard Webhooks signature: `v1,` + base64(HMAC-SHA256(key, "{id}.{ts}.{body}")).
pub fn sign(key: &[u8], message_id: &str, timestamp: &str, body: &[u8]) -> String {
    // HMAC accepts keys of any length, so this never takes the else branch.
    let Ok(mut mac) = HmacSha256::new_from_slice(key) else {
        return String::new();
    };
    mac.update(message_id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.as_bytes());
    mac.update(b".");
    mac.update(body);
    format!("v1,{}", BASE64.encode(mac.finalize().into_bytes()))
}

/// Whether any space-separated candidate in a `webhook-signature` header is
/// the signature `key` makes over this message. Constant-time per candidate.
pub fn signature_matches(
    key: &[u8],
    message_id: &str,
    timestamp: &str,
    body: &[u8],
    header: &str,
) -> bool {
    let expected = sign(key, message_id, timestamp, body);
    header.split(' ').any(|candidate| {
        crate::security::constant_time_eq(candidate.as_bytes(), expected.as_bytes())
    })
}

/// Why a received delivery failed verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyError {
    /// A required header is missing or not valid text.
    MissingHeader(&'static str),
    /// `webhook-timestamp` is not a unix timestamp in seconds.
    BadTimestamp,
    /// `webhook-timestamp` is outside the tolerance window.
    StaleTimestamp,
    /// No signature candidate matches.
    BadSignature,
}

impl std::fmt::Display for VerifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingHeader(name) => write!(f, "missing {name} header"),
            Self::BadTimestamp => write!(f, "invalid webhook-timestamp"),
            Self::StaleTimestamp => write!(f, "webhook-timestamp outside the tolerance window"),
            Self::BadSignature => write!(f, "invalid webhook-signature"),
        }
    }
}

/// The signing headers of a received delivery.
#[derive(Debug, Clone, Copy)]
pub struct SignedHeaders<'a> {
    pub id: Option<&'a str>,
    pub timestamp: Option<&'a str>,
    pub signature: Option<&'a str>,
}

/// Verify a received delivery: headers present, timestamp within
/// [`TIMESTAMP_TOLERANCE_SECS`] of `now` (unix seconds), and a matching
/// signature. Returns the `webhook-id`, the replay key.
pub fn verify<'a>(
    key: &[u8],
    headers: SignedHeaders<'a>,
    body: &[u8],
    now: i64,
) -> Result<&'a str, VerifyError> {
    let id = headers
        .id
        .filter(|id| !id.is_empty())
        .ok_or(VerifyError::MissingHeader(HEADER_ID))?;
    let timestamp = headers
        .timestamp
        .ok_or(VerifyError::MissingHeader(HEADER_TIMESTAMP))?;
    let signature = headers
        .signature
        .ok_or(VerifyError::MissingHeader(HEADER_SIGNATURE))?;
    let seconds: i64 = timestamp
        .trim()
        .parse()
        .map_err(|_| VerifyError::BadTimestamp)?;
    if (now - seconds).abs() > TIMESTAMP_TOLERANCE_SECS {
        return Err(VerifyError::StaleTimestamp);
    }
    if !signature_matches(key, id, timestamp, body, signature) {
        return Err(VerifyError::BadSignature);
    }
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secret() -> String {
        format!("whsec_{}", BASE64.encode([7u8; 32]))
    }

    #[test]
    fn secret_must_be_prefixed_base64_of_24_to_64_bytes() {
        assert!(decode_secret(&secret()).is_ok());
        assert!(decode_secret(&BASE64.encode([7u8; 32])).is_err());
        assert!(decode_secret("whsec_not base64!").is_err());
        assert!(decode_secret(&format!("whsec_{}", BASE64.encode([7u8; 16]))).is_err());
        assert!(decode_secret(&format!("whsec_{}", BASE64.encode([7u8; 65]))).is_err());
    }

    #[test]
    fn generated_secrets_are_valid_and_distinct() {
        let a = generate_secret();
        let b = generate_secret();
        assert_ne!(a, b);
        assert_eq!(decode_secret(&a).map(|key| key.len()), Ok(32));
    }

    #[test]
    fn signature_follows_standard_webhooks() {
        // Standard Webhooks' published test vector.
        let key = decode_secret("whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw").unwrap_or_default();
        let body = br#"{"test": 2432232314}"#;
        let signature = sign(&key, "msg_p5jXN8AQM9LWM0D4loKWxJek", "1614265330", body);
        assert_eq!(signature, "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=");
        assert!(signature_matches(
            &key,
            "msg_p5jXN8AQM9LWM0D4loKWxJek",
            "1614265330",
            body,
            &format!("v1,bogus {signature}")
        ));
    }

    #[test]
    fn verify_checks_headers_window_and_signature() {
        let key = [9u8; 32];
        let body = br#"{"name":"x"}"#;
        let now = 1_700_000_000;
        let ts = now.to_string();
        let signature = sign(&key, "msg_1", &ts, body);
        let headers = SignedHeaders {
            id: Some("msg_1"),
            timestamp: Some(&ts),
            signature: Some(&signature),
        };
        assert_eq!(verify(&key, headers, body, now), Ok("msg_1"));
        assert_eq!(
            verify(&key, headers, body, now + TIMESTAMP_TOLERANCE_SECS),
            Ok("msg_1")
        );
        assert_eq!(
            verify(&key, headers, body, now + TIMESTAMP_TOLERANCE_SECS + 1),
            Err(VerifyError::StaleTimestamp)
        );
        assert_eq!(
            verify(&key, headers, b"{}", now),
            Err(VerifyError::BadSignature)
        );
        assert_eq!(
            verify(&[1u8; 32], headers, body, now),
            Err(VerifyError::BadSignature)
        );
        let missing = SignedHeaders {
            signature: None,
            ..headers
        };
        assert_eq!(
            verify(&key, missing, body, now),
            Err(VerifyError::MissingHeader(HEADER_SIGNATURE))
        );
        let garbage = SignedHeaders {
            timestamp: Some("yesterday"),
            ..headers
        };
        assert_eq!(
            verify(&key, garbage, body, now),
            Err(VerifyError::BadTimestamp)
        );
    }
}
