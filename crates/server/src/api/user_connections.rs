// User Connections API routes
// Console adapters use the selected organization’s default virtual user.
// Decision: GitHub App installation flow replaces OAuth App for repo access
// Decision: API-key providers (Daytona etc.) register via ConnectorPlugin
//   and define their own form schema + validation. Server discovers them at runtime.

use crate::auth::ResolvedOrg;
use crate::auth::config::AuthConfig;
use crate::auth::middleware::{AuthState, AuthUser as ManagementUser};
use crate::auth::oauth::GitHubAppService;
use crate::domains::mcp_servers::McpServerService;
use crate::domains::plugins::oauth_anchor::humanize_connection_name;
use crate::kernel_imports::{
    Caller, McpServerAuthMode,
    contracts::typed_id::{AgentId, SessionId, VirtualUserId},
    contracts::url_validation::validate_safe_url,
    mcp_oauth_provider_id_for_uuid,
};
use crate::oauth_client::{OAuthCodeExchangeRequest, exchange_oauth_code};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect},
    routing::{delete, get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use everruns_contracts::connector::{
    ConnectorFormSchema as CoreFormSchema, ConnectorRegistry, ConnectorType,
};
use rand::RngExt;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::HashMap, sync::Arc};
use utoipa::ToSchema;

use super::common::{impl_auth_state, sanitized_internal_error};
use crate::domains::mcp_servers::MCP_SERVER_MANAGE;
use crate::domains::virtual_users::lifecycle::ensure_identity_for_agent;
use crate::storage::models::{
    CreateUserConnectionRow, CreateVirtualUserConnectionRow, UpsertMcpOAuthSessionCredentials,
};
pub mod mcp_connections;
use mcp_connections::list_mcp_connections;
mod mcp_oauth;
use mcp_oauth::{
    ensure_mcp_oauth_registration, oauth_refusal_message, validate_authorization_params,
};

/// App state for user connections routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub encryption: Option<Arc<EncryptionService>>,
    pub auth: AuthState,
    pub auth_config: AuthConfig,
    pub connectors: ConnectorRegistry,
    pub mcp_service: Arc<McpServerService>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        auth: AuthState,
        auth_config: AuthConfig,
        connectors: ConnectorRegistry,
        mcp_service: Arc<McpServerService>,
    ) -> Self {
        Self {
            db,
            encryption,
            auth,
            auth_config,
            connectors,
            mcp_service,
        }
    }
}

impl_auth_state!(AppState);

/// Console connections proxy the authenticated account's default runtime account
/// in its selected organization. Management authentication stays separate.
#[derive(Debug, Clone)]
pub struct ConnectionUser {
    pub id: uuid::Uuid,
    pub management_user_id: uuid::Uuid,
    pub org_id: i64,
}
impl<S> axum::extract::FromRequestParts<S> for ConnectionUser
where
    S: Send + Sync,
    AuthState: axum::extract::FromRef<S>,
{
    type Rejection = crate::auth::middleware::AuthError;
    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        use axum::extract::FromRef;
        let management = ManagementUser::from_request_parts(parts, state).await?;
        let org = ResolvedOrg::from_request_parts(parts, state).await?;
        let auth = AuthState::from_ref(state);
        let db = auth
            .db
            .as_ref()
            .ok_or_else(|| Self::Rejection::internal("Runtime identity unavailable"))?;
        let user = db
            .default_virtual_user(org.org_id, management.id)
            .await
            .map_err(|_| Self::Rejection::internal("Runtime identity unavailable"))?;
        if user.status != "active" {
            return Err(Self::Rejection::forbidden("Runtime account is not active"));
        }
        Ok(Self {
            id: user.id.uuid(),
            management_user_id: management.id,
            org_id: org.org_id,
        })
    }
}

// ============================================================================
// Response / Request Types
// ============================================================================

