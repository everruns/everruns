// Refresh-token rotation for the MCP OAuth token endpoint.
//
// Decision: the earlier lookup in `handle_refresh_token_grant` validates
// client, user and grant without burning the token on unrelated transient
// failures; this step is the gate that closes concurrent refresh races. One
// replay inside the reuse grace window also mints (a lost response, or two
// sessions of one host refreshing together); a later replay deletes the token's
// rotation family. Policy: `storage::repositories::auth::REFRESH_REUSE_GRACE_SECS`.

use super::{McpOAuthState, OAuthErrorResponse};
use crate::storage::{OAuthRefreshTokenRow, OAuthRefreshUse};

fn invalid() -> OAuthErrorResponse {
    OAuthErrorResponse {
        error: "invalid_grant".to_string(),
        error_description: Some("Invalid or expired refresh token".to_string()),
    }
}

/// Mark the presented refresh token rotated and return it, so the caller can
/// issue its successor in the same family.
pub(super) async fn rotate(
    state: &McpOAuthState,
    client_id: &str,
    token_hash: &str,
) -> Result<OAuthRefreshTokenRow, OAuthErrorResponse> {
    let used = state
        .db
        .use_oauth_refresh_token(token_hash)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to rotate OAuth refresh token");
            OAuthErrorResponse {
                error: "server_error".to_string(),
                error_description: None,
            }
        })?;
    match used {
        OAuthRefreshUse::Mint(row) => Ok(row),
        OAuthRefreshUse::Refused => Err(invalid()),
        OAuthRefreshUse::ReuseDetected => {
            tracing::warn!(
                client_id,
                "MCP OAuth: rotated refresh token replayed after grace window; family revoked"
            );
            Err(invalid())
        }
    }
}
