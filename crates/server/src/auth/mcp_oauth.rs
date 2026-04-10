// MCP OAuth 2.1 endpoints (backend-agnostic)
//
// Spec: specs/mcp.md
//
// Decision: Same pattern as CLI auth — authorize endpoint uses AuthUser extractor,
// works with any auth backend (builtin, PropelAuth, Auth0, etc.).
// Decision: PKCE mandatory (S256 only). No implicit grant.
// Decision: Dynamic client registration per RFC 7591.
// Decision: MCP access tokens are JWTs with token_type="mcp_access".

use super::{
    audit,
    jwt::JwtService,
    middleware::{AuthError, AuthState, AuthUser},
};
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateOAuthAuthorizationCodeRow, CreateOAuthClientRow, CreateOAuthRefreshTokenRow,
};
use axum::{
    Form, Json, Router,
    extract::{FromRef, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use rand::Rng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use utoipa::ToSchema;

/// Authorization code TTL (5 minutes)
const AUTH_CODE_TTL_SECS: i64 = 300;

/// MCP access token lifetime (1 hour)
const MCP_ACCESS_TOKEN_LIFETIME_SECS: i64 = 3600;

/// MCP refresh token lifetime (30 days)
const MCP_REFRESH_TOKEN_LIFETIME_SECS: i64 = 30 * 24 * 3600;

/// Generate a random hex string (32 bytes = 64 hex chars)
fn generate_random_hex() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    hex::encode(bytes)
}

/// SHA-256 hash for storage
fn hash_value(value: &str) -> String {
    let hash = Sha256::digest(value.as_bytes());
    hex::encode(hash)
}

/// Verify PKCE S256 challenge: BASE64URL(SHA256(verifier)) == challenge
fn verify_pkce_s256(verifier: &str, challenge: &str) -> bool {
    use base64::Engine;
    let hash = Sha256::digest(verifier.as_bytes());
    let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);
    computed == challenge
}

// ============================================
// Request/Response types
// ============================================

/// POST /v1/oauth/register request
#[derive(Debug, Deserialize, ToSchema)]
pub struct OAuthRegisterRequest {
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

/// POST /v1/oauth/register response
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthRegisterResponse {
    pub client_id: String,
    pub client_secret: String,
    pub client_name: String,
    pub redirect_uris: Vec<String>,
}

/// GET /v1/oauth/authorize query parameters
#[derive(Debug, Deserialize)]
pub struct OAuthAuthorizeQuery {
    pub client_id: String,
    pub redirect_uri: String,
    pub response_type: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    pub state: String,
    #[serde(default = "default_scope")]
    pub scope: String,
}

fn default_scope() -> String {
    "mcp".to_string()
}

/// POST /v1/oauth/token request (form-encoded per OAuth spec)
#[derive(Debug, Deserialize)]
pub struct OAuthTokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub redirect_uri: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
}

/// POST /v1/oauth/token response
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// OAuth error response (RFC 6749 §5.2)
#[derive(Debug, Serialize)]
pub struct OAuthErrorResponse {
    pub error: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_description: Option<String>,
}

impl IntoResponse for OAuthErrorResponse {
    fn into_response(self) -> Response {
        (StatusCode::BAD_REQUEST, Json(self)).into_response()
    }
}

/// Authorization server metadata (RFC 8414)
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthServerMetadata {
    pub issuer: String,
    pub authorization_endpoint: String,
    pub token_endpoint: String,
    pub registration_endpoint: String,
    pub response_types_supported: Vec<String>,
    pub grant_types_supported: Vec<String>,
    pub code_challenge_methods_supported: Vec<String>,
    pub token_endpoint_auth_methods_supported: Vec<String>,
    pub scopes_supported: Vec<String>,
}

// ============================================
// State
// ============================================

