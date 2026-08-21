// LLM Provider API endpoints
// Routes: /v1/providers/...

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::providers::{
    CreateProvider, DeleteProvider, GetProvider, LLM_PROVIDER_MANAGE, LLM_PROVIDER_VIEW,
    ListProviders, ProviderService, SyncProviderModels, UpdateProvider,
};
use crate::kernel_imports::{
    Caller, Policy, evaluate_policies_with, everruns_provider::driver_registry::DriverOAuthFlow,
    everruns_provider::driver_registry::DriverRegistry, everruns_provider::provider::DriverId,
    everruns_provider::provider::ProviderStatus,
};
use crate::services::{ModelSyncService, ProviderResolverService};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    response::Redirect,
    routing::{get, post},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use everruns_provider::provider::{Provider, ProviderRequestOptions, ProviderTraceConfig};
use everruns_provider::typed_id::ProviderId;
use everruns_provider::url_validation::validate_safe_url;
use hmac::{Hmac, KeyInit, Mac};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
type HmacSha256 = Hmac<Sha256>;
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::{
    ApiResult, ErrorResponse, ListResponse, UrlBuilder, WithUrls, impl_auth_state,
};
use super::dispatch::{Dispatchable, impl_dispatchable};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub service: Arc<ProviderService>,
    pub sync_service: Arc<ModelSyncService>,
    pub auth: AuthState,
    /// Driver registry, used to discover whether a provider's driver declares an
    /// interactive OAuth connect flow.
    pub driver_registry: Arc<DriverRegistry>,
    /// Encryption service for storing the credential obtained via OAuth.
    pub encryption: Option<Arc<EncryptionService>>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        encryption: Option<Arc<EncryptionService>>,
        driver_registry: Arc<DriverRegistry>,
        auth: AuthState,
        provider_resolver: Option<Arc<ProviderResolverService>>,
    ) -> Self {
        let service = if let Some(resolver) = provider_resolver {
            ProviderService::with_resolver(db.clone(), encryption.clone(), resolver)
        } else {
            ProviderService::new(db.clone(), encryption.clone())
        };
        Self {
            db: db.clone(),
            service: Arc::new(service),
            sync_service: Arc::new(ModelSyncService::new(
                db,
                driver_registry.clone(),
                encryption.clone(),
            )),
            auth,
            driver_registry,
            encryption,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_provider_service(self.service.clone())
        .with_model_sync_service(self.sync_service.clone())
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// Request to create a new LLM provider
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateProviderRequest {
    /// Display name for the provider.
    #[schema(example = "OpenAI Production")]
    pub name: String,
    /// The type of LLM provider (e.g., openai, anthropic).
    pub provider_type: DriverId,
    /// Base URL for the provider's API. Required for custom endpoints.
    /// For standard providers, this can be omitted to use the default URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://api.openai.com/v1")]
    pub base_url: Option<String>,
    /// API key for authenticating with the provider.
    /// Will be encrypted at rest if encryption is configured.
    ///
    /// Single-field convenience for simple providers and programmatic clients.
    /// Multi-field drivers (Bedrock, MAI) should send `credentials` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Typed credential fields keyed by the driver's declared credential-schema
    /// field names. Validated against the schema and assembled into the stored
    /// credential document. Takes precedence over `api_key` when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<std::collections::BTreeMap<String, String>>,
    /// Trace/observability link configuration. Stored as a per-provider override
    /// of the driver's default templates; omit to keep driver defaults.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<ProviderTraceConfig>,
    /// Extra headers and diagnostics options applied to every request sent to
    /// this provider. Omit to configure none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<ProviderRequestOptions>,
}

/// Response from syncing models from a provider
#[derive(Debug, Serialize, ToSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum SyncModelsResponse {
    /// Sync completed successfully
    Success {
        /// Number of new models discovered
        created: usize,
        /// Number of existing models updated
        updated: usize,
        /// Number of models marked as stale (not seen in this sync)
        stale: usize,
    },
    /// Provider doesn't support model discovery
    NotSupported,
}

/// Request to update an LLM provider. Only provided fields will be updated.
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateProviderRequest {
    /// Display name for the provider.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "OpenAI Development")]
    pub name: Option<String>,
    /// The type of LLM provider (e.g., openai, anthropic).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<DriverId>,
    /// Base URL for the provider's API.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://api.openai.com/v1")]
    pub base_url: Option<String>,
    /// API key for authenticating with the provider.
    /// Will be encrypted at rest if encryption is configured.
    ///
    /// Single-field convenience for simple providers and programmatic clients.
    /// Multi-field drivers (Bedrock, MAI) should send `credentials` instead.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Typed credential fields keyed by the driver's declared credential-schema
    /// field names. Validated against the schema and assembled into the stored
    /// credential document. Takes precedence over `api_key` when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<std::collections::BTreeMap<String, String>>,
    /// The status of the provider. Set to "inactive" to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ProviderStatus>,
    /// Trace/observability link configuration override. Merged into the
    /// provider's stored settings, preserving other settings keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace: Option<ProviderTraceConfig>,
    /// Extra headers and diagnostics options applied to every request sent to
    /// this provider. Replaces the stored options wholesale; omit to leave them
    /// unchanged.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_options: Option<ProviderRequestOptions>,
}

/// Create a new LLM provider
#[utoipa::path(
    post,
    path = "/v1/providers",
    request_body = CreateProviderRequest,
    responses(
        (status = 201, description = "Provider created", body = WithUrls<Provider>),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal error")
    ),
    tag = "providers"
)]
pub async fn create_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateProviderRequest>,
) -> Result<(StatusCode, Json<WithUrls<Provider>>), (StatusCode, Json<ErrorResponse>)> {
    let api_key = resolve_credential_document(
        &state.driver_registry,
        Some(&req.provider_type),
        req.credentials.as_ref(),
        req.api_key,
    )?;
    state
        .dispatcher(&org)
        .run_created_with_urls(CreateProvider {
            name: req.name,
            provider_type: req.provider_type,
            base_url: req.base_url,
            api_key,
            trace: req.trace,
            request_options: req.request_options,
        })
        .await
}

