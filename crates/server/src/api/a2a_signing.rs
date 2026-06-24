// A2A request-replay protection (TM-A2A-010).
//
// Decision: Slack-style HMAC signing scheme — chosen because it is
//   deterministic, requires no extra credential issuance flow, matches the
//   precedent in `slack_events.rs`, and does not depend on token endpoints
//   external A2A clients may not implement. JWT bearer was the alternative
//   per the issue; Slack-style was selected for parity with existing code
//   and zero-trust on the client side (no token caching).
// Decision: Signing is **opt-in per channel** via
//   `A2aChannelConfig::signing_secret`. Channels without a secret keep the
//   existing API-key-only behavior so existing deployments do not break.
// Decision: Signature base string is Slack-derived but target-bound:
//   `v0:{timestamp}:{channel_scope}:{body}` where `channel_scope` is
//   `{app_id}:{channel_id}`. Slack signs only `v0:{timestamp}:{body}`; we
//   add the channel scope because operators who reuse the same
//   `signing_secret` across multiple A2A channels would otherwise be
//   vulnerable to cross-channel replay (a captured request for channel A
//   could be redirected to channel B since the replay store is keyed
//   per-channel — the dedup would not catch it). Encoded with HMAC-SHA256,
//   hex-encoded, prefixed `v0=`. Timestamp window is 300 seconds (5
//   minutes) absolute, mirroring Slack and the Slack ingress already in
//   this codebase, which gives a forgiving but bounded replay window for
//   clock skew.
// Decision: Replay defence-in-depth: in addition to the timestamp window,
//   the **signature itself** is recorded as a single-use nonce with the
//   same TTL. A captured request that survives the timestamp check will
//   still be rejected within that window because its (deterministic)
//   signature is already in the store. The signature is unique per
//   `(timestamp, channel_scope, body, secret)`. Two legitimate requests
//   with byte-identical bodies sent in the same unix second to the same
//   channel produce the same HMAC and the second one will be rejected as
//   a replay — this mirrors the Slack precedent and is acceptable in
//   practice because JSON-RPC clients already vary `id` per request,
//   which makes the body distinct. Clients that may legitimately repeat
//   identical bodies (e.g. unkeyed notifications) must include a
//   per-request token in the body or bump the timestamp by at least one
//   second between retries.
// Decision: Two backends mirror the channel rate limiter — in-memory
//   `HashMap` for single-instance/dev and Valkey `SET ... NX EX` for
//   distributed deployments. The check runs after API key verification
//   so an unauthenticated caller cannot grow the in-memory store and
//   cannot probe channel existence from signing-related signals.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use parking_lot::RwLock;
use sha2::Sha256;

use crate::security::constant_time_eq;
use crate::valkey::ValkeyClient;

type HmacSha256 = Hmac<Sha256>;

pub const A2A_TIMESTAMP_HEADER: &str = "x-everruns-a2a-timestamp";
pub const A2A_SIGNATURE_HEADER: &str = "x-everruns-a2a-signature";

/// 5-minute window — same as the Slack signing scheme. Captured requests
/// older than this are rejected outright; live requests must finish their
/// signing dance within this budget.
pub const A2A_REPLAY_WINDOW_SECS: u64 = 300;

/// Literal version tag used in Slack-style signatures. Held as a constant
/// so the same string appears in the basestring (`v0:{ts}:{body}`) and the
/// signature header (`v0={hex_hmac}`); the HMAC digest itself is
/// hex-encoded, not base64.
const SIG_VERSION: &str = "v0";

/// Result of verifying an A2A signed request. The variants are deliberately
/// granular so the caller can log a precise reason without leaking it back
/// to the client (the HTTP response stays a generic 401).
#[derive(Debug, PartialEq, Eq)]
pub enum SignatureCheckError {
    /// Required header missing.
    MissingHeader(&'static str),
    /// Timestamp header was not parseable as a unix-seconds integer.
    InvalidTimestamp,
    /// Timestamp is outside the allowed window (too old or too far in the
    /// future). Both directions are rejected — clock skew on the client
    /// side cannot bypass replay protection by stamping the future.
    StaleTimestamp,
    /// HMAC mismatch.
    SignatureMismatch,
    /// The exact signature has already been recorded inside the window —
    /// either an in-flight retry on top of an already-accepted request or
    /// a captured replay.
    Replay,
}

impl SignatureCheckError {
    /// Operator-facing reason string for log lines. Kept opaque enough not
    /// to be useful to a remote attacker, but specific enough to diagnose
    /// a misconfigured client.
    pub fn as_log_reason(&self) -> &'static str {
        match self {
            SignatureCheckError::MissingHeader(_) => "missing_header",
            SignatureCheckError::InvalidTimestamp => "invalid_timestamp",
            SignatureCheckError::StaleTimestamp => "stale_timestamp",
            SignatureCheckError::SignatureMismatch => "signature_mismatch",
            SignatureCheckError::Replay => "replay",
        }
    }
}