/// State for MCP OAuth routes — decoupled from any specific AuthBackend.
#[derive(Clone)]
pub struct McpOAuthState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub jwt_service: Arc<JwtService>,
    /// Public issuer URL without the API prefix (e.g. `https://app.example.com`).
    pub issuer_url: String,
    /// Full backend API base URL including any path prefix (e.g. `https://app.example.com/api`).
    pub api_base_url: String,
}

impl FromRef<McpOAuthState> for AuthState {
    fn from_ref(state: &McpOAuthState) -> Self {
        state.auth.clone()
    }
}

// ============================================
// Routes
// ============================================

/// Create root-level MCP OAuth metadata routes.
pub fn mcp_oauth_public_routes(state: McpOAuthState) -> Router {
    Router::new()
        .route(
            "/.well-known/oauth-authorization-server",
            get(oauth_server_metadata),
        )
        .with_state(state)
}

/// Create MCP OAuth API routes mounted under the API prefix.
pub fn mcp_oauth_api_routes(state: McpOAuthState) -> Router {
    Router::new()
        .route("/v1/oauth/register", post(oauth_register))
        .route("/v1/oauth/authorize", get(oauth_authorize))
        .route("/v1/oauth/token", post(oauth_token))
        .with_state(state)
}

// ============================================
// Handlers
// ============================================

/// GET /.well-known/oauth-authorization-server — Server metadata
async fn oauth_server_metadata(State(state): State<McpOAuthState>) -> Json<OAuthServerMetadata> {
    let issuer = state.issuer_url.trim_end_matches('/');
    let api_base = state.api_base_url.trim_end_matches('/');
    Json(OAuthServerMetadata {
        issuer: issuer.to_string(),
        authorization_endpoint: format!("{api_base}/v1/oauth/authorize"),
        token_endpoint: format!("{api_base}/v1/oauth/token"),
        registration_endpoint: format!("{api_base}/v1/oauth/register"),
        response_types_supported: vec!["code".to_string()],
        grant_types_supported: vec![
            "authorization_code".to_string(),
            "refresh_token".to_string(),
        ],
        code_challenge_methods_supported: vec!["S256".to_string()],
        token_endpoint_auth_methods_supported: vec!["client_secret_post".to_string()],
        scopes_supported: vec!["mcp".to_string()],
    })
}

/// POST /v1/oauth/register — Dynamic Client Registration (RFC 7591)
async fn oauth_register(
    State(state): State<McpOAuthState>,
    Json(req): Json<OAuthRegisterRequest>,
) -> Result<(StatusCode, Json<OAuthRegisterResponse>), OAuthErrorResponse> {
    // Validate
    if req.client_name.is_empty() || req.client_name.len() > 255 {
        return Err(OAuthErrorResponse {
            error: "invalid_client_metadata".to_string(),
            error_description: Some("client_name must be 1-255 characters".to_string()),
        });
    }
    if req.redirect_uris.is_empty() {
        return Err(OAuthErrorResponse {
            error: "invalid_client_metadata".to_string(),
            error_description: Some("At least one redirect_uri is required".to_string()),
        });
    }

    // Generate client credentials
    let client_id = format!("mcp_client_{}", generate_random_hex());
    let client_secret = format!("mcp_secret_{}", generate_random_hex());
    let client_secret_hash = hash_value(&client_secret);

    let redirect_uris_json =
        serde_json::to_value(&req.redirect_uris).map_err(|_| OAuthErrorResponse {
            error: "invalid_client_metadata".to_string(),
            error_description: Some("Invalid redirect_uris".to_string()),
        })?;

    state
        .db
        .create_oauth_client(CreateOAuthClientRow {
            client_id: client_id.clone(),
            client_secret_hash,
            client_name: req.client_name.clone(),
            redirect_uris: redirect_uris_json,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create OAuth client: {}", e);
            OAuthErrorResponse {
                error: "server_error".to_string(),
                error_description: Some("Failed to register client".to_string()),
            }
        })?;

    Ok((
        StatusCode::CREATED,
        Json(OAuthRegisterResponse {
            client_id,
            client_secret,
            client_name: req.client_name,
            redirect_uris: req.redirect_uris,
        }),
    ))
}