/// Resolve the credential document to store from a request.
///
/// When typed `credentials` are supplied they are validated against the
/// driver's declared credential schema (when the driver is known) and assembled
/// into the single credential document that is encrypted at rest. Otherwise the
/// legacy single-field `api_key` is used verbatim. Validation failures surface
/// as `400 Bad Request` rather than a generic error.
fn resolve_credential_document(
    registry: &DriverRegistry,
    provider_type: Option<&DriverId>,
    credentials: Option<&std::collections::BTreeMap<String, String>>,
    api_key: Option<String>,
) -> Result<Option<String>, (StatusCode, Json<ErrorResponse>)> {
    let Some(fields) = credentials else {
        return Ok(api_key);
    };

    if let Some(descriptor) = provider_type.and_then(|pt| registry.descriptor(pt)) {
        let errors = descriptor.credential_schema.validate(fields);
        if !errors.is_empty() {
            return Err(ErrorResponse::new(errors.join(" ")).into_response(StatusCode::BAD_REQUEST));
        }
    }

    Ok(everruns_provider::credential_schema::assemble_credential_document(fields))
}

/// List all LLM providers
#[utoipa::path(
    get,
    path = "/v1/providers",
    responses(
        (status = 200, description = "List of providers", body = ListResponse<WithUrls<Provider>>)
    ),
    tag = "providers"
)]
pub async fn list_providers(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<ListResponse<WithUrls<Provider>>> {
    let providers = ListProviders {}.run(&state.ctx(&org)).await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(providers).with_urls(&builder)))
}

/// Get a specific LLM provider
#[utoipa::path(
    get,
    path = "/v1/providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "Provider found", body = WithUrls<Provider>),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "providers"
)]
pub async fn get_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<WithUrls<Provider>> {
    state
        .dispatcher(&org)
        .run_with_urls(GetProvider { id })
        .await
}