/// Verify an A2A request signature against the configured shared secret.
///
/// The expected basestring is `v0:{timestamp}:{channel_scope}:{body}` with
/// each segment separated by a literal colon and no whitespace. The HMAC
/// digest is hex-encoded and prefixed `v0=`. `channel_scope` is the
/// caller-supplied target identifier — for the A2A endpoint this is
/// `{app_id}:{channel_id}` so the same `signing_secret` reused across
/// channels cannot let a captured request be replayed against a
/// different channel target. The body slice must be the **raw request
/// bytes** (not a re-serialized JSON value) so the signature matches
/// what the client produced.
///
/// Returns `Ok(signature_string)` on success — the caller passes that into
/// the nonce store.
pub fn verify_signature(
    timestamp_header: Option<&str>,
    signature_header: Option<&str>,
    channel_scope: &str,
    body: &[u8],
    signing_secret: &str,
    now_secs: i64,
) -> Result<String, SignatureCheckError> {
    let timestamp =
        timestamp_header.ok_or(SignatureCheckError::MissingHeader(A2A_TIMESTAMP_HEADER))?;
    let signature =
        signature_header.ok_or(SignatureCheckError::MissingHeader(A2A_SIGNATURE_HEADER))?;

    let ts: i64 = timestamp
        .parse()
        .map_err(|_| SignatureCheckError::InvalidTimestamp)?;
    // `abs_diff` is overflow-safe even for extreme parseable timestamps
    // (e.g. `i64::MIN`) — those callers should still get a 401 rather than
    // a debug-build panic.
    if now_secs.abs_diff(ts) > A2A_REPLAY_WINDOW_SECS {
        return Err(SignatureCheckError::StaleTimestamp);
    }

    let mut sig_basestring =
        Vec::with_capacity(SIG_VERSION.len() + 3 + 16 + channel_scope.len() + body.len());
    sig_basestring.extend_from_slice(SIG_VERSION.as_bytes());
    sig_basestring.push(b':');
    sig_basestring.extend_from_slice(timestamp.as_bytes());
    sig_basestring.push(b':');
    sig_basestring.extend_from_slice(channel_scope.as_bytes());
    sig_basestring.push(b':');
    sig_basestring.extend_from_slice(body);

    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .map_err(|_| SignatureCheckError::SignatureMismatch)?;
    mac.update(&sig_basestring);
    let expected = format!(
        "{}={}",
        SIG_VERSION,
        hex::encode(mac.finalize().into_bytes())
    );

    if !constant_time_eq(expected.as_bytes(), signature.as_bytes()) {
        return Err(SignatureCheckError::SignatureMismatch);
    }
    Ok(signature.to_string())
}

/// Store of recently-seen signatures keyed by `(scope, signature)` with a
/// fixed TTL equal to the replay window. Two backends — in-memory for
/// single-instance/dev and Valkey for distributed deployments. Picked at
/// construction time and cloned cheaply (Arc-wrapped state).
#[derive(Clone)]
pub struct A2aReplayStore {
    backend: Backend,
}

#[derive(Clone)]
enum Backend {
    InMemory {
        // Key: `{scope}:{signature}`. Value: the `Instant` after which the
        // entry is expired and may be pruned. `parking_lot::RwLock` does
        // not poison so writer-panic does not strand readers.
        cache: Arc<RwLock<HashMap<String, Instant>>>,
    },
    Valkey(ValkeyClient),
}

impl A2aReplayStore {
    pub fn in_memory() -> Self {
        Self {
            backend: Backend::InMemory {
                cache: Arc::new(RwLock::new(HashMap::new())),
            },
        }
    }

