// User Connections API routes
// Decision: User-scoped (not org-scoped) — token represents user's identity
// Decision: GitHub App installation flow replaces OAuth App for repo access
// Decision: API-key providers (Daytona etc.) register via ConnectionProviderPlugin
//   and define their own form schema + validation. Server discovers them at runtime.

use crate::auth::ResolvedOrg;
use crate::auth::config::AuthConfig;
use crate::auth::middleware::{AuthState, AuthUser};
use crate::auth::oauth::GitHubAppService;
use crate::services::McpServerService;
use crate::services::mcp_server::{McpServerOAuthSettings, McpServerSettings};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{delete, get, post, put},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use everruns_core::connection_provider::{
    ConnectionFormSchema as CoreFormSchema, ConnectionProviderRegistry, ConnectionType,
};
use everruns_core::{
    Caller, McpServerAuthMode, SessionId, mcp_oauth_provider_id_for_uuid,
    mcp_oauth_session_secret_name, validate_safe_url,
};
use rand::Rng;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{impl_auth_state, sanitized_bad_gateway, sanitized_internal_error};
use crate::storage::models::CreateUserConnectionRow;

/// App state for user connections routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    pub auth_config: AuthConfig,
    pub connection_providers: ConnectionProviderRegistry,
    pub mcp_service: Arc<McpServerService>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
        auth_config: AuthConfig,
        connection_providers: ConnectionProviderRegistry,
        mcp_service: Arc<McpServerService>,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
            auth_config,
            connection_providers,
            mcp_service,
        }
    }
}

impl_auth_state!(AppState);

// ============================================================================
// Response / Request Types
// ============================================================================

/// Connection info returned in API responses (never includes token)
#[derive(Debug, Serialize, ToSchema)]
pub struct ConnectionResponse {
    pub provider: String,
    pub connection_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scopes: Option<String>,
    pub connected_at: DateTime<Utc>,
}

/// Provider info for the connections UI
#[derive(Debug, Serialize)]
pub struct ProviderResponse {
    pub provider_id: String,
    pub display_name: String,
    pub description: String,
    pub icon: String,
    pub connection_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub form_schema: Option<FormSchemaResponse>,
}

/// Form schema for API-key providers
#[derive(Debug, Serialize)]
pub struct FormSchemaResponse {
    pub fields: Vec<FormFieldResponse>,
    pub instructions_markdown: String,
}

/// Single form field
#[derive(Debug, Serialize)]
pub struct FormFieldResponse {
    pub name: String,
    pub label: String,
    pub field_type: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub help_text: Option<String>,
}

/// Request body for API-key connection creation (plugin-based providers).
/// Accepts api_key plus any additional form fields as extra_fields.
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyConnectionRequest {
    pub api_key: String,
    /// Additional provider-specific form fields (e.g. org_slug for Deno personal tokens).
    #[serde(flatten)]
    pub extra_fields: std::collections::HashMap<String, serde_json::Value>,
}

/// Request body for API-key-based connections (e.g., Brave Search)
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApiKeyConnectionRequest {
    pub api_key: String,
}

/// Providers that support API-key-based connections (legacy, prefer ConnectionProviderPlugin).
const API_KEY_PROVIDERS: &[&str] = &[];

/// Cookie name for GitHub App installation CSRF state
const GITHUB_INSTALL_STATE_COOKIE: &str = "github_install_state";

