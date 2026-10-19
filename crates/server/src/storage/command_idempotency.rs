//! Idempotency keys for `POST /v1/commands/{name}` (see
//! the HTTP handler in `api::command_dispatch`). The storage only claims, completes and
//! releases rows; deciding what a stored row means for a request is the
//! handler's job.

use chrono::{DateTime, Utc};
use uuid::Uuid;

/// Who a key belongs to and which request it names.
#[derive(Debug, Clone)]
pub struct IdempotencyKeyScope {
    pub org_id: i64,
    /// Caller's principal; `Uuid::nil()` when the request has none.
    pub principal_id: Uuid,
    pub key: String,
}

/// A request that wants to own `scope`.
#[derive(Debug, Clone)]
pub struct ClaimIdempotencyKey {
    pub scope: IdempotencyKeyScope,
    pub command: String,
    /// Hash of what makes two requests the same: command, params, version.
    pub fingerprint: String,
    /// End of the in-flight lease.
    pub locked_until: DateTime<Utc>,
    /// When the key is forgotten.
    pub expires_at: DateTime<Utc>,
}

/// A live row for a key this request did not claim.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredIdempotencyKey {
    pub command: String,
    pub fingerprint: String,
    /// The stored response once the first request finished, as the handler
    /// sealed it; `None` while it is still running.
    pub response: Option<Vec<u8>>,
}

/// Outcome of a claim.
#[derive(Debug, Clone, PartialEq)]
pub enum IdempotencyClaim {
    /// This request owns the key and must run the command, then complete or
    /// release it.
    Claimed,
    /// Another request owns or finished the key.
    Existing(StoredIdempotencyKey),
}

/// Expired rows a single claim sweeps, so the table stays bounded without a
/// separate job.
pub(crate) const EXPIRED_SWEEP_LIMIT: i64 = 100;