    pub fn with_valkey(client: ValkeyClient) -> Self {
        Self {
            backend: Backend::Valkey(client),
        }
    }

    /// Record a signature. Returns `true` if the signature was first-seen
    /// (caller must accept the request) or `false` if it was already
    /// recorded inside the replay window (caller must reject as replay).
    /// Valkey command errors fall open — the same accepted risk as the
    /// rate limiter — and are logged.
    pub async fn try_record(&self, scope: &str, signature: &str) -> bool {
        match &self.backend {
            Backend::InMemory { cache } => {
                let key = format!("{scope}:{signature}");
                let now = Instant::now();
                let ttl = std::time::Duration::from_secs(A2A_REPLAY_WINDOW_SECS);

                // Fast-path: if the entry exists and is not expired, this
                // is a replay. We hold the read lock only long enough to
                // peek; the write path below handles eviction + insert.
                if let Some(expires_at) = cache.read().get(&key).copied()
                    && expires_at > now
                {
                    return false;
                }

                let mut write = cache.write();
                // Re-check under the write lock to avoid a TOCTOU race
                // between two concurrent first-time signatures.
                if let Some(expires_at) = write.get(&key).copied()
                    && expires_at > now
                {
                    return false;
                }
                // Bound the cache growth without paying for an O(n) sweep on
                // every insert. Under steady traffic the cache stays well
                // below the threshold; under bursty load we sweep at most
                // once per ~`PRUNE_THRESHOLD` inserts, amortising the
                // pruning cost. The cap also gives the per-channel rate
                // limiter a chance to reject excess traffic between
                // sweeps.
                const PRUNE_THRESHOLD: usize = 1024;
                if write.len() >= PRUNE_THRESHOLD {
                    write.retain(|_, expires_at| *expires_at > now);
                }
                write.insert(key, now + ttl);
                true
            }
            Backend::Valkey(client) => {
                let key = format!("a2a:replay:{scope}:{signature}");
                // Fail-open mirrors TM-DOS-010 / TM-A2A-013 — same
                // accepted risk profile when the Valkey command itself
                // fails.
                client
                    .try_record_nonce(&key, A2A_REPLAY_WINDOW_SECS)
                    .await
                    .unwrap_or(true)
            }
        }
    }
}