/// GitHub App installation callback query params
#[derive(Debug, Deserialize)]
pub struct GitHubInstallationCallbackQuery {
    pub installation_id: i64,
    #[allow(dead_code)]
    pub setup_action: Option<String>,
    pub state: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthAuthorizeQuery {
    pub return_to: Option<String>,
    pub mode: Option<String>,
    pub session_id: Option<String>,
    pub popup: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    pub state: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingOAuthState {
    state: String,
    provider: String,
    return_to: String,
    mode: String,
    session_id: Option<String>,
    popup: bool,
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
struct OAuthProtectedResourceMetadata {
    #[serde(default)]
    authorization_servers: Vec<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthServerMetadata {
    issuer: Option<String>,
    authorization_endpoint: String,
    token_endpoint: String,
    #[serde(default)]
    registration_endpoint: Option<String>,
    #[serde(default)]
    scopes_supported: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize, Clone)]
struct OAuthClientRegistration {
    client_id: String,
    client_secret: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OAuthTokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    scope: Option<String>,
}

// ============================================================================
// Routes
// ============================================================================

/// Create user connections routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/user/connections", get(list_connections))
        .route(
            "/v1/user/connections/providers",
            get(list_connection_providers),
        )
        .route(
            "/v1/user/connections/{provider}",
            delete(delete_connection).post(create_api_key_connection),
        )
        .route(
            "/v1/user/connections/{provider}/verify",
            post(verify_connection),
        )
        .route(
            "/v1/user/connections/{provider}/authorize",
            get(authorize_connection),
        )
        .route(
            "/v1/user/connections/{provider}/callback",
            get(connection_oauth_callback),
        )
        .route(
            "/v1/user/connections/github/authorize",
            get(github_authorize),
        )
        .route("/v1/user/connections/github/callback", get(github_callback))
        .route(
            "/v1/user/connections/api-key/{provider}",
            put(put_api_key_connection),
        )
        .with_state(state)
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /v1/user/connections — List user's connected accounts
pub async fn list_connections(
    State(state): State<AppState>,
    auth: AuthUser,
) -> Result<Json<Vec<ConnectionResponse>>, StatusCode> {
    let rows = state.db.list_user_connections(auth.id).await.map_err(|e| {
        tracing::error!("Failed to list user connections: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    let connections = rows
        .into_iter()
        .map(|r| ConnectionResponse {
            provider: r.provider,
            connection_type: r.connection_type,
            provider_username: r.provider_username,
            scopes: r.scopes,
            connected_at: r.created_at,
        })
        .collect();

    Ok(Json(connections))
}

/// GET /v1/user/connections/providers — List available connection providers
///
/// Returns both hardcoded providers (GitHub/OAuth) and plugin-registered
/// providers (Daytona/API-key). Frontend uses this to render connection forms.
pub async fn list_connection_providers(
    State(state): State<AppState>,
    org: ResolvedOrg,
) -> Json<Vec<ProviderResponse>> {
    let mut providers = Vec::new();

    // Hardcoded GitHub OAuth provider (only if configured)
    if state.auth_config.github_connection.is_some() {
        providers.push(ProviderResponse {
            provider_id: "github".to_string(),
            display_name: "GitHub".to_string(),
            description: "Access private repositories for agent sessions".to_string(),
            icon: "github".to_string(),
            connection_type: "oauth".to_string(),
            form_schema: None,
        });
    }

    // Platform-registered providers (API-key based)
    for provider in state.connection_providers.list() {
        let form_schema = provider.form_schema().map(|s| form_schema_to_response(&s));
        let conn_type = match provider.connection_type() {
            ConnectionType::OAuth => "oauth",
            ConnectionType::ApiKey => "api_key",
        };
        providers.push(ProviderResponse {
            provider_id: provider.provider_id().to_string(),
            display_name: provider.display_name().to_string(),
            description: provider.description().to_string(),
            icon: provider.icon().to_string(),
            connection_type: conn_type.to_string(),
            form_schema,
        });
    }

    match state.db.list_mcp_servers(org.org_id, None, false).await {
        Ok(servers) => {
            providers.extend(servers.into_iter().filter_map(|server| {
                let settings = McpServerService::settings_from_row(&server);
                (settings.auth_mode == McpServerAuthMode::OAuth).then(|| ProviderResponse {
                    provider_id: mcp_oauth_provider_id_for_uuid(server.id.uuid()),
                    display_name: server.name.clone(),
                    description: server.description.unwrap_or_else(|| {
                        format!("Authenticate {} for chat MCP tool access", server.name)
                    }),
                    icon: "plug".to_string(),
                    connection_type: "oauth".to_string(),
                    form_schema: None,
                })
            }));
        }
        Err(error) => {
            tracing::error!(%error, org_id = org.org_id, "Failed to list MCP OAuth providers");
        }
    }

    Json(providers)
}

/// POST /v1/user/connections/:provider — Create API-key connection
///
/// For providers that use direct API key entry (not OAuth).
/// Validates the key via the provider's validate() method before saving.
pub async fn create_api_key_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<CreateApiKeyConnectionRequest>,
) -> Result<(StatusCode, Json<ConnectionResponse>), (StatusCode, String)> {
    let provider = state
        .connection_providers
        .get(&provider_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Unknown connection provider: {provider_id}"),
            )
        })?;

    // Only API-key providers support direct creation
    if provider.connection_type() != ConnectionType::ApiKey {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Provider '{provider_id}' uses OAuth, not API key"),
        ));
    }

    let encryption = state.encryption.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        )
    })?;

    // Build form fields map for validation (includes api_key + any extra fields)
    let mut fields = std::collections::HashMap::new();
    fields.insert("api_key".to_string(), body.api_key.clone());
    for (key, value) in &body.extra_fields {
        if let Some(s) = value.as_str() {
            fields.insert(key.clone(), s.to_string());
        }
    }

    // Validate all fields via the provider
    let validation: everruns_core::connection_provider::ConnectionValidation =
        provider.validate_fields(&fields).await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("API key validation failed: {e}"),
            )
        })?;

    // Encrypt and store
    let access_token_encrypted = encryption.encrypt_string(&body.api_key).map_err(|e| {
        tracing::error!("Failed to encrypt API key: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to store connection".to_string(),
        )
    })?;

    let row = state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.id,
            provider: provider_id.clone(),
            connection_type: "api_key".to_string(),
            provider_user_id: None,
            provider_username: validation.provider_username.clone(),
            access_token_encrypted: Some(access_token_encrypted),
            installation_id: None,
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            provider_metadata: validation.provider_metadata.clone(),
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to store {provider_id} connection: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?;

    Ok((
        StatusCode::CREATED,
        Json(ConnectionResponse {
            provider: row.provider,
            connection_type: row.connection_type,
            provider_username: row.provider_username,
            scopes: row.scopes,
            connected_at: row.created_at,
        }),
    ))
}

/// DELETE /v1/user/connections/:provider — Disconnect
pub async fn delete_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
) -> Result<StatusCode, StatusCode> {
    let deleted = state
        .db
        .delete_user_connection(auth.id, &provider)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete connection: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(StatusCode::NOT_FOUND)
    }
}

