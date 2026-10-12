// Grant bookkeeping for the MCP OAuth endpoints (connected AI clients).
//
// Spec: knowledge/integrations/mcp-connected-clients.md (phase 1).
//
// Decision: the consent POST writes the grant; the token endpoint only reads
// it, or creates one for an authorization code or refresh token that predates
// grants. Both token grants re-check the grant against the database (never a
// cache): a revoked grant mints nothing, and a refresh racing a revoke can at
// most leave a refresh token the next refresh rejects.

use super::{McpOAuthState, OAuthErrorResponse};
use crate::auth::jwt::McpTokenGrant;
use crate::auth::middleware::AuthError;
use crate::storage::{OAuthGrantRow, UserRow};
use uuid::Uuid;

fn server_error() -> OAuthErrorResponse {
    OAuthErrorResponse {
        error: "server_error".to_string(),
        error_description: None,
    }
}

fn revoked() -> OAuthErrorResponse {
    OAuthErrorResponse {
        error: "invalid_grant".to_string(),
        error_description: Some("Authorization was revoked".to_string()),
    }
}

/// Record the user's approval on the consent page.
pub(super) async fn approve(
    state: &McpOAuthState,
    client_id: &str,
    user_id: Uuid,
) -> Result<OAuthGrantRow, AuthError> {
    state
        .db
        .approve_oauth_grant(client_id, user_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to record MCP OAuth grant");
            AuthError::internal("Failed to record authorization")
        })
}

/// The live grant a token request is made under.
///
/// `grant_id` comes from the refresh token. `None` (an authorization code, or a
/// refresh token minted before grants existed) resolves the client and user's
/// grant, creating one only if none exists, so a revoked grant stays revoked.
pub(super) async fn live_grant(
    state: &McpOAuthState,
    client_id: &str,
    user_id: Uuid,
    grant_id: Option<Uuid>,
) -> Result<OAuthGrantRow, OAuthErrorResponse> {
    let grant = match grant_id {
        Some(id) => state.db.get_oauth_grant(id).await,
        None => state
            .db
            .ensure_oauth_grant(client_id, user_id)
            .await
            .map(Some),
    }
    .map_err(|e| {
        tracing::error!(error = %e, "Failed to look up MCP OAuth grant");
        server_error()
    })?
    .ok_or_else(revoked)?;
    if grant.revoked_at.is_some() || grant.client_id != client_id || grant.user_id != user_id {
        return Err(revoked());
    }
    Ok(grant)
}

/// Mint a resource-bound MCP access token (THREAT[TM-MCP-006]: token_type
/// `mcp_access`, aud = `{root}/mcp`, so it is rejected on `/api/*`) that names
/// the client and grant, so `/mcp` can reject it once the grant is revoked.
pub(super) fn mint_access_token(
    state: &McpOAuthState,
    user: &UserRow,
    grant: &OAuthGrantRow,
) -> Result<String, OAuthErrorResponse> {
    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();
    state
        .jwt_service
        .generate_mcp_access_token_for_grant(
            user.id,
            &user.email,
            &user.name,
            &roles,
            &state.mcp_resource(),
            McpTokenGrant {
                client_id: &grant.client_id,
                grant_id: grant.id,
            },
        )
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("Failed to generate token".to_string()),
        })
}