/// Convenience helper: wall-clock time in seconds since the Unix epoch,
/// extracted so tests can pass a fixed value into [`verify_signature`].
pub fn now_unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, ts: i64, channel_scope: &str, body: &[u8]) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("v0:{ts}:{channel_scope}:").as_bytes());
        mac.update(body);
        format!("v0={}", hex::encode(mac.finalize().into_bytes()))
    }

    #[test]
    fn verify_signature_accepts_valid_request() {
        let secret = "topsecret";
        let ts = 1_700_000_000_i64;
        let channel_scope = "app1:ch1";
        let body = br#"{"jsonrpc":"2.0","method":"message/send"}"#;
        let sig = sign(secret, ts, channel_scope, body);

        let result = verify_signature(
            Some(&ts.to_string()),
            Some(&sig),
            channel_scope,
            body,
            secret,
            ts, // now == ts is well within the window
        );
        assert_eq!(result, Ok(sig));
    }

    #[test]
    fn verify_signature_rejects_missing_headers() {
        let secret = "topsecret";
        let body = b"{}";
        assert_eq!(
            verify_signature(None, Some("v0=abc"), "app1:ch1", body, secret, 0),
            Err(SignatureCheckError::MissingHeader(A2A_TIMESTAMP_HEADER)),
        );
        assert_eq!(
            verify_signature(Some("0"), None, "app1:ch1", body, secret, 0),
            Err(SignatureCheckError::MissingHeader(A2A_SIGNATURE_HEADER)),
        );
    }

    #[test]
    fn verify_signature_rejects_stale_timestamp() {
        let secret = "topsecret";
        let ts = 1_700_000_000_i64;
        let body = b"{}";
        let sig = sign(secret, ts, "app1:ch1", body);
        // 6 minutes later
        let now = ts + 360;
        assert_eq!(
            verify_signature(
                Some(&ts.to_string()),
                Some(&sig),
                "app1:ch1",
                body,
                secret,
                now
            ),
            Err(SignatureCheckError::StaleTimestamp),
        );
    }

    #[test]
    fn verify_signature_rejects_future_timestamp() {
        // A client with a wildly fast clock cannot bypass replay protection
        // by stamping the future — the same window applies symmetrically.
        let secret = "topsecret";
        let ts = 1_700_000_000_i64;
        let body = b"{}";
        let sig = sign(secret, ts, "app1:ch1", body);
        let now = ts - 360;
        assert_eq!(
            verify_signature(
                Some(&ts.to_string()),
                Some(&sig),
                "app1:ch1",
                body,
                secret,
                now
            ),
            Err(SignatureCheckError::StaleTimestamp),
        );
    }

    #[test]
    fn verify_signature_rejects_invalid_timestamp_format() {
        assert_eq!(
            verify_signature(
                Some("not-a-number"),
                Some("v0=abc"),
                "app1:ch1",
                b"{}",
                "s",
                0,
            ),
            Err(SignatureCheckError::InvalidTimestamp),
        );
    }

    #[test]
    fn verify_signature_rejects_wrong_signature() {
        let ts = 1_700_000_000_i64;
        let body = b"{}";
        // Compute a signature with one secret, verify with another.
        let bogus = sign("not-the-secret", ts, "app1:ch1", body);
        assert_eq!(
            verify_signature(
                Some(&ts.to_string()),
                Some(&bogus),
                "app1:ch1",
                body,
                "the-secret",
                ts,
            ),
            Err(SignatureCheckError::SignatureMismatch),
        );
    }

    #[test]
    fn verify_signature_rejects_scope_mismatch() {
        // Signature produced for one channel scope must not verify against a
        // different one, even when secret/timestamp/body are identical. This
        // is the core security property added by the scope-bound basestring
        // (TM-A2A-010) — without it, operators reusing one `signing_secret`
        // across multiple A2A channels are exposed to cross-channel replay.
        let secret = "topsecret";
        let ts = 1_700_000_000_i64;
        let body = br#"{"jsonrpc":"2.0","method":"message/send"}"#;
        let sig_for_ch1 = sign(secret, ts, "app1:ch1", body);
        assert_eq!(
            verify_signature(
                Some(&ts.to_string()),
                Some(&sig_for_ch1),
                "app1:ch2",
                body,
                secret,
                ts,
            ),
            Err(SignatureCheckError::SignatureMismatch),
        );
    }

    #[test]
    fn verify_signature_rejects_tampered_body() {
        let secret = "topsecret";
        let ts = 1_700_000_000_i64;
        let original = br#"{"a":1}"#;
        let sig = sign(secret, ts, "app1:ch1", original);
        let tampered = br#"{"a":2}"#;
        assert_eq!(
            verify_signature(
                Some(&ts.to_string()),
                Some(&sig),
                "app1:ch1",
                tampered,
                secret,
                ts,
            ),
            Err(SignatureCheckError::SignatureMismatch),
        );
    }

    #[tokio::test]
    async fn replay_store_in_memory_first_seen_then_replay() {
        let store = A2aReplayStore::in_memory();
        let scope = "app1:ch1";
        let sig = "v0=abcdef";
        assert!(store.try_record(scope, sig).await, "first sighting");
        assert!(
            !store.try_record(scope, sig).await,
            "second sighting must be rejected as replay",
        );
    }

    #[tokio::test]
    async fn replay_store_in_memory_distinct_scopes_independent() {
        let store = A2aReplayStore::in_memory();
        let sig = "v0=abcdef";
        assert!(store.try_record("app1:ch1", sig).await);
        // Same signature on a different channel scope must still be
        // first-seen — buckets are scoped per channel just like the
        // rate limiter (TM-A2A-013).
        assert!(store.try_record("app1:ch2", sig).await);
        assert!(store.try_record("app2:ch1", sig).await);
    }

    #[tokio::test]
    async fn replay_store_in_memory_distinct_signatures_independent() {
        let store = A2aReplayStore::in_memory();
        let scope = "app1:ch1";
        for i in 0..5 {
            let sig = format!("v0=signature-{i}");
            assert!(store.try_record(scope, &sig).await);
        }
    }
}