/// Response for connection verification
#[derive(Debug, Serialize)]
pub struct VerifyConnectionResponse {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// POST /v1/user/connections/:provider/verify — Verify stored API key still works
///
/// Generic endpoint: decrypts the stored credential and calls the provider's
/// validate() method. Works for any API-key provider that implements
/// ConnectionProvider::validate().
pub async fn verify_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider_id): Path<String>,
) -> Result<Json<VerifyConnectionResponse>, (StatusCode, String)> {
    // Look up the stored connection
    let row = state
        .db
        .get_user_connection(auth.id, &provider_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to look up connection for verify: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to look up connection".to_string(),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("No connection found for provider: {provider_id}"),
            )
        })?;

    // Only API-key connections can be verified this way
    if row.connection_type != "api_key" {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("Provider '{provider_id}' does not support API key verification"),
        ));
    }

    let encrypted = row.access_token_encrypted.ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "No stored credential found".to_string(),
        )
    })?;

    // Decrypt
    let encryption = state.encryption.as_ref().ok_or_else(|| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        )
    })?;
    let credential = encryption.decrypt_to_string(&encrypted).map_err(|e| {
        tracing::error!("Failed to decrypt credential for verify: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to decrypt credential".to_string(),
        )
    })?;

    let provider = state
        .connection_providers
        .get(&provider_id)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                format!("Unknown connection provider: {provider_id}"),
            )
        })?;

    // Build fields map from stored credential + metadata for full validation
    let mut fields = std::collections::HashMap::new();
    fields.insert("api_key".to_string(), credential);
    if let Some(meta) = row.provider_metadata.as_ref()
        && let Some(obj) = meta.as_object()
    {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                fields.insert(k.clone(), s.to_string());
            }
        }
    }

    match provider.validate_fields(&fields).await {
        Ok(_) => Ok(Json(VerifyConnectionResponse {
            valid: true,
            error: None,
        })),
        Err(msg) => Ok(Json(VerifyConnectionResponse {
            valid: false,
            error: Some(msg),
        })),
    }
}

/// GET /v1/user/connections/:provider/authorize — start OAuth flow for dynamic providers.
pub async fn authorize_connection(
    State(state): State<AppState>,
    org: ResolvedOrg,
    jar: CookieJar,
    Path(provider): Path<String>,
    Query(query): Query<OAuthAuthorizeQuery>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let Some(server_id) = parse_mcp_oauth_provider_id(&provider) else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Unknown OAuth provider: {provider}"),
        ));
    };
    let row = state
        .db
        .get_mcp_server(org.org_id, server_id)
        .await
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "MCP server not found".to_string()))?;
    let settings = McpServerService::settings_from_row(&row);
    if settings.auth_mode != McpServerAuthMode::OAuth {
        return Err((
            StatusCode::BAD_REQUEST,
            "MCP server does not use OAuth".to_string(),
        ));
    }

    let return_to = normalize_return_to(
        query.return_to.as_deref(),
        &format!("/settings/connections?connected={provider}"),
    );
    let popup = query.popup.unwrap_or(false);
    let mode = normalize_oauth_mode(query.mode.as_deref())?;
    let session_id = match mode.as_str() {
        "session" => Some(query.session_id.ok_or((
            StatusCode::BAD_REQUEST,
            "session_id is required for session OAuth flows".to_string(),
        ))?),
        _ => None,
    };

    let oauth_state = {
        let bytes: [u8; 16] = rand::rng().random();
        hex::encode(bytes)
    };
    let code_verifier = generate_pkce_verifier();
    let code_challenge = pkce_challenge(&code_verifier);

    let (metadata, registration, updated_settings) =
        ensure_mcp_oauth_registration(&state, &row, settings, &provider).await?;
    state
        .db
        .update_mcp_server(
            org.org_id,
            server_id,
            crate::storage::models::UpdateMcpServer {
                settings: Some(serde_json::to_value(updated_settings).unwrap_or_default()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;

    let pending = PendingOAuthState {
        state: oauth_state.clone(),
        provider: provider.clone(),
        return_to,
        mode,
        session_id,
        popup,
        code_verifier,
    };
    let cookie = Cookie::build((
        oauth_state_cookie_name(&provider),
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&pending)
                .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
        ),
    ))
    .path("/")
    .http_only(true)
    .secure(true)
    .same_site(SameSite::Lax)
    .max_age(time::Duration::minutes(10))
    .build();

    // Validate authorize URL even for preconfigured endpoints (settings may
    // have been stored before SSRF validation was enforced at discovery time).
    validate_safe_url(&metadata.authorization_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Authorization endpoint blocked: {e}"),
        )
    })?;
    let authorize_url = reqwest::Url::parse(&metadata.authorization_endpoint)
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
    let scopes = if !metadata.scopes_supported.is_empty() {
        metadata.scopes_supported.join(" ")
    } else {
        "email".to_string()
    };
    let redirect_uri = mcp_oauth_redirect_uri(&state.auth_config, &provider);
    let mut url = authorize_url;
    url.query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &registration.client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("scope", &scopes)
        .append_pair("state", &oauth_state)
        .append_pair("code_challenge", &code_challenge)
        .append_pair("code_challenge_method", "S256");

    Ok((jar.add(cookie), Redirect::to(url.as_str())))
}

