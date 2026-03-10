// User Connections API routes
// Decision: User-scoped (not org-scoped) — token represents user's identity
// Decision: GitHub App installation flow replaces OAuth App for repo access
// Decision: API-key providers (Daytona etc.) register via ConnectionProviderPlugin
//   and define their own form schema + validation. Server discovers them at runtime.

use crate::auth::config::AuthConfig;
use crate::auth::middleware::{AuthState, AuthUser};
use crate::auth::oauth::GitHubAppService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{FromRef, Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{delete, get, post, put},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use chrono::{DateTime, Utc};
use everruns_core::connection_provider::{
    ConnectionFormSchema as CoreFormSchema, ConnectionProviderPlugin, ConnectionType,
};
use everruns_core::deployment::DeploymentGrade;
use rand::Rng;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use crate::storage::models::CreateUserConnectionRow;

/// App state for user connections routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    pub auth_config: AuthConfig,
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

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

/// Request body for API-key connection creation (plugin-based providers)
#[derive(Debug, Deserialize)]
pub struct CreateApiKeyConnectionRequest {
    pub api_key: String,
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
    _auth: AuthUser,
) -> Json<Vec<ProviderResponse>> {
    let grade = DeploymentGrade::from_env();
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

    // Plugin-registered providers (API-key based)
    for plugin in inventory::iter::<ConnectionProviderPlugin> {
        if plugin.experimental_only && !grade.experimental_features_enabled() {
            continue;
        }
        let provider = (plugin.factory)();
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
    let grade = DeploymentGrade::from_env();

    // Find the registered ConnectionProvider for this provider_id
    let provider: Option<Box<dyn everruns_core::connection_provider::ConnectionProvider>> =
        inventory::iter::<ConnectionProviderPlugin>
            .into_iter()
            .filter(|p| !p.experimental_only || grade.experimental_features_enabled())
            .map(|p| (p.factory)())
            .find(|p| p.provider_id() == provider_id);

    let provider = provider.ok_or_else(|| {
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

    // Validate the API key
    let validation: everruns_core::connection_provider::ConnectionValidation =
        provider.validate(&body.api_key).await.map_err(|e| {
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
    let grade = DeploymentGrade::from_env();

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

    // Find the provider plugin
    let provider: Option<Box<dyn everruns_core::connection_provider::ConnectionProvider>> =
        inventory::iter::<ConnectionProviderPlugin>
            .into_iter()
            .filter(|p| !p.experimental_only || grade.experimental_features_enabled())
            .map(|p| (p.factory)())
            .find(|p| p.provider_id() == provider_id);

    let provider = provider.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Unknown connection provider: {provider_id}"),
        )
    })?;

    // Call provider's validate()
    match provider.validate(&credential).await {
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
}