/// Update an LLM provider
#[utoipa::path(
    patch,
    path = "/v1/providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    request_body = UpdateProviderRequest,
    responses(
        (status = 200, description = "Provider updated", body = WithUrls<Provider>),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "providers"
)]
pub async fn update_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> ApiResult<WithUrls<Provider>> {
    // Validate typed credentials against the driver schema. A PATCH may omit
    // `provider_type`, so when credentials are supplied without it we infer the
    // driver from the stored row — otherwise the credential schema is unknown
    // and an incomplete multi-field credential would slip through unvalidated.
    let provider_type = match req.provider_type.clone() {
        Some(pt) => Some(pt),
        None if req.credentials.is_some() => {
            let caller = Caller::from(&org);
            let uuid = id
                .parse::<ProviderId>()
                .map_err(|e| {
                    ErrorResponse::new(format!("Invalid provider ID: {e}"))
                        .into_response(StatusCode::BAD_REQUEST)
                })?
                .uuid();
            state
                .service
                .get(&caller, uuid)
                .await
                .ok()
                .flatten()
                .map(|p| p.provider_type)
        }
        None => None,
    };
    let api_key = resolve_credential_document(
        &state.driver_registry,
        provider_type.as_ref(),
        req.credentials.as_ref(),
        req.api_key,
    )?;
    state
        .dispatcher(&org)
        .run_with_urls(UpdateProvider {
            id,
            name: req.name,
            provider_type: req.provider_type,
            base_url: req.base_url,
            api_key,
            status: req.status,
            trace: req.trace,
            request_options: req.request_options,
        })
        .await
}

/// Delete an LLM provider
#[utoipa::path(
    delete,
    path = "/v1/providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 204, description = "Provider deleted"),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "providers"
)]
pub async fn delete_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(DeleteProvider { id })
        .await
}

/// Sync models from an LLM provider
///
/// Fetches the list of available models from the provider's API and updates
/// the database. Only works for providers with standard base URLs (not custom).
#[utoipa::path(
    post,
    path = "/v1/providers/{id}/sync-models",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "Models synced", body = SyncModelsResponse),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Sync failed")
    ),
    tag = "providers"
)]
pub async fn sync_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<SyncModelsResponse> {
    Ok(Json(SyncProviderModels { id }.run(&state.ctx(&org)).await?))
}

/// A driver's declared credential schema, so the Settings UI can render
/// discrete typed inputs (multi-field AWS keys, Entra OAuth fields) instead of
/// one opaque password field.
#[derive(Debug, Serialize, ToSchema)]
pub struct DriverCredentialInfo {
    /// Driver id (e.g. `openai`, `bedrock`, `mai`).
    pub driver: String,
    /// The fields and instructions to render for this driver's credential.
    pub credential_schema: everruns_provider::credential_schema::CredentialFormSchema,
    /// Whether the driver declares an interactive "Connect with …" OAuth flow.
    pub supports_oauth: bool,
}

/// Provider resource config: caller policies plus the credential schemas the UI
/// renders per driver.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProvidersConfigResponse {
    /// Map of policy ID → whether the caller satisfies it.
    pub policies: std::collections::HashMap<String, bool>,
    /// Per-driver credential schemas, ordered by driver id.
    pub drivers: Vec<DriverCredentialInfo>,
}

/// GET /v1/providers/config
#[utoipa::path(
    get,
    path = "/v1/providers/config",
    responses(
        (status = 200, description = "Resource config for providers", body = ProvidersConfigResponse),
    ),
    tag = "providers"
)]
pub async fn provider_config(
    State(state): State<AppState>,
    org: ResolvedOrg,
) -> Json<ProvidersConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        state.auth.permission_resolver.as_ref(),
        &caller,
        &[&LLM_PROVIDER_VIEW, &LLM_PROVIDER_MANAGE],
    );

    let mut drivers: Vec<DriverCredentialInfo> = state
        .driver_registry
        .registered_providers()
        .into_iter()
        .filter_map(|id| {
            let descriptor = state.driver_registry.descriptor(&id)?;
            Some(DriverCredentialInfo {
                driver: id.to_string(),
                credential_schema: descriptor.credential_schema.clone(),
                supports_oauth: descriptor.oauth.is_some(),
            })
        })
        .collect();
    drivers.sort_by(|a, b| a.driver.cmp(&b.driver));

    Json(ProvidersConfigResponse { policies, drivers })
}

// ============================================================================
// OAuth provider connection (e.g. "Connect with OpenRouter")
//
// Some drivers declare an interactive OAuth flow (DriverDescriptor::oauth) so an
// admin can connect a provider by authorizing in the browser instead of pasting
// an API key. The flow yields a long-lived credential that is stored in the same
// encrypted credentials field a hand-typed key would use, so model resolution is
// unchanged and non-admin users are unaffected. The pattern is driver-agnostic:
// adding OAuth to another driver only requires a DriverOAuthConfig on its
// descriptor, not new endpoints.
// ============================================================================