pub async fn connection_oauth_callback(
    State(state): State<AppState>,
    org: ResolvedOrg,
    jar: CookieJar,
    Path(provider): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    if provider == "github" {
        return Err((
            StatusCode::BAD_REQUEST,
            "Use the GitHub-specific callback".to_string(),
        ));
    }
    let Some(server_id) = parse_mcp_oauth_provider_id(&provider) else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Unknown OAuth provider: {provider}"),
        ));
    };
    let pending = validate_pending_oauth_state(&jar, &provider, query.state.as_deref())?;
    let clear_cookie = jar.remove(Cookie::from(oauth_state_cookie_name(&provider)));

    let row = state
        .db
        .get_mcp_server(org.org_id, server_id)
        .await
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "MCP server not found".to_string()))?;
    let settings = McpServerService::settings_from_row(&row);
    let oauth = settings.oauth.clone().ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth metadata missing for MCP server".to_string(),
    ))?;
    let client_id = oauth.client_id.ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth client_id missing".to_string(),
    ))?;
    let client_secret = oauth
        .client_secret_encrypted
        .as_deref()
        .map(|v| state.mcp_service.decrypt_string_from_b64(v))
        .transpose()
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
    let token_endpoint = oauth.token_endpoint.ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth token endpoint missing".to_string(),
    ))?;
    let redirect_uri = mcp_oauth_redirect_uri(&state.auth_config, &provider);

    let token = exchange_oauth_code(
        &token_endpoint,
        &client_id,
        client_secret.as_deref(),
        &redirect_uri,
        &query.code,
        &pending.code_verifier,
    )
    .await?;

    let expires_at = token
        .expires_in
        .map(|seconds| Utc::now() + chrono::Duration::seconds(seconds));

    if pending.mode == "session" {
        let session_id = pending
            .session_id
            .as_deref()
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing session_id for session OAuth flow".to_string(),
            ))?
            .parse::<SessionId>()
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid session_id: {e}")))?;
        state
            .db
            .get_session(org.org_id, session_id)
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
            .ok_or((StatusCode::NOT_FOUND, "Session not found".to_string()))?;
        // Verify caller identity: ensure the authenticated user initiated this session's OAuth
        // flow. The session is already org-scoped via get_session, but we also verify the
        // OAuth state cookie was set in *this* browser (validated above via
        // validate_pending_oauth_state). This prevents cross-user token injection since the
        // state cookie + PKCE verifier are browser-bound.
        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        ))?;
        state
            .db
            .upsert_session_secret(crate::storage::models::UpsertSessionSecret {
                session_id,
                name: mcp_oauth_session_secret_name(server_id, "access_token"),
                value_encrypted: encryption
                    .encrypt_string(&token.access_token)
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
            })
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        if let Some(refresh_token) = &token.refresh_token {
            state
                .db
                .upsert_session_secret(crate::storage::models::UpsertSessionSecret {
                    session_id,
                    name: mcp_oauth_session_secret_name(server_id, "refresh_token"),
                    value_encrypted: encryption
                        .encrypt_string(refresh_token)
                        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                })
                .await
                .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        }
        if let Some(expires_at) = expires_at {
            state
                .db
                .upsert_session_secret(crate::storage::models::UpsertSessionSecret {
                    session_id,
                    name: mcp_oauth_session_secret_name(server_id, "expires_at"),
                    value_encrypted: encryption
                        .encrypt_string(&expires_at.to_rfc3339())
                        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                })
                .await
                .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        }
    } else {
        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        ))?;
        state
            .db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id: org.user_id.ok_or((
                    StatusCode::UNAUTHORIZED,
                    "User identity required for OAuth connection".to_string(),
                ))?,
                provider: provider.clone(),
                connection_type: "oauth".to_string(),
                provider_user_id: None,
                provider_username: Some(row.name.clone()),
                access_token_encrypted: Some(
                    encryption
                        .encrypt_string(&token.access_token)
                        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                ),
                refresh_token_encrypted: token
                    .refresh_token
                    .as_deref()
                    .map(|value| encryption.encrypt_string(value))
                    .transpose()
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                scopes: token.scope.clone(),
                expires_at,
                installation_id: None,
                provider_metadata: None,
            })
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
    }

    let _ = state
        .mcp_service
        .cache_tools_for_bearer_token(&Caller::from(&org), server_id, &token.access_token)
        .await;

    let redirect_target = finalize_oauth_redirect(
        &state.auth_config,
        &pending.return_to,
        &provider,
        pending.popup,
    );
    Ok((clear_cookie, Redirect::to(&redirect_target)))
}

/// GET /v1/user/connections/github/authorize — Redirect to GitHub App installation
pub async fn github_authorize(
    State(state): State<AppState>,
    _auth: AuthUser,
    jar: CookieJar,
    Query(_params): Query<std::collections::HashMap<String, String>>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let config = state
        .auth_config
        .github_connection
        .as_ref()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub App not configured".to_string(),
            )
        })?;

    let service = GitHubAppService::new(config);

    // Generate state for CSRF protection
    let mut rng = rand::rng();
    let bytes: [u8; 16] = rng.random();
    let install_state = hex::encode(bytes);

    // Store state in HttpOnly cookie for validation in callback
    let state_cookie = Cookie::build((GITHUB_INSTALL_STATE_COOKIE, install_state.clone()))
        .path("/")
        .http_only(true)
        .secure(true)
        .same_site(SameSite::Lax)
        .max_age(time::Duration::minutes(10))
        .build();
    let jar = jar.add(state_cookie);

    let auth_url = service.installation_url(&install_state);
    Ok((jar, Redirect::to(&auth_url.url)))
}