/// GET /v1/oauth/authorize — Authorization endpoint (requires authenticated user)
async fn oauth_authorize(
    State(state): State<McpOAuthState>,
    headers: HeaderMap,
    user: AuthUser,
    Query(query): Query<OAuthAuthorizeQuery>,
) -> Result<Response, AuthError> {
    let ip = audit::client_ip(&headers);

    // Validate response_type
    if query.response_type != "code" {
        return Err(AuthError::unauthorized("Unsupported response_type"));
    }

    // Validate PKCE method
    if query.code_challenge_method != "S256" {
        return Err(AuthError::unauthorized(
            "Only S256 code_challenge_method is supported",
        ));
    }

    // Look up client
    let client = state
        .db
        .get_oauth_client_by_client_id(&query.client_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up OAuth client: {}", e);
            AuthError::unauthorized("Invalid client_id")
        })?
        .ok_or_else(|| AuthError::unauthorized("Invalid client_id"))?;

    // Validate redirect_uri against registered URIs
    let registered_uris: Vec<String> =
        serde_json::from_value(client.redirect_uris).unwrap_or_default();
    if !registered_uris.contains(&query.redirect_uri) {
        return Err(AuthError::unauthorized("Invalid redirect_uri"));
    }

    // Use user's first org (same as CLI auth)
    let org_id = user
        .organizations
        .first()
        .map(|o| o.org_id)
        .unwrap_or(everruns_core::DEFAULT_ORG_ID);

    // Generate authorization code
    let code = generate_random_hex();
    let code_hash = hash_value(&code);

    let expires_at = Utc::now() + Duration::seconds(AUTH_CODE_TTL_SECS);

    state
        .db
        .create_oauth_authorization_code(CreateOAuthAuthorizationCodeRow {
            code_hash,
            client_id: query.client_id.clone(),
            user_id: user.id,
            org_id,
            redirect_uri: query.redirect_uri.clone(),
            code_challenge: query.code_challenge,
            code_challenge_method: query.code_challenge_method,
            scope: query.scope,
            expires_at,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to create authorization code: {}", e);
            AuthError::unauthorized("Failed to create authorization code")
        })?;

    audit::emit(
        state.db.clone(),
        org_id,
        Some(user.id),
        "auth.mcp_oauth.authorize",
        ip,
        serde_json::json!({"client_id": query.client_id}),
    );

    // Redirect to client with code and state
    let redirect_url = format!(
        "{}?code={}&state={}",
        query.redirect_uri,
        urlencoding::encode(&code),
        urlencoding::encode(&query.state)
    );

    Ok(Redirect::to(&redirect_url).into_response())
}

/// POST /v1/oauth/token — Token exchange (authorization_code or refresh_token grant)
async fn oauth_token(
    State(state): State<McpOAuthState>,
    headers: HeaderMap,
    Form(req): Form<OAuthTokenRequest>,
) -> Result<Json<OAuthTokenResponse>, OAuthErrorResponse> {
    let ip = audit::client_ip(&headers);

    match req.grant_type.as_str() {
        "authorization_code" => handle_authorization_code_grant(&state, &req, ip).await,
        "refresh_token" => handle_refresh_token_grant(&state, &req, ip).await,
        _ => Err(OAuthErrorResponse {
            error: "unsupported_grant_type".to_string(),
            error_description: Some("Supported: authorization_code, refresh_token".to_string()),
        }),
    }
}