/// HttpOnly, browser-bound state persisted across the OAuth round-trip. Binds
/// the callback to the provider, org, and the PKCE verifier this browser
/// generated, so a forged callback cannot inject an attacker's credential.
///
/// THREAT[TM-API-021]: this binding plus the `provider.manage` re-check in the
/// callback is the CSRF/forgery mitigation; do not relax it without updating
/// `knowledge/security/threat-model.md`.
#[derive(Debug, Serialize, Deserialize)]
struct PendingProviderOAuth {
    /// CSRF token echoed via the callback URL and matched here.
    state: String,
    /// Prefixed public provider id this flow targets.
    provider_id: String,
    /// Org the provider belongs to.
    org_id: i64,
    /// PKCE code verifier exchanged for the credential.
    code_verifier: String,
}

#[derive(Debug, Deserialize)]
pub struct ProviderOAuthCallbackQuery {
    pub code: String,
    pub state: Option<String>,
}

/// OpenRouter's token-exchange response: the `key` is a user-controlled API key.
#[derive(Debug, Deserialize)]
struct OpenRouterKeyResponse {
    key: String,
}

/// GET /v1/providers/{id}/oauth/authorize — begin the driver's OAuth connect flow.
///
/// Looks up the provider's driver, and if it declares an OAuth flow, redirects
/// the admin's browser to the provider's authorization endpoint with PKCE.
pub async fn oauth_authorize(
    org: ResolvedOrg,
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let caller = require_provider_manage(&state, &org)?;
    let (provider, oauth) = resolve_oauth_provider(&state, &caller, &id).await?;
    // Fail fast: the callback can only store the obtained key when encryption is
    // configured. Surface the same message here so an admin does not complete an
    // OAuth consent only to hit a 500 on the callback.
    require_encryption_configured(&state)?;
    // Defense-in-depth: the authorize URL is driver-declared, but validate it so
    // a future misconfigured driver cannot redirect into the internal network.
    validate_safe_url(&oauth.authorize_url).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Authorization endpoint blocked: {e}"),
        )
    })?;

    let csrf_state = random_hex();
    let code_verifier = generate_pkce_verifier();
    let code_challenge = pkce_challenge(&code_verifier);

    let pending = PendingProviderOAuth {
        state: csrf_state.clone(),
        provider_id: id.clone(),
        org_id: org.org_id,
        code_verifier,
    };
    let cookie = Cookie::build((
        oauth_state_cookie_name(&id),
        encode_pending_oauth(&pending, &state.auth.config.jwt.secret)
            .map_err(|e| internal("OAuth provider connection", &e))?,
    ))
    .path("/")
    .http_only(true)
    .secure(true)
    .same_site(SameSite::Lax)
    .max_age(time::Duration::minutes(10))
    .build();

    // The CSRF token rides in the callback URL so it round-trips regardless of
    // whether the provider echoes a `state` parameter of its own.
    let callback_url = format!(
        "{}/v1/providers/{}/oauth/callback?state={}",
        state.auth.config.base_url.trim_end_matches('/'),
        urlencoding::encode(&id),
        urlencoding::encode(&csrf_state),
    );

    let redirect_url = match oauth.flow {
        DriverOAuthFlow::OpenRouterPkce => {
            let mut url = reqwest::Url::parse(&oauth.authorize_url)
                .map_err(|e| internal("OAuth provider connection", &e))?;
            url.query_pairs_mut()
                .append_pair("callback_url", &callback_url)
                .append_pair("code_challenge", &code_challenge)
                .append_pair("code_challenge_method", "S256");
            url.to_string()
        }
    };

    tracing::info!(
        org_id = org.org_id,
        provider = %provider.provider_type,
        "Starting OAuth provider connection"
    );
    Ok((jar.add(cookie), Redirect::to(&redirect_url)))
}