/// GET /v1/user/connections/github/callback — GitHub App installation callback
///
/// After user installs the GitHub App on their repos, GitHub redirects here
/// with the installation_id. We verify the installation and store the ID.
/// Validates CSRF state from cookie before proceeding.
pub async fn github_callback(
    State(state): State<AppState>,
    auth: AuthUser,
    jar: CookieJar,
    Query(query): Query<GitHubInstallationCallbackQuery>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    // Validate CSRF state parameter
    validate_install_state(&jar, query.state.as_deref())?;

    // Clear the state cookie after validation
    let jar = jar.remove(Cookie::from(GITHUB_INSTALL_STATE_COOKIE));

    let config = state
        .auth_config
        .github_connection
        .as_ref()
        .ok_or_else(|| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "GitHub App not configured".to_string(),
            )
        })?;

    let service = GitHubAppService::new(config);

    // Verify the installation exists and get account details
    let result = service
        .verify_installation(query.installation_id)
        .await
        .map_err(|e| {
            tracing::error!("GitHub App installation verification failed: {}", e);
            (
                StatusCode::BAD_REQUEST,
                "GitHub App installation verification failed".to_string(),
            )
        })?;

    // Store installation_id (no OAuth token needed — tokens minted on demand)
    state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.id,
            provider: "github".to_string(),
            connection_type: "oauth".to_string(),
            provider_user_id: Some(result.account_id),
            provider_username: Some(result.account_login),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            scopes: Some(result.permissions),
            expires_at: None,
            installation_id: Some(result.installation_id),
            provider_metadata: None,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to store GitHub App installation: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?;

    // Redirect back to settings page
    let frontend_url = state.auth_config.frontend_url.trim_end_matches('/');
    Ok((
        jar,
        Redirect::to(&format!(
            "{}/settings/connections?connected=github",
            frontend_url
        )),
    ))
}

/// PUT /v1/user/connections/api-key/:provider — Store an API-key-based connection
///
/// For providers that authenticate with a simple API key (e.g., brave_search).
/// The API key is encrypted at rest using envelope encryption.
pub async fn put_api_key_connection(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
    Json(body): Json<ApiKeyConnectionRequest>,
) -> Result<StatusCode, (StatusCode, String)> {
    // Validate provider
    if !API_KEY_PROVIDERS.contains(&provider.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Provider '{}' does not support API key connections. Supported: {}",
                provider,
                API_KEY_PROVIDERS.join(", ")
            ),
        ));
    }

    if body.api_key.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "API key cannot be empty".to_string(),
        ));
    }

    // Encrypt the API key
    let encryption = state.encryption.as_ref().ok_or_else(|| {
        tracing::error!("Encryption service not available for API key storage");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        )
    })?;

    let encrypted = encryption.encrypt_string(&body.api_key).map_err(|e| {
        tracing::error!("Failed to encrypt API key: {}", e);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to encrypt API key".to_string(),
        )
    })?;

    // Upsert connection
    state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.id,
            provider: provider.clone(),
            connection_type: "api_key".to_string(),
            provider_user_id: None,
            provider_username: None,
            access_token_encrypted: Some(encrypted),
            refresh_token_encrypted: None,
            scopes: None,
            expires_at: None,
            installation_id: None,
            provider_metadata: None,
        })
        .await
        .map_err(|e| {
            tracing::error!("Failed to store {} connection: {}", provider, e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?;

    Ok(StatusCode::NO_CONTENT)
}

// ============================================================================
// State validation
// ============================================================================

/// Validate CSRF state: cookie must exist, query param must exist, and they must match.
fn validate_install_state(
    jar: &CookieJar,
    query_state: Option<&str>,
) -> Result<String, (StatusCode, String)> {
    let stored = jar
        .get(GITHUB_INSTALL_STATE_COOKIE)
        .map(|c| c.value().to_string())
        .ok_or_else(|| {
            tracing::warn!("GitHub install callback missing state cookie (possible CSRF attempt)");
            (
                StatusCode::BAD_REQUEST,
                "Invalid or expired installation state".to_string(),
            )
        })?;

    let callback = query_state.ok_or_else(|| {
        tracing::warn!("GitHub install callback missing state parameter");
        (
            StatusCode::BAD_REQUEST,
            "Missing state parameter".to_string(),
        )
    })?;

    if stored != callback {
        tracing::warn!("GitHub install callback state mismatch (possible CSRF attempt)");
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid installation state".to_string(),
        ));
    }

    Ok(stored)
}

// ============================================================================
// Helpers
// ============================================================================

fn form_schema_to_response(schema: &CoreFormSchema) -> FormSchemaResponse {
    FormSchemaResponse {
        instructions_markdown: schema.instructions_markdown.clone(),
        fields: schema
            .fields
            .iter()
            .map(|f| {
                let field_type = match f.field_type {
                    everruns_core::connection_provider::FieldType::Password => "password",
                    everruns_core::connection_provider::FieldType::Text => "text",
                    everruns_core::connection_provider::FieldType::Url => "url",
                };
                FormFieldResponse {
                    name: f.name.clone(),
                    label: f.label.clone(),
                    field_type: field_type.to_string(),
                    required: f.required,
                    placeholder: f.placeholder.clone(),
                    help_text: f.help_text.clone(),
                }
            })
            .collect(),
    }
}

