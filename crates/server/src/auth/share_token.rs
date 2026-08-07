// Read-only share tokens for eval runs (knowledge/execution/public-endpoints.md,
// knowledge/evaluation/evals.md). A share token unlocks an anonymous, read-only view of one
// eval run via `GET /v1/public/eval-runs/{token}`.
//
// Decision: mirror the personal-access-token pattern — an opaque random token,
// surfaced once at creation, stored only as a SHA-256 hash. The `evr_share_`
// prefix distinguishes it from PATs (`evr_pat_`) and provider api keys. Unlike a
// PAT it is NOT a user credential: it grants read-only access to exactly one
// shared run and plugs into no auth backend.

use rand::RngExt;
use sha2::{Digest, Sha256};

/// Share token prefix for identification.
pub const SHARE_PREFIX: &str = "evr_share_";
const SHARE_LENGTH: usize = 32; // 32 random bytes = 64 hex chars

/// A freshly generated share token. The full `token` is shown only once.
#[derive(Debug)]
pub struct GeneratedShareToken {
    /// Full token (`evr_share_<64 hex>`) — surfaced once, never stored.
    pub token: String,
    /// SHA-256 hash for database storage/lookup.
    pub token_hash: String,
    /// Prefix for display (e.g. `evr_share_abc12345...`).
    pub token_prefix: String,
}

/// Mint a new share token.
pub fn generate_share_token() -> GeneratedShareToken {
    let mut rng = rand::rng();
    let random_bytes: Vec<u8> = (0..SHARE_LENGTH).map(|_| rng.random()).collect();
    let random_hex = hex::encode(&random_bytes);
    let token = format!("{SHARE_PREFIX}{random_hex}");
    let token_hash = hash_share_token(&token);
    let token_prefix = format!("{SHARE_PREFIX}{}...", &random_hex[..8]);
    GeneratedShareToken {
        token,
        token_hash,
        token_prefix,
    }
}

/// Hash a share token for storage/lookup (bare SHA-256, like PATs).
pub fn hash_share_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