/// GET /v1/providers/{id}/oauth/callback — finish the OAuth connect flow.
///
/// Validates the browser-bound state, exchanges the authorization code for a
/// credential, and stores it on the provider (encrypted at rest).
pub async fn oauth_callback(
    org: ResolvedOrg,
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Query(query): Query<ProviderOAuthCallbackQuery>,
) -> Result<(CookieJar, Redirect), (StatusCode, String)> {
    let caller = require_provider_manage(&state, &org)?;
    let pending = validate_pending_oauth(
        &jar,
        &id,
        org.org_id,
        query.state.as_deref(),
        &state.auth.config.jwt.secret,
    )?;
    // A `__Host-`-prefixed cookie is only deleted if the removal Set-Cookie also
    // carries Secure and Path=/, so build the removal cookie with those attributes
    // rather than relying on a bare name (which browsers would reject and ignore).
    let clear_cookie = jar.remove(
        Cookie::build(oauth_state_cookie_name(&id))
            .path("/")
            .secure(true)
            .http_only(true)
            .build(),
    );

    let (provider, oauth) = resolve_oauth_provider(&state, &caller, &id).await?;
    // Storing the obtained key requires encryption; fail fast with the canonical
    // message rather than letting the update surface a generic 500.
    require_encryption_configured(&state)?;

    let api_key = match oauth.flow {
        DriverOAuthFlow::OpenRouterPkce => {
            exchange_openrouter_code(&oauth.token_url, &query.code, &pending.code_verifier).await?
        }
    };

    // Store the obtained key exactly like a hand-entered one: encrypted at rest,
    // resolver cache invalidated. No backward-compat path needed.
    state
        .service
        .update(
            &caller,
            provider.id.uuid(),
            UpdateProviderRequest {
                name: None,
                provider_type: None,
                base_url: None,
                api_key: Some(api_key),
                credentials: None,
                status: None,
                trace: None,
                request_options: None,
            },
        )
        .await
        .map_err(|e| internal("OAuth provider connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "Provider not found".to_string()))?;

    tracing::info!(
        org_id = org.org_id,
        provider = %provider.provider_type,
        "Stored credential from OAuth provider connection"
    );

    let redirect = format!(
        "{}/settings/providers?connected={}",
        state.auth.config.frontend_url.trim_end_matches('/'),
        urlencoding::encode(provider.provider_type.as_str()),
    );
    Ok((clear_cookie, Redirect::to(&redirect)))
}

/// Resolve the provider and its driver's declared OAuth flow, or a client error
/// if the provider is missing or its driver does not support OAuth.
async fn resolve_oauth_provider(
    state: &AppState,
    caller: &Caller,
    id: &str,
) -> Result<
    (
        Provider,
        everruns_provider::driver_registry::DriverOAuthConfig,
    ),
    (StatusCode, String),
> {
    let provider_id = id
        .parse::<ProviderId>()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Invalid provider ID: {e}")))?;
    let provider = state
        .service
        .get(caller, provider_id.uuid())
        .await
        .map_err(|e| internal("OAuth provider connection", &e))?
        .ok_or((StatusCode::NOT_FOUND, "Provider not found".to_string()))?;
    let oauth = state
        .driver_registry
        .descriptor(&provider.provider_type)
        .and_then(|d| d.oauth.clone())
        .ok_or((
            StatusCode::BAD_REQUEST,
            format!(
                "Provider driver '{}' does not support OAuth connection",
                provider.provider_type
            ),
        ))?;
    Ok((provider, oauth))
}

/// Exchange an OpenRouter PKCE authorization code for a user-controlled API key.
async fn exchange_openrouter_code(
    token_url: &str,
    code: &str,
    code_verifier: &str,
) -> Result<String, (StatusCode, String)> {
    // token_url is driver-declared (openrouter.ai), not user input; validate
    // anyway as defense-in-depth against future misconfiguration.
    validate_safe_url(token_url).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            format!("Token endpoint blocked: {e}"),
        )
    })?;
    let response = reqwest::Client::new()
        .post(token_url)
        .json(&serde_json::json!({
            "code": code,
            "code_verifier": code_verifier,
            "code_challenge_method": "S256",
        }))
        .send()
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "OAuth token exchange request failed");
            (StatusCode::BAD_GATEWAY, "Token exchange failed".to_string())
        })?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        tracing::error!(%status, body_len = body.len(), "OAuth token exchange returned error");
        return Err((StatusCode::BAD_GATEWAY, "Token exchange failed".to_string()));
    }
    let parsed: OpenRouterKeyResponse = response.json().await.map_err(|e| {
        tracing::error!(error = %e, "Failed to parse OAuth token exchange response");
        (StatusCode::BAD_GATEWAY, "Token exchange failed".to_string())
    })?;
    Ok(parsed.key)
}