fn oauth_state_cookie_name(provider: &str) -> String {
    format!("oauth_connection_state_{}", provider.replace(':', "_"))
}

fn normalize_return_to(value: Option<&str>, default_path: &str) -> String {
    match value {
        Some(path) if path.starts_with('/') => path.to_string(),
        _ => default_path.to_string(),
    }
}

fn mcp_oauth_redirect_uri(config: &AuthConfig, provider: &str) -> String {
    // base_url already includes any API prefix (set by AUTH_BASE_URL / BASE_URL env)
    format!(
        "{}/v1/user/connections/{provider}/callback",
        config.base_url.trim_end_matches('/')
    )
}

fn parse_mcp_oauth_provider_id(provider: &str) -> Option<uuid::Uuid> {
    provider
        .strip_prefix("mcp_oauth_")
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
}

fn generate_pkce_verifier() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn validate_pending_oauth_state(
    jar: &CookieJar,
    provider: &str,
    query_state: Option<&str>,
) -> Result<PendingOAuthState, (StatusCode, String)> {
    let cookie_name = oauth_state_cookie_name(provider);
    let cookie = jar.get(&cookie_name).ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Invalid or expired OAuth state".to_string(),
        )
    })?;
    let decoded = URL_SAFE_NO_PAD
        .decode(cookie.value())
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let pending: PendingOAuthState = serde_json::from_slice(&decoded)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let callback_state = query_state.ok_or_else(|| {
        (
            StatusCode::BAD_REQUEST,
            "Missing state parameter".to_string(),
        )
    })?;
    if pending.state != callback_state {
        return Err((StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()));
    }
    Ok(pending)
}

async fn ensure_mcp_oauth_registration(
    state: &AppState,
    row: &crate::storage::McpServerRow,
    mut settings: McpServerSettings,
    provider: &str,
) -> Result<
    (
        OAuthServerMetadata,
        OAuthClientRegistration,
        McpServerSettings,
    ),
    (StatusCode, String),
> {
    let oauth = settings
        .oauth
        .get_or_insert_with(McpServerOAuthSettings::default);
    let metadata = if oauth.authorization_endpoint.is_some() && oauth.token_endpoint.is_some() {
        OAuthServerMetadata {
            issuer: oauth.issuer.clone(),
            authorization_endpoint: oauth.authorization_endpoint.clone().unwrap_or_default(),
            token_endpoint: oauth.token_endpoint.clone().unwrap_or_default(),
            registration_endpoint: oauth.registration_endpoint.clone(),
            scopes_supported: oauth.scopes_supported.clone(),
        }
    } else {
        let resource_metadata = discover_resource_metadata(&row.url).await?;
        let issuer = match resource_metadata.authorization_servers.first().cloned() {
            Some(issuer) => issuer,
            None => resource_origin(&parse_and_validate_url(&row.url)?)?,
        };
        validate_safe_url(&issuer).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("OAuth issuer blocked: {e}"),
            )
        })?;
        let metadata = discover_oauth_server_metadata(&issuer).await?;
        oauth.issuer = metadata.issuer.clone().or(Some(issuer));
        oauth.authorization_endpoint = Some(metadata.authorization_endpoint.clone());
        oauth.token_endpoint = Some(metadata.token_endpoint.clone());
        oauth.registration_endpoint = metadata.registration_endpoint.clone();
        oauth.scopes_supported = if !metadata.scopes_supported.is_empty() {
            metadata.scopes_supported.clone()
        } else {
            resource_metadata.scopes_supported
        };
        metadata
    };

    let registration = if let Some(client_id) = oauth.client_id.clone() {
        OAuthClientRegistration {
            client_id,
            client_secret: oauth
                .client_secret_encrypted
                .as_deref()
                .map(|value| state.mcp_service.decrypt_string_from_b64(value))
                .transpose()
                .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
        }
    } else {
        let registration_endpoint = metadata.registration_endpoint.as_deref().ok_or((
            StatusCode::BAD_REQUEST,
            "OAuth server metadata missing registration_endpoint; configure client_id manually"
                .to_string(),
        ))?;
        let registration = register_oauth_client(
            registration_endpoint,
            &mcp_oauth_redirect_uri(&state.auth_config, provider),
        )
        .await?;
        oauth.client_id = Some(registration.client_id.clone());
        oauth.client_secret_encrypted = registration
            .client_secret
            .as_deref()
            .map(|value| state.mcp_service.encrypt_string_to_b64(value))
            .transpose()
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        registration
    };

    Ok((metadata, registration, settings))
}

async fn discover_resource_metadata(
    server_url: &str,
) -> Result<OAuthProtectedResourceMetadata, (StatusCode, String)> {
    let resource_url = parse_and_validate_url(server_url)?;
    let origin = resource_origin(&resource_url)?;
    let response = reqwest::Client::new()
        .get(format!("{origin}/.well-known/oauth-protected-resource"))
        .send()
        .await
        .map_err(|e| sanitized_bad_gateway("OAuth external service", &e))?;
    json_response_or_error(response).await
}