/// Connection info returned in API responses (never includes token)
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = Connection)]
pub struct ConnectionResponse {
    /// Stable provider identifier.
    #[schema(example = "github")]
    pub provider: String,
    /// Credential mechanism used by this connection.
    #[schema(example = "oauth")]
    pub connection_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Display name on the provider.
    #[schema(example = "octocat")]
    pub provider_username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Provider scopes granted to this account.
    #[schema(example = "read:user")]
    pub scopes: Option<String>,
    /// Time the grant was created.
    #[schema(example = "2026-09-29T12:00:00Z")]
    pub connected_at: DateTime<Utc>,
}

/// Provider info for the connections UI
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ProviderResponse {
    /// Stable provider identifier.
    #[schema(example = "github")]
    pub provider_id: String,
    /// Provider display name.
    #[schema(example = "GitHub")]
    pub display_name: String,
    /// Provider description.
    #[schema(example = "Connect your repositories")]
    pub description: String,
    /// Provider icon name.
    #[schema(example = "github")]
    pub icon: String,
    /// Provider credential mechanism.
    #[schema(example = "oauth")]
    pub connection_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Fields required for API key providers.
    pub form_schema: Option<FormSchemaResponse>,
}

/// Form schema for API-key providers
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FormSchemaResponse {
    /// Provider form fields.
    pub fields: Vec<FormFieldResponse>,
    /// Instructions shown before connection setup.
    #[schema(example = "Create an API key in provider settings.")]
    pub instructions_markdown: String,
}

/// Single form field
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct FormFieldResponse {
    /// Provider form field name.
    #[schema(example = "api_key")]
    pub name: String,
    /// Provider form field label.
    #[schema(example = "API key")]
    pub label: String,
    /// Input type used by the form.
    #[schema(example = "password")]
    pub field_type: String,
    /// Whether the field must be supplied.
    #[schema(example = true)]
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Suggested input placeholder.
    #[schema(example = "Enter your API key")]
    pub placeholder: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Explanation for this form field.
    #[schema(example = "Your key is stored encrypted.")]
    pub help_text: Option<String>,
}

/// Request body for API-key connection creation (plugin-based providers).
/// Accepts api_key plus any additional form fields as extra_fields.
#[derive(Debug, Deserialize, ToSchema)]
#[schema(as = CreateConnectionRequest)]
pub struct CreateApiKeyConnectionRequest {
    /// Provider API key, encrypted before storage.
    #[schema(example = "example-api-key")]
    pub api_key: String,
    /// Additional provider-specific form fields (e.g. org_slug for Deno personal tokens).
    #[serde(flatten)]
    pub extra_fields: std::collections::HashMap<String, serde_json::Value>,
}

/// Request body for API-key-based connections (e.g., Brave Search)
#[derive(Debug, Deserialize, ToSchema)]
pub struct ApiKeyConnectionRequest {
    /// Provider API key, encrypted before storage.
    #[schema(example = "example-api-key")]
    pub api_key: String,
}

/// GitHub App installation callback query params
#[derive(Debug, Deserialize)]
pub struct GitHubInstallationCallbackQuery {
    pub installation_id: i64,
    #[allow(dead_code)]
    pub setup_action: Option<String>,
    pub state: Option<String>,
}