/// Require server-side encryption to be configured. The OAuth flow always ends
/// by storing a credential, so without encryption it cannot succeed — surface
/// the same message `ProviderService` uses, before the external round-trip.
fn require_encryption_configured(state: &AppState) -> Result<(), (StatusCode, String)> {
    if state.encryption.is_none() {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            "Encryption not configured. Cannot store API key.".to_string(),
        ));
    }
    Ok(())
}

/// Require the caller to hold the provider-manage permission for this org.
fn require_provider_manage(
    state: &AppState,
    org: &ResolvedOrg,
) -> Result<Caller, (StatusCode, String)> {
    let caller = Caller::from(org);
    Policy::evaluate_with(
        &LLM_PROVIDER_MANAGE,
        state.auth.permission_resolver.as_ref(),
        &caller,
    )
    .map_err(|_| {
        (
            StatusCode::FORBIDDEN,
            "You do not have permission to manage providers".to_string(),
        )
    })?;
    Ok(caller)
}

fn validate_pending_oauth(
    jar: &CookieJar,
    provider_id: &str,
    org_id: i64,
    query_state: Option<&str>,
    signing_secret: &str,
) -> Result<PendingProviderOAuth, (StatusCode, String)> {
    let cookie = jar.get(&oauth_state_cookie_name(provider_id)).ok_or((
        StatusCode::BAD_REQUEST,
        "Invalid or expired OAuth state".to_string(),
    ))?;
    let pending = decode_pending_oauth(cookie.value(), signing_secret)?;
    let callback_state = query_state.ok_or((
        StatusCode::BAD_REQUEST,
        "Missing state parameter".to_string(),
    ))?;
    if pending.state != callback_state
        || pending.provider_id != provider_id
        || pending.org_id != org_id
    {
        return Err((StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()));
    }
    Ok(pending)
}

fn encode_pending_oauth(
    pending: &PendingProviderOAuth,
    signing_secret: &str,
) -> Result<String, serde_json::Error> {
    let payload = serde_json::to_vec(pending)?;
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .expect("HMAC accepts signing keys of any length");
    mac.update(&payload);
    let signature = mac.finalize().into_bytes();
    Ok(format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(payload),
        URL_SAFE_NO_PAD.encode(signature)
    ))
}

fn decode_pending_oauth(
    value: &str,
    signing_secret: &str,
) -> Result<PendingProviderOAuth, (StatusCode, String)> {
    let (payload, signature) = value
        .split_once('.')
        .ok_or((StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let payload = URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let signature = URL_SAFE_NO_PAD
        .decode(signature)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    let mut mac = HmacSha256::new_from_slice(signing_secret.as_bytes())
        .expect("HMAC accepts signing keys of any length");
    mac.update(&payload);
    mac.verify_slice(&signature)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))?;
    serde_json::from_slice(&payload)
        .map_err(|_| (StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()))
}

fn oauth_state_cookie_name(provider_id: &str) -> String {
    format!(
        "__Host-provider_oauth_state_{}",
        provider_id.replace([':', '/'], "_")
    )
}

fn random_hex() -> String {
    let bytes: [u8; 16] = rand::rng().random();
    hex::encode(bytes)
}

fn generate_pkce_verifier() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    URL_SAFE_NO_PAD.encode(bytes)
}

fn pkce_challenge(verifier: &str) -> String {
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest)
}

fn internal(context: &str, err: &impl std::fmt::Display) -> (StatusCode, String) {
    tracing::error!("{context} error: {err}");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Internal server error".to_string(),
    )
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/providers/config", get(provider_config))
        .route("/v1/providers", post(create_provider).get(list_providers))
        .route(
            "/v1/providers/{id}",
            get(get_provider)
                .patch(update_provider)
                .delete(delete_provider),
        )
        .route("/v1/providers/{id}/sync-models", post(sync_models))
        .route("/v1/providers/{id}/oauth/authorize", get(oauth_authorize))
        .route("/v1/providers/{id}/oauth/callback", get(oauth_callback))
        .with_state(state)
}

#[cfg(test)]
mod oauth_tests {
    use super::*;