async fn discover_oauth_server_metadata(
    issuer: &str,
) -> Result<OAuthServerMetadata, (StatusCode, String)> {
    let issuer = parse_and_validate_url(issuer)?;
    let response = reqwest::Client::new()
        .get(format!(
            "{}/.well-known/oauth-authorization-server",
            issuer.as_str().trim_end_matches('/')
        ))
        .send()
        .await
        .map_err(|e| sanitized_bad_gateway("OAuth external service", &e))?;
    let metadata: OAuthServerMetadata = json_response_or_error(response).await?;
    validate_safe_url(&metadata.authorization_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Authorization endpoint blocked: {e}"),
        )
    })?;
    validate_safe_url(&metadata.token_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Token endpoint blocked: {e}"),
        )
    })?;
    if let Some(registration_endpoint) = &metadata.registration_endpoint {
        validate_safe_url(registration_endpoint).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                format!("Registration endpoint blocked: {e}"),
            )
        })?;
    }
    Ok(metadata)
}

async fn register_oauth_client(
    registration_endpoint: &str,
    redirect_uri: &str,
) -> Result<OAuthClientRegistration, (StatusCode, String)> {
    validate_safe_url(registration_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Registration endpoint blocked: {e}"),
        )
    })?;
    let response = reqwest::Client::new()
        .post(registration_endpoint)
        .json(&serde_json::json!({
            "client_name": "Everruns MCP",
            "redirect_uris": [redirect_uri],
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "token_endpoint_auth_method": "client_secret_post"
        }))
        .send()
        .await
        .map_err(|e| sanitized_bad_gateway("OAuth external service", &e))?;
    json_response_or_error(response).await
}

async fn exchange_oauth_code(
    token_endpoint: &str,
    client_id: &str,
    client_secret: Option<&str>,
    redirect_uri: &str,
    code: &str,
    code_verifier: &str,
) -> Result<OAuthTokenResponse, (StatusCode, String)> {
    validate_safe_url(token_endpoint).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Token endpoint blocked: {e}"),
        )
    })?;
    let mut params = vec![
        ("grant_type", "authorization_code".to_string()),
        ("client_id", client_id.to_string()),
        ("redirect_uri", redirect_uri.to_string()),
        ("code", code.to_string()),
        ("code_verifier", code_verifier.to_string()),
    ];
    if let Some(secret) = client_secret {
        params.push(("client_secret", secret.to_string()));
    }
    let response = reqwest::Client::new()
        .post(token_endpoint)
        .form(&params)
        .send()
        .await
        .map_err(|e| sanitized_bad_gateway("OAuth external service", &e))?;
    json_response_or_error(response).await
}

fn finalize_oauth_redirect(
    auth_config: &AuthConfig,
    return_to: &str,
    provider: &str,
    popup: bool,
) -> String {
    let frontend = auth_config.frontend_url.trim_end_matches('/');
    let encoded_provider = urlencoding::encode(provider);
    if popup {
        format!(
            "{}/connection-complete?provider={}&status=success&return_to={}",
            frontend,
            encoded_provider,
            urlencoding::encode(return_to),
        )
    } else if return_to.contains('?') {
        format!("{frontend}{return_to}&connected={encoded_provider}")
    } else {
        format!("{frontend}{return_to}?connected={encoded_provider}")
    }
}

fn normalize_oauth_mode(mode: Option<&str>) -> Result<String, (StatusCode, String)> {
    match mode.unwrap_or("user") {
        "user" => Ok("user".to_string()),
        "session" => Ok("session".to_string()),
        other => Err((
            StatusCode::BAD_REQUEST,
            format!("Invalid OAuth mode: {other}"),
        )),
    }
}

fn resource_origin(url: &Url) -> Result<String, (StatusCode, String)> {
    let host = url.host_str().ok_or((
        StatusCode::BAD_REQUEST,
        "Invalid MCP server URL: missing host".to_string(),
    ))?;
    let host = if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    };
    let mut origin = format!("{}://{}", url.scheme(), host);
    if let Some(port) = url.port() {
        origin.push(':');
        origin.push_str(&port.to_string());
    }
    Ok(origin)
}

fn parse_and_validate_url(url: &str) -> Result<Url, (StatusCode, String)> {
    validate_safe_url(url).map_err(|e| (StatusCode::BAD_REQUEST, format!("Blocked URL: {e}")))?;
    Url::parse(url).map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid URL: {e}")))
}