async fn handle_authorization_code_grant(
    state: &McpOAuthState,
    req: &OAuthTokenRequest,
    ip: Option<String>,
) -> Result<Json<OAuthTokenResponse>, OAuthErrorResponse> {
    let code = req.code.as_deref().ok_or_else(|| OAuthErrorResponse {
        error: "invalid_request".to_string(),
        error_description: Some("code is required".to_string()),
    })?;
    let client_id = req.client_id.as_deref().ok_or_else(|| OAuthErrorResponse {
        error: "invalid_request".to_string(),
        error_description: Some("client_id is required".to_string()),
    })?;
    let client_secret = req
        .client_secret
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("client_secret is required".to_string()),
        })?;
    let redirect_uri = req
        .redirect_uri
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("redirect_uri is required".to_string()),
        })?;
    let code_verifier = req
        .code_verifier
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("code_verifier is required".to_string()),
        })?;

    // Validate client credentials
    let client = state
        .db
        .get_oauth_client_by_client_id(client_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_client".to_string(),
            error_description: Some("Unknown client_id".to_string()),
        })?;

    // Verify client secret
    let secret_hash = hash_value(client_secret);
    if secret_hash != client.client_secret_hash {
        return Err(OAuthErrorResponse {
            error: "invalid_client".to_string(),
            error_description: Some("Invalid client_secret".to_string()),
        });
    }

    // Look up and consume authorization code
    let code_hash = hash_value(code);
    let auth_code = state
        .db
        .get_oauth_authorization_code_by_hash(&code_hash)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Invalid or expired authorization code".to_string()),
        })?;

    // Check not already consumed
    if auth_code.consumed {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Authorization code already used".to_string()),
        });
    }

    // Validate client_id matches
    if auth_code.client_id != client_id {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("client_id mismatch".to_string()),
        });
    }

    // Validate redirect_uri matches
    if auth_code.redirect_uri != redirect_uri {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("redirect_uri mismatch".to_string()),
        });
    }

    // Verify PKCE
    if !verify_pkce_s256(code_verifier, &auth_code.code_challenge) {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("PKCE verification failed".to_string()),
        });
    }

    // Consume the code (one-time use)
    state
        .db
        .consume_oauth_authorization_code(auth_code.id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?;

    // Fetch user to populate token claims
    let user = state
        .db
        .get_user(auth_code.user_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("User not found".to_string()),
        })?;

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    // Generate MCP access token (JWT with mcp_access token_type)
    let access_token = state
        .jwt_service
        .generate_access_token(auth_code.user_id, &user.email, &user.name, &roles)
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("Failed to generate token".to_string()),
        })?;

    // Generate refresh token
    let refresh_token_raw = generate_random_hex();
    let refresh_token_hash = hash_value(&refresh_token_raw);

    state
        .db
        .create_oauth_refresh_token(CreateOAuthRefreshTokenRow {
            token_hash: refresh_token_hash,
            client_id: client_id.to_string(),
            user_id: auth_code.user_id,
            org_id: auth_code.org_id,
            scope: auth_code.scope,
            expires_at: Utc::now() + Duration::seconds(MCP_REFRESH_TOKEN_LIFETIME_SECS),
        })
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?;

    // Cleanup expired codes (fire and forget)
    let db = state.db.clone();
    tokio::spawn(async move {
        let _ = db.delete_expired_oauth_authorization_codes().await;
    });

    audit::emit(
        state.db.clone(),
        auth_code.org_id,
        Some(auth_code.user_id),
        "auth.mcp_oauth.token",
        ip,
        serde_json::json!({"client_id": client_id, "grant_type": "authorization_code"}),
    );

    Ok(Json(OAuthTokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: MCP_ACCESS_TOKEN_LIFETIME_SECS,
        refresh_token: Some(refresh_token_raw),
    }))
}