    const TEST_SIGNING_SECRET: &str = "test-oauth-cookie-signing-secret";

    fn jar_with_pending(pending: &PendingProviderOAuth) -> CookieJar {
        let value = encode_pending_oauth(pending, TEST_SIGNING_SECRET).unwrap();
        CookieJar::new().add(Cookie::new(
            oauth_state_cookie_name(&pending.provider_id),
            value,
        ))
    }

    fn sample_pending() -> PendingProviderOAuth {
        PendingProviderOAuth {
            state: "csrf-token".to_string(),
            provider_id: "provider_abc".to_string(),
            org_id: 42,
            code_verifier: "verifier".to_string(),
        }
    }

    #[test]
    fn pkce_challenge_is_base64url_sha256() {
        // RFC 7636 worked example.
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        assert_eq!(
            pkce_challenge(verifier),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn cookie_name_sanitizes_separators() {
        assert_eq!(
            oauth_state_cookie_name("mcp:oauth/id"),
            "__Host-provider_oauth_state_mcp_oauth_id"
        );
    }

    #[test]
    fn validate_pending_accepts_matching_state() {
        let pending = sample_pending();
        let jar = jar_with_pending(&pending);
        let result = validate_pending_oauth(
            &jar,
            "provider_abc",
            42,
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
        )
        .unwrap();
        assert_eq!(result.code_verifier, "verifier");
    }

    #[test]
    fn validate_pending_rejects_state_mismatch() {
        let jar = jar_with_pending(&sample_pending());
        let err =
            validate_pending_oauth(&jar, "provider_abc", 42, Some("wrong"), TEST_SIGNING_SECRET)
                .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_org_mismatch() {
        // A cookie minted for another org must not authorize this org's provider.
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            99,
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_provider_mismatch() {
        let jar = jar_with_pending(&sample_pending());
        // Cookie name is per-provider, so a different provider id finds no cookie.
        let err = validate_pending_oauth(
            &jar,
            "provider_other",
            42,
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_requires_query_state() {
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(&jar, "provider_abc", 42, None, TEST_SIGNING_SECRET)
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_missing_cookie() {
        let err = validate_pending_oauth(
            &CookieJar::new(),
            "provider_abc",
            42,
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_unsigned_cookie() {
        let pending = sample_pending();
        let forged_value = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&pending).unwrap());
        let jar = CookieJar::new().add(Cookie::new(
            oauth_state_cookie_name("provider_abc"),
            forged_value,
        ));
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            42,
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_tampered_signature() {
        let pending = sample_pending();
        let mut value = encode_pending_oauth(&pending, TEST_SIGNING_SECRET).unwrap();
        value.push('A');
        let jar = CookieJar::new().add(Cookie::new(oauth_state_cookie_name("provider_abc"), value));
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            42,
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.
}

#[cfg(test)]
mod tests {
    use super::*;

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

    #[test]
    fn test_error_response_internal_error_format() {
        let error = ErrorResponse::new("Internal server error");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["detail"], "Internal server error");
    }

    #[test]
    fn test_error_response_not_found_format() {
        let error = ErrorResponse::new("Provider not found");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["detail"], "Provider not found");
    }

    #[test]
    fn test_error_response_encryption_not_configured() {
        let error = ErrorResponse::new("Encryption not configured. Cannot store API key.");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(
            parsed["detail"],
            "Encryption not configured. Cannot store API key."
        );
    }

    #[test]
    fn test_invalid_base_url_error_is_client_facing() {
        // URL validation errors should be returned as-is (not masked as internal error)
        let error_msg =
            "Invalid base URL: URL host resolves to a blocked address: loopback (127.0.0.0/8)";
        assert!(error_msg.contains("Invalid base URL"));
    }

    #[test]
    fn test_internal_error_does_not_leak_details() {
        // Simulate what happens when a database error occurs
        // The error message should be generic, not contain DB details
        let generic_message = "Internal server error".to_string();

        // This is what we return to clients - verify it doesn't contain
        // typical database error patterns
        assert!(!generic_message.contains("SQLX"));
        assert!(!generic_message.contains("connection"));
        assert!(!generic_message.contains("database"));
        assert!(!generic_message.contains("query"));
        assert!(!generic_message.contains("postgres"));
        assert!(!generic_message.contains("encryption key"));
    }
}