async fn json_response_or_error<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, (StatusCode, String)> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let truncated: &str = if body.len() > 512 {
            &body[..512]
        } else {
            &body
        };
        tracing::error!(
            status = %status,
            body_len = body.len(),
            "External service error: {truncated}"
        );
        return Err((StatusCode::BAD_GATEWAY, "Bad gateway".to_string()));
    }
    response
        .json()
        .await
        .map_err(|e| sanitized_bad_gateway("External service response parse", &e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_state_accepted() {
        let state_value = "abc123deadbeef";
        let jar = CookieJar::new().add(Cookie::new(
            GITHUB_INSTALL_STATE_COOKIE,
            state_value.to_string(),
        ));
        let result = validate_install_state(&jar, Some(state_value));
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), state_value);
    }

    #[test]
    fn missing_cookie_rejected() {
        let jar = CookieJar::new();
        let (status, msg) = validate_install_state(&jar, Some("abc123")).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("expired"));
    }

    #[test]
    fn missing_query_state_rejected() {
        let jar = CookieJar::new().add(Cookie::new(
            GITHUB_INSTALL_STATE_COOKIE,
            "abc123".to_string(),
        ));
        let (status, msg) = validate_install_state(&jar, None).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("Missing state"));
    }

    #[test]
    fn mismatched_state_rejected() {
        let jar = CookieJar::new().add(Cookie::new(
            GITHUB_INSTALL_STATE_COOKIE,
            "correct_state".to_string(),
        ));
        let (status, msg) = validate_install_state(&jar, Some("wrong_state")).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("Invalid installation state"));
    }

    #[test]
    fn both_missing_reports_expired() {
        let jar = CookieJar::new();
        // Cookie checked first — reports expired state
        let (status, _) = validate_install_state(&jar, None).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn state_cookie_has_secure_properties() {
        let state_value = "test_state";
        let cookie = Cookie::build((GITHUB_INSTALL_STATE_COOKIE, state_value))
            .path("/")
            .http_only(true)
            .secure(true)
            .same_site(SameSite::Lax)
            .max_age(time::Duration::minutes(10))
            .build();

        assert_eq!(cookie.name(), GITHUB_INSTALL_STATE_COOKIE);
        assert_eq!(cookie.value(), state_value);
        assert!(cookie.http_only().unwrap_or(false));
        assert!(cookie.secure().unwrap_or(false));
        assert_eq!(cookie.same_site(), Some(SameSite::Lax));
        assert_eq!(cookie.max_age(), Some(time::Duration::minutes(10)));
        assert_eq!(cookie.path(), Some("/"));
    }

    #[test]
    fn callback_query_deserialize_with_state() {
        let json = r#"{"installation_id": 12345, "state": "abc123"}"#;
        let query: GitHubInstallationCallbackQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.installation_id, 12345);
        assert_eq!(query.state, Some("abc123".to_string()));
    }

    #[test]
    fn callback_query_deserialize_without_state() {
        let json = r#"{"installation_id": 12345}"#;
        let query: GitHubInstallationCallbackQuery = serde_json::from_str(json).unwrap();
        assert_eq!(query.installation_id, 12345);
        assert_eq!(query.state, None);
    }

    // =========================================================================
    // GitHub installation callback security negative tests (EVE-54 / EVE-61)
    // =========================================================================

    #[test]
    fn empty_cookie_does_not_match_nonempty_query() {
        let jar = CookieJar::new().add(Cookie::new(GITHUB_INSTALL_STATE_COOKIE, "".to_string()));
        let result = validate_install_state(&jar, Some("attacker_state"));
        assert!(
            result.is_err(),
            "empty cookie must not match non-empty query"
        );
    }

    #[test]
    fn nonempty_cookie_does_not_match_empty_query() {
        let jar = CookieJar::new().add(Cookie::new(
            GITHUB_INSTALL_STATE_COOKIE,
            "real_state".to_string(),
        ));
        let result = validate_install_state(&jar, Some(""));
        assert!(
            result.is_err(),
            "non-empty cookie must not match empty query"
        );
    }

    #[test]
    fn whitespace_padded_state_rejected() {
        let jar = CookieJar::new().add(Cookie::new(
            GITHUB_INSTALL_STATE_COOKIE,
            "abc123".to_string(),
        ));
        let result = validate_install_state(&jar, Some(" abc123 "));
        assert!(result.is_err(), "whitespace-padded state must not match");
    }

    #[test]
    fn all_state_failures_return_bad_request() {
        let (status, _) = validate_install_state(&CookieJar::new(), Some("x")).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let jar = CookieJar::new().add(Cookie::new(GITHUB_INSTALL_STATE_COOKIE, "x".to_string()));
        let (status, _) = validate_install_state(&jar, None).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = validate_install_state(&jar, Some("y")).unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    // =========================================================================
    // VerifyConnectionResponse serialization tests
    // =========================================================================

    #[test]
    fn verify_response_valid_serializes_without_error() {
        let resp = VerifyConnectionResponse {
            valid: true,
            error: None,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["valid"], true);
        assert!(
            json.get("error").is_none(),
            "error field should be skipped when None"
        );
    }

    #[test]
    fn verify_response_invalid_serializes_with_error() {
        let resp = VerifyConnectionResponse {
            valid: false,
            error: Some("Invalid API key".to_string()),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["valid"], false);
        assert_eq!(json["error"], "Invalid API key");
    }

    #[test]
    fn normalize_oauth_mode_defaults_to_user() {
        assert_eq!(normalize_oauth_mode(None).unwrap(), "user");
        assert_eq!(normalize_oauth_mode(Some("session")).unwrap(), "session");
    }

    #[test]
    fn normalize_oauth_mode_rejects_unknown_values() {
        let err = normalize_oauth_mode(Some("admin")).unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn resource_origin_preserves_non_default_port() {
        let url = reqwest::Url::parse("https://example.com:8443/v1/mcp").unwrap();
        assert_eq!(resource_origin(&url).unwrap(), "https://example.com:8443");
    }

    #[test]
    fn resource_origin_brackets_ipv6_hosts() {
        let url = reqwest::Url::parse("https://[::1]:8443/v1/mcp").unwrap();
        assert_eq!(resource_origin(&url).unwrap(), "https://[::1]:8443");
    }
}