/// Browser setup options. The canonical target is selected by the authorized resource path.
#[derive(Debug, Deserialize, ToSchema)]
pub struct OAuthAuthorizeQuery {
    /// Same-origin return path after setup.
    #[schema(example = "/settings/connections")]
    pub return_to: Option<String>,
    /// Legacy setup mode; canonical resource paths capture the target.
    #[schema(example = "virtual_user")]
    pub mode: Option<String>,
    /// Optional session grant destination for legacy setup.
    pub session_id: Option<String>,
    /// Required when `mode = identity`: the agent whose service grant this is.
    /// The grant is owned by the agent's identity, not by the admin who
    /// authorizes it (EVE-1030).
    /// Optional legacy service agent target.
    pub agent_id: Option<String>,
    /// Whether setup completes in a popup.
    #[schema(example = false)]
    pub popup: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingOAuthState {
    state: String,
    org_id: i64,
    management_user_id: Option<uuid::Uuid>,
    #[serde(default)]
    runtime_credential: Option<String>,
    provider: String,
    return_to: String,
    mode: String,
    session_id: Option<String>,
    /// Set only for `mode = identity`. These bind the callback to the exact
    /// agent and identity resolved before the redirect. The callback atomically
    /// requires both to remain active, linked, and in the authorized org.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    virtual_user_id: Option<String>,
    popup: bool,
    code_verifier: String,
}

// ============================================================================
// Routes
// ============================================================================

/// Create user connections routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/connection-providers", get(list_runtime_connectors))
        .route(
            "/v1/user/connection-migrations",
            get(list_pending_connection_migrations),
        )
        .route(
            "/v1/user/connection-migrations/{connection_id}",
            post(migrate_pending_connection),
        )
        .route(
            "/v1/virtual-users/{identity_id}/connections/{provider}/authorize",
            get(authorize_target_connection).post(start_target_connection),
        )
        .route(
            "/v1/virtual-users/me/mcp-connections",
            get(list_mcp_connections),
        )
        .route("/v1/user/connections", get(list_connections))
        .route("/v1/user/mcp-connections", get(list_mcp_connections))
        .route("/v1/user/connections/providers", get(list_connectors))
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
            "/v1/connection-callbacks/{provider}",
            get(connection_oauth_callback),
        )
        .route("/v1/connection-callbacks/github", get(github_callback))
        .with_state(state)
}

// ============================================================================
// Handlers
// ============================================================================