async fn handle_refresh_token_grant(
    state: &McpOAuthState,
    req: &OAuthTokenRequest,
    ip: Option<String>,
) -> Result<Json<OAuthTokenResponse>, OAuthErrorResponse> {
    let refresh_token = req
        .refresh_token
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("refresh_token is required".to_string()),
        })?;
    let client_id = req.client_id.as_deref().ok_or_else(|| OAuthErrorResponse {
        error: "invalid_request".to_string(),
        error_description: Some("client_id is required".to_string()),
    })?;
    let client_secret = req
        .client_secret
        .as_deref()
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_request".to_string(),
            error_description: Some("client_secret is required".to_string()),
        })?;

    // Validate client credentials
    let client = state
        .db
        .get_oauth_client_by_client_id(client_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_client".to_string(),
            error_description: Some("Unknown client_id".to_string()),
        })?;

    let secret_hash = hash_value(client_secret);
    if secret_hash != client.client_secret_hash {
        return Err(OAuthErrorResponse {
            error: "invalid_client".to_string(),
            error_description: Some("Invalid client_secret".to_string()),
        });
    }

    // Look up refresh token
    let token_hash = hash_value(refresh_token);
    let stored_token = state
        .db
        .get_oauth_refresh_token_by_hash(&token_hash)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("Invalid or expired refresh token".to_string()),
        })?;

    // Validate client_id matches
    if stored_token.client_id != client_id {
        return Err(OAuthErrorResponse {
            error: "invalid_grant".to_string(),
            error_description: Some("client_id mismatch".to_string()),
        });
    }

    // Delete old refresh token (rotation)
    let _ = state.db.delete_oauth_refresh_token(stored_token.id).await;

    // Fetch user
    let user = state
        .db
        .get_user(stored_token.user_id)
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?
        .ok_or_else(|| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("User not found".to_string()),
        })?;

    let roles: Vec<String> = serde_json::from_value(user.roles.clone()).unwrap_or_default();

    // Generate new access token
    let access_token = state
        .jwt_service
        .generate_access_token(stored_token.user_id, &user.email, &user.name, &roles)
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: Some("Failed to generate token".to_string()),
        })?;

    // Generate new refresh token (rotation)
    let new_refresh_token = generate_random_hex();
    let new_refresh_token_hash = hash_value(&new_refresh_token);

    state
        .db
        .create_oauth_refresh_token(CreateOAuthRefreshTokenRow {
            token_hash: new_refresh_token_hash,
            client_id: client_id.to_string(),
            user_id: stored_token.user_id,
            org_id: stored_token.org_id,
            scope: stored_token.scope,
            expires_at: Utc::now() + Duration::seconds(MCP_REFRESH_TOKEN_LIFETIME_SECS),
        })
        .await
        .map_err(|_| OAuthErrorResponse {
            error: "server_error".to_string(),
            error_description: None,
        })?;

    audit::emit(
        state.db.clone(),
        stored_token.org_id,
        Some(stored_token.user_id),
        "auth.mcp_oauth.token",
        ip,
        serde_json::json!({"client_id": client_id, "grant_type": "refresh_token"}),
    );

    Ok(Json(OAuthTokenResponse {
        access_token,
        token_type: "Bearer".to_string(),
        expires_in: MCP_ACCESS_TOKEN_LIFETIME_SECS,
        refresh_token: Some(new_refresh_token),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_random_hex_uniqueness() {
        let a = generate_random_hex();
        let b = generate_random_hex();
        assert_ne!(a, b);
        assert_eq!(a.len(), 64); // 32 bytes = 64 hex chars
    }

    #[test]
    fn test_hash_value_consistency() {
        let value = "test-value";
        assert_eq!(hash_value(value), hash_value(value));
    }

    #[test]
    fn test_pkce_s256_verification() {
        use base64::Engine;

        // Generate a verifier and compute the challenge
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let hash = Sha256::digest(verifier.as_bytes());
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(hash);

        assert!(verify_pkce_s256(verifier, &challenge));
        assert!(!verify_pkce_s256("wrong-verifier", &challenge));
    }

    #[test]
    fn test_pkce_s256_rfc_example() {
        // RFC 7636 Appendix B test vector
        // verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
        // challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(verify_pkce_s256(verifier, challenge));
    }
}