/// GET /v1/user/connections — List user's connected accounts
pub async fn list_connections(
    State(state): State<AppState>,
    auth: ConnectionUser,
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

/// GET /v1/user/connections/providers — List available connectors
///
/// Returns both hardcoded connectors (GitHub/OAuth) and plugin-registered
/// connectors (Daytona/API-key). Frontend uses this to render connection forms.
pub async fn list_connectors(
    State(state): State<AppState>,
    org: ResolvedOrg,
) -> Json<Vec<ProviderResponse>> {
    list_connectors_for_org(&state, org.org_id).await
}
#[utoipa::path(summary = "List providers available to this runtime account and endpoint.", get, path="/v1/connection-providers", responses((status=200,description="Available connection providers",body=Vec<ProviderResponse>)),tag="virtual-users")]
async fn list_runtime_connectors(
    State(state): State<AppState>,
    account: crate::auth::runtime::RuntimeAccount,
) -> Result<Json<Vec<ProviderResponse>>, StatusCode> {
    let allowed = account
        .allowed_mcp_providers(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let Json(mut providers) = list_connectors_for_org(&state, account.org_id).await;
    if let Some(allowed) = allowed {
        providers.retain(|p| {
            !p.provider_id.starts_with("mcp_oauth_") || allowed.contains(&p.provider_id)
        });
    }
    Ok(Json(providers))
}
async fn list_connectors_for_org(state: &AppState, org_id: i64) -> Json<Vec<ProviderResponse>> {
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
    for provider in state.connectors.list() {
        let form_schema = provider.form_schema().map(|s| form_schema_to_response(&s));
        let conn_type = match provider.connection_type() {
            ConnectorType::OAuth => "oauth",
            ConnectorType::ApiKey => "api_key",
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

    let plugin_display_names = match state.db.list_active_plugin_installs(org_id).await {
        Ok(installs) => installs
            .into_iter()
            .map(|install| {
                let display_name = serde_json::from_value::<
                    everruns_core::DeclarativeCapabilityDefinition,
                >(install.definition.clone())
                .ok()
                .and_then(|definition| definition.display_name)
                .or_else(|| {
                    install
                        .manifest
                        .get("displayName")
                        .or_else(|| install.manifest.get("display_name"))
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
                .unwrap_or_else(|| humanize_connection_name(&install.name));
                (install.name, display_name)
            })
            .collect::<HashMap<_, _>>(),
        Err(error) => {
            tracing::error!(%error, org_id = org_id, "Failed to list plugin connection labels");
            HashMap::new()
        }
    };

    match state.db.list_mcp_servers(org_id, None, false).await {
        Ok(servers) => {
            providers.extend(servers.into_iter().filter_map(|server| {
                let settings = McpServerService::settings_from_row(&server);
                (settings.auth_mode == McpServerAuthMode::OAuth).then(|| ProviderResponse {
                    provider_id: mcp_oauth_provider_id_for_uuid(server.id.uuid()),
                    display_name: mcp_connection_display_name(
                        &server.settings,
                        &server.name,
                        &plugin_display_names,
                    ),
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
            tracing::error!(%error, org_id = org_id, "Failed to list MCP OAuth providers");
        }
    }

    Json(providers)
}

fn mcp_connection_display_name(
    settings: &serde_json::Value,
    fallback: &str,
    plugin_display_names: &HashMap<String, String>,
) -> String {
    if let Some(display_name) = settings
        .get(crate::domains::plugins::oauth_anchor::CONNECTION_DISPLAY_NAME_KEY)
        .and_then(|value| value.as_str())
    {
        return display_name.to_string();
    }

    let Some(anchor) = settings.get("plugin_anchor") else {
        return fallback.to_string();
    };
    let Some(plugin_name) = anchor.get("plugin").and_then(|value| value.as_str()) else {
        return fallback.to_string();
    };
    let plugin_display_name = plugin_display_names
        .get(plugin_name)
        .cloned()
        .unwrap_or_else(|| humanize_connection_name(plugin_name));
    match anchor.get("server").and_then(|value| value.as_str()) {
        Some(server_name) if server_name != plugin_name => {
            format!("{plugin_display_name} — {server_name}")
        }
        _ => plugin_display_name,
    }
}

/// POST /v1/user/connections/:provider — Create API-key connection
///
/// For providers that use direct API key entry (not OAuth).
/// Validates the key via the provider's validate() method before saving.
pub async fn create_api_key_connection(
    State(state): State<AppState>,
    auth: ConnectionUser,
    Path(provider_id): Path<String>,
    Json(body): Json<CreateApiKeyConnectionRequest>,
) -> Result<(StatusCode, Json<ConnectionResponse>), (StatusCode, String)> {
    let provider = state.connectors.get(&provider_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Unknown connector: {provider_id}"),
        )
    })?;

    // Only API-key providers support direct creation
    if provider.connection_type() != ConnectorType::ApiKey {
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
    let validation: everruns_contracts::connector::ConnectorValidation =
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
    auth: ConnectionUser,
    Path(provider): Path<String>,
) -> Result<StatusCode, StatusCode> {
    state
        .db
        .delete_user_connection(auth.id, &provider)
        .await
        .map_err(|e| {
            tracing::error!("Failed to delete connection: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(StatusCode::NO_CONTENT)
}

/// Response for connection verification
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct VerifyConnectionResponse {
    /// Whether the provider accepted the saved credential.
    #[schema(example = true)]
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Provider verification failure without credential details.
    #[schema(example = "Credential expired")]
    pub error: Option<String>,
}

/// POST /v1/user/connections/:provider/verify — Verify stored API key still works
///
/// Generic endpoint: decrypts the stored credential and calls the provider's
/// validate() method. Works for any API-key provider that implements
/// Connector::validate().
pub async fn verify_connection(
    State(state): State<AppState>,
    auth: ConnectionUser,
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

    let provider = state.connectors.get(&provider_id).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Unknown connector: {provider_id}"),
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
    auth: ConnectionUser,
    jar: CookieJar,
    Path(provider): Path<String>,
    Query(query): Query<OAuthAuthorizeQuery>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    authorize_connection_inner(
        state,
        OAuthAuthority {
            org_id: org.org_id,
            caller: Some(Caller::from(&org)),
            target_id: auth.id,
            management_user_id: Some(auth.management_user_id),
            runtime_credential: None,
        },
        jar,
        provider,
        query,
    )
    .await
}
struct OAuthAuthority {
    org_id: i64,
    caller: Option<Caller>,
    target_id: uuid::Uuid,
    management_user_id: Option<uuid::Uuid>,
    runtime_credential: Option<String>,
}
impl OAuthAuthority {
    fn management_caller(&self) -> Result<&Caller, (StatusCode, String)> {
        self.caller.as_ref().ok_or((
            StatusCode::FORBIDDEN,
            "Management authority required".into(),
        ))
    }
}
async fn authorize_connection_inner(
    state: AppState,
    authority: OAuthAuthority,
    jar: CookieJar,
    provider: String,
    query: OAuthAuthorizeQuery,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let Some(server_id) = parse_mcp_oauth_provider_id(&provider) else {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Unknown OAuth provider: {provider}"),
        ));
    };
    let row = state
        .db
        .get_mcp_server(authority.org_id, server_id)
        .await
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "MCP server not found".to_string()))?;
    if row.status != "active" {
        return Err((
            StatusCode::BAD_REQUEST,
            "MCP server is not active".to_string(),
        ));
    }
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

    // Identity mode authorizes a grant the agent owns: one credential shared by
    // every session and every invoking user. That is a different privilege from
    // connecting your own account, so it is gated and resolved here, before the
    // redirect. The callback writes only while this exact agent remains linked
    // to the identity that was authorized (EVE-1030).
    let identity_agent = match mode.as_str() {
        "identity" => {
            let agent_public_id = query.agent_id.ok_or((
                StatusCode::BAD_REQUEST,
                "agent_id is required for identity OAuth flows".to_string(),
            ))?;
            let caller = authority.management_caller()?;
            // THREAT[TM-AUTHZ-018]: service grants require MCP management
            // authority before any discovery, registration, or identity write.
            enforce_identity_grant_policy(&state, caller)?;

            let agent = state
                .db
                .get_agent_by_public_id(authority.org_id, &agent_public_id)
                .await
                .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
                .ok_or((StatusCode::NOT_FOUND, "Agent not found".to_string()))?;
            if agent.status != "active" {
                return Err((StatusCode::BAD_REQUEST, "Agent is not active".to_string()));
            }
            Some(agent)
        }
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
            authority.org_id,
            server_id,
            crate::storage::models::UpdateMcpServer {
                settings: Some(serde_json::to_value(&updated_settings).unwrap_or_default()),
                ..Default::default()
            },
        )
        .await
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;

    let (agent_id, virtual_user_id) = match identity_agent {
        Some(agent) => {
            let agent_id = agent.id.to_string();
            let (identity_id, _principal) =
                ensure_identity_for_agent(&state.db, authority.org_id, &agent)
                    .await
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
            (Some(agent_id), Some(identity_id.to_string()))
        }
        None => (
            None,
            Some(
                everruns_contracts::typed_id::VirtualUserId::from_uuid(authority.target_id)
                    .to_string(),
            ),
        ),
    };

    let service_grant = mode == "identity";
    let pending = PendingOAuthState {
        state: oauth_state.clone(),
        org_id: authority.org_id,
        management_user_id: authority.management_user_id,
        runtime_credential: authority.runtime_credential,
        provider: provider.clone(),
        return_to,
        mode,
        session_id,
        virtual_user_id,
        agent_id,
        popup,
        code_verifier,
    };
    register_pending_setup(&state, &pending).await?;
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
    let oauth = updated_settings.oauth.as_ref().ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth settings missing for MCP server".to_string(),
    ))?;
    let scopes = oauth.scope.clone().unwrap_or_else(|| {
        if !metadata.scopes_supported.is_empty() {
            metadata.scopes_supported.join(" ")
        } else {
            "email".to_string()
        }
    });
    let redirect_uri = mcp_oauth_redirect_uri(&state.auth_config, &provider);
    let mut url = authorize_url;
    {
        let mut pairs = url.query_pairs_mut();
        pairs
            .append_pair("response_type", "code")
            .append_pair("client_id", &registration.client_id)
            .append_pair("redirect_uri", &redirect_uri)
            .append_pair("scope", &scopes)
            .append_pair("state", &oauth_state)
            .append_pair("code_challenge", &code_challenge)
            .append_pair("code_challenge_method", "S256");
        if let Some(resource) = oauth.resource.as_deref() {
            pairs.append_pair("resource", resource);
        }
        if service_grant {
            validate_authorization_params(&oauth.service_authorization_params)?;
            for (key, value) in &oauth.service_authorization_params {
                pairs.append_pair(key, value);
            }
        }
    }

    Ok((jar.add(cookie), Redirect::to(url.as_str())))
}

pub async fn connection_oauth_callback(
    State(state): State<AppState>,
    org: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
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
    consume_pending_setup(&state, &pending).await?;
    let authority = callback_authority(&state, &pending, org).await?;
    let clear_cookie = jar.remove(Cookie::from(oauth_state_cookie_name(&provider)));
    if let Some(error) = query.error.as_deref() {
        return Err((
            StatusCode::BAD_REQUEST,
            oauth_refusal_message(error, query.error_description.as_deref()),
        ));
    }
    let code = query.code.as_deref().ok_or((
        StatusCode::BAD_REQUEST,
        "OAuth callback is missing the authorization code".to_string(),
    ))?;

    let row = state
        .db
        .get_mcp_server(authority.org_id, server_id)
        .await
        .map_err(|e| sanitized_internal_error("OAuth connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "MCP server not found".to_string()))?;
    if row.status != "active" {
        return Err((
            StatusCode::BAD_REQUEST,
            "MCP server is not active".to_string(),
        ));
    }
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
        state.mcp_service.egress_service().as_ref(),
        OAuthCodeExchangeRequest {
            token_endpoint: &token_endpoint,
            client_id: &client_id,
            client_secret: client_secret.as_deref(),
            redirect_uri: &redirect_uri,
            code,
            code_verifier: &pending.code_verifier,
            resource: oauth.resource.as_deref(),
        },
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
            .get_session(authority.org_id, session_id)
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
            .upsert_mcp_oauth_session_credentials(UpsertMcpOAuthSessionCredentials {
                virtual_user_id: pending
                    .virtual_user_id
                    .as_deref()
                    .and_then(|id| id.parse::<VirtualUserId>().ok()),
                session_id,
                server_id,
                access_token_encrypted: encryption
                    .encrypt_string(&token.access_token)
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                refresh_token_encrypted: token
                    .refresh_token
                    .as_deref()
                    .map(|value| encryption.encrypt_string(value))
                    .transpose()
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
                expires_at_encrypted: expires_at
                    .map(|value| encryption.encrypt_string(&value.to_rfc3339()))
                    .transpose()
                    .map_err(|e| sanitized_internal_error("OAuth connection", &e))?,
            })
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
    } else if pending.mode == "identity" {
        let agent_id = pending
            .agent_id
            .as_deref()
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing agent_id for identity OAuth flow".to_string(),
            ))?
            .parse::<AgentId>()
            .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid agent_id: {e}")))?;
        let identity_id = pending
            .virtual_user_id
            .as_deref()
            .ok_or((
                StatusCode::BAD_REQUEST,
                "Missing virtual_user_id for identity OAuth flow".to_string(),
            ))?
            .parse::<VirtualUserId>()
            .map_err(|e| {
                (
                    StatusCode::BAD_REQUEST,
                    format!("Invalid virtual_user_id: {e}"),
                )
            })?;

        // Re-check both the permission and org ownership here rather than
        // trusting the cookie. The state cookie is browser-bound but not
        // signed, so a planted one must not be able to aim a grant at another
        // tenant's identity or clear a gate the authorizing user never passed.
        let caller = authority.management_caller()?;
        enforce_identity_grant_policy(&state, caller)?;

        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        ))?;
        let connection = state
            .db
            .upsert_virtual_user_connection_for_active_agent(
                authority.org_id,
                agent_id,
                CreateVirtualUserConnectionRow {
                    virtual_user_id: identity_id,
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
                },
            )
            .await
            .map_err(|e| sanitized_internal_error("OAuth connection", &e))?;
        if connection.is_none() {
            return Err((
                StatusCode::BAD_REQUEST,
                "Agent is no longer active with the authorized identity".to_string(),
            ));
        }
    } else {
        let encryption = state.encryption.as_ref().ok_or((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured".to_string(),
        ))?;
        state
            .db
            .upsert_user_connection(CreateUserConnectionRow {
                user_id: pending
                    .virtual_user_id
                    .as_deref()
                    .ok_or((
                        StatusCode::BAD_REQUEST,
                        "Missing runtime subject".to_string(),
                    ))?
                    .parse::<VirtualUserId>()
                    .map_err(|_| {
                        (
                            StatusCode::BAD_REQUEST,
                            "Invalid runtime subject".to_string(),
                        )
                    })?
                    .uuid(),
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
    _auth: ConnectionUser,
    jar: CookieJar,
    Query(_params): Query<std::collections::HashMap<String, String>>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    github_authorize_inner(
        state,
        OAuthAuthority {
            org_id: _auth.org_id,
            caller: None,
            target_id: _auth.id,
            management_user_id: Some(_auth.management_user_id),
            runtime_credential: None,
        },
        jar,
        None,
    )
    .await
}
async fn github_authorize_inner(
    state: AppState,
    authority: OAuthAuthority,
    jar: CookieJar,
    return_to: Option<String>,
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
    let bytes: [u8; 16] = rand::rng().random();
    let install_state = hex::encode(bytes);
    let pending = PendingOAuthState {
        state: install_state.clone(),
        org_id: authority.org_id,
        management_user_id: authority.management_user_id,
        runtime_credential: authority.runtime_credential,
        provider: "github".into(),
        return_to: normalize_return_to(
            return_to.as_deref(),
            "/settings/connections?connected=github",
        ),
        mode: "virtual_user".into(),
        session_id: None,
        agent_id: None,
        virtual_user_id: Some(VirtualUserId::from_uuid(authority.target_id).to_string()),
        popup: false,
        code_verifier: String::new(),
    };
    register_pending_setup(&state, &pending).await?;
    let state_cookie = Cookie::build((
        oauth_state_cookie_name("github"),
        URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&pending)
                .map_err(|e| sanitized_internal_error("GitHub setup", &e))?,
        ),
    ))
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
    org: Result<ResolvedOrg, crate::auth::middleware::AuthError>,
    jar: CookieJar,
    Query(query): Query<GitHubInstallationCallbackQuery>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let pending = validate_pending_oauth_state(&jar, "github", query.state.as_deref())?;
    consume_pending_setup(&state, &pending).await?;
    let auth = callback_authority(&state, &pending, org).await?;
    let jar = jar.remove(Cookie::from(oauth_state_cookie_name("github")));
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

    // Prevent installation hijacking across users: an installation already linked
    // to another user must not be claimable via callback replay/forgery.
    if let Some(existing_owner_id) = state
        .db
        .get_user_id_by_installation_id("github", result.installation_id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to resolve GitHub installation owner: {}", e);
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Failed to store connection".to_string(),
            )
        })?
        && existing_owner_id != auth.target_id
    {
        tracing::warn!(
            user_id = %auth.target_id,
            existing_owner_id = %existing_owner_id,
            installation_id = result.installation_id,
            "GitHub installation already linked to another user"
        );
        return Err((
            StatusCode::CONFLICT,
            "GitHub installation is already linked to another user".to_string(),
        ));
    }

    // Store installation_id (no OAuth token needed — tokens minted on demand)
    state
        .db
        .upsert_user_connection(CreateUserConnectionRow {
            user_id: auth.target_id,
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

    let frontend_url = state.auth_config.frontend_url.trim_end_matches('/');
    Ok((
        jar,
        Redirect::to(&format!("{}{}", frontend_url, pending.return_to)),
    ))
}

pub(crate) mod runtime_accounts;
pub use runtime_accounts::ConnectionSetupResponse;
use runtime_accounts::*;

#[cfg(test)]
#[path = "user_connections_tests.rs"]
mod tests;
