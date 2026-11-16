// LLM Provider API endpoints
// Routes: /v1/providers/...

use crate::auth::config::AuthMode;
use crate::auth::{AuthState, AuthUser, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::models::ModelSyncService;
use crate::domains::providers::record::Provider;
pub use crate::domains::providers::types::{
    CreateProviderRequest, SyncModelsResponse, UpdateProviderRequest,
};
use crate::domains::providers::{
    CheckProviderCredentials, CreateProvider, CredentialCheckResult, DeleteProvider, GetProvider,
    LLM_PROVIDER_MANAGE, LLM_PROVIDER_VIEW, ListProviders, ProviderService, ReviewProviderModels,
    SyncProviderModels, UpdateProvider,
};
use crate::kernel_imports::{
    Caller, Policy, contracts::driver_registry::DriverOAuthFlow,
    contracts::driver_registry::DriverRegistry, contracts::provider::DriverId,
    evaluate_policies_with,
};
use crate::services::ProviderResolverService;
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
use everruns_contracts::typed_id::ProviderId;
use everruns_contracts::url_validation::validate_safe_url;
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
    /// Model service, so a provider that gains a credential can bootstrap the
    /// org's enabled models and default model in the same request. Carries the
    /// provider resolver when one exists, so the cache is invalidated after the
    /// bootstrap writes.
    pub model_service: Arc<crate::domains::models::ModelService>,
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
        let services = crate::domains::providers::ProviderServices::new(
            db.clone(),
            encryption.clone(),
            driver_registry.clone(),
            provider_resolver,
        );
        Self {
            db,
            service: services.provider,
            model_service: services.model,
            sync_service: services.model_sync,
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
        .with_provider_services(&self.provider_services())
    }

    /// The service instances these routes use, for every other command surface
    /// (MCP, gRPC worker, in-process worker) to share.
    pub fn provider_services(&self) -> crate::domains::providers::ProviderServices {
        crate::domains::providers::ProviderServices {
            provider: self.service.clone(),
            model_sync: self.sync_service.clone(),
            model: self.model_service.clone(),
        }
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

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
            api: req.api,
        })
        .await
}

/// Request to check a provider credential without storing it.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckCredentialsRequest {
    /// The type of LLM provider (e.g., openai, anthropic).
    pub provider_type: DriverId,
    /// Single-field credential. Mutually exclusive with `credentials`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "sk-proj-...")]
    pub api_key: Option<String>,
    /// Typed multi-field credential, validated against the driver's schema
    /// exactly as on create.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credentials: Option<std::collections::BTreeMap<String, String>>,
    /// Base URL for the provider's API, when not the driver default.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "https://api.openai.com/v1")]
    pub base_url: Option<String>,
}

/// Check a provider API key without storing it
///
/// Check whether a provider accepts an API key, without storing it
///
/// Used by org setup so a key the provider will reject is caught at entry
/// instead of at the first agent run. Nothing is persisted.
#[utoipa::path(
    post,
    path = "/v1/providers/check-credentials",
    request_body = CheckCredentialsRequest,
    responses(
        (status = 200, description = "Check completed", body = CredentialCheckResult),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal error")
    ),
    tag = "providers"
)]
pub async fn check_credentials(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CheckCredentialsRequest>,
) -> Result<Json<CredentialCheckResult>, (StatusCode, Json<ErrorResponse>)> {
    let api_key = resolve_credential_document(
        &state.driver_registry,
        Some(&req.provider_type),
        req.credentials.as_ref(),
        req.api_key,
    )?
    .ok_or_else(|| {
        ErrorResponse::new("An API key is required").into_response(StatusCode::BAD_REQUEST)
    })?;

    let result = CheckProviderCredentials {
        provider_type: req.provider_type,
        api_key,
        base_url: req.base_url,
    }
    .run(&state.ctx(&org))
    .await
    // Uses the HTTP adapter conversion so an internal error is redacted rather
    // than echoed back with provider detail.
    .map_err(<(StatusCode, Json<ErrorResponse>)>::from)?;

    Ok(Json(result))
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

    Ok(everruns_contracts::credential_schema::assemble_credential_document(fields))
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
            api: req.api,
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
        (status = 422, description = "Provider rejected or could not serve the model list (code `provider_sync_failed`)")
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

/// Mark a provider's discovered models as reviewed
///
/// Clears the `is_new` flag on the provider's discovered models: only models
/// discovered after this call, and still disabled, read back as new.
#[utoipa::path(
    post,
    path = "/v1/providers/{id}/models/review",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "Provider with its review time updated", body = Provider),
        (status = 400, description = "Invalid provider ID"),
        (status = 403, description = "Caller cannot manage providers"),
        (status = 404, description = "Provider not found")
    ),
    tag = "providers"
)]
pub async fn review_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Provider> {
    Ok(Json(
        ReviewProviderModels { id }.run(&state.ctx(&org)).await?,
    ))
}

/// A driver's declared credential schema, so the Settings UI can render
/// discrete typed inputs (multi-field AWS keys, Entra OAuth fields) instead of
/// one opaque password field.
#[derive(Debug, Serialize, ToSchema)]
pub struct DriverCredentialInfo {
    /// Driver id (e.g. `openai`, `bedrock`, `mai`).
    pub driver: String,
    /// The fields and instructions to render for this driver's credential.
    pub credential_schema: everruns_contracts::credential_schema::CredentialFormSchema,
    /// Whether the driver declares an interactive "Connect with …" OAuth flow.
    pub supports_oauth: bool,
    pub services: Vec<everruns_contracts::ServiceKind>,
    /// Supported explicit chat APIs; "default" is also accepted.
    pub apis: Vec<String>,
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
            // Legacy Codex stays an SDK driver for Yolop. Server accounts use
            // the personal ChatGPT connection and its runtime ownership checks.
            if id.as_str() == "openai-codex" {
                return None;
            }
            if !org.feature_flags.is_driver_offered(id.as_str()) {
                return None;
            }
            let descriptor = state.driver_registry.descriptor(&id)?;
            Some(DriverCredentialInfo {
                driver: id.to_string(),
                credential_schema: descriptor.credential_schema.clone(),
                apis: descriptor.apis.clone(),
                supports_oauth: descriptor.oauth.is_some(),
                services: descriptor.services.clone(),
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
/// the callback to the provider, org, admin, and the PKCE verifier this browser
/// generated, so a forged callback cannot inject an attacker's credential.
///
/// Decision: the callback is authenticated by this state, not by the session
/// cookie. The provider's consent page can outlive the 15-minute access-token
/// cookie (signing in or signing up at OpenRouter takes a while), and only XHRs
/// refresh that cookie, so a top-level redirect back routinely arrived with no
/// session and failed with a bare 401. The state is HMAC-signed with the JWT
/// secret, expires server-side, and names the admin who started the flow; the
/// callback reloads that admin's membership and re-checks `provider.manage`.
///
// THREAT[TM-API-021]: this binding plus the `provider.manage` re-check in the
// callback is the CSRF/forgery mitigation; do not relax it without updating
// `knowledge/security/threat-model.md`.
#[derive(Debug, Serialize, Deserialize)]
struct PendingProviderOAuth {
    /// CSRF token echoed via the callback URL and matched here.
    state: String,
    /// Prefixed public provider id this flow targets.
    provider_id: String,
    /// Org the provider belongs to.
    org_id: i64,
    /// Admin who started the flow; the callback acts as them. `None` only for
    /// the anonymous no-auth (local development) user.
    user_id: Option<uuid::Uuid>,
    /// Unix seconds after which the state is refused, whatever the browser kept.
    expires_at: i64,
    /// PKCE code verifier exchanged for the credential.
    code_verifier: String,
}

/// How long a started connect flow stays valid (state cookie and payload).
const PROVIDER_OAUTH_STATE_TTL_MINUTES: i64 = 10;

#[derive(Debug, Deserialize)]
pub struct ProviderOAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    /// Set by the provider when the user declines consent.
    pub error: Option<String>,
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
        user_id: (state.auth.config.mode != AuthMode::None)
            .then_some(caller.user_id)
            .flatten(),
        expires_at: chrono::Utc::now().timestamp() + PROVIDER_OAUTH_STATE_TTL_MINUTES * 60,
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
    .max_age(time::Duration::minutes(PROVIDER_OAUTH_STATE_TTL_MINUTES))
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
        DriverOAuthFlow::ChatGptPlan => {
            return Err((
                StatusCode::BAD_REQUEST,
                "Use the ChatGPT connection endpoint.".to_string(),
            ));
        }
        DriverOAuthFlow::OpenRouterPkce => openrouter_authorize_url(
            &oauth.authorize_url,
            &callback_url,
            &code_challenge,
            &org.name,
        )
        .map_err(|e| internal("OAuth provider connection", &e))?,
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
/// Reached by the provider's browser redirect, so it carries no session of its
/// own: the signed state cookie set by `oauth_authorize` authenticates it (see
/// [`PendingProviderOAuth`]). Validates that state, exchanges the authorization
/// code for a credential, stores it on the provider (encrypted at rest), and
/// always returns the browser to the providers page: `?connected=<driver>` on
/// success, `?connect_error=<code>` otherwise.
pub async fn oauth_callback(
    State(state): State<AppState>,
    jar: CookieJar,
    Path(id): Path<String>,
    Query(query): Query<ProviderOAuthCallbackQuery>,
) -> (CookieJar, Redirect) {
    // A `__Host-`-prefixed cookie is only deleted if the removal Set-Cookie also
    // carries Secure and Path=/, so build the removal cookie with those attributes
    // rather than relying on a bare name (which browsers would reject and ignore).
    let clear_cookie = jar.clone().remove(
        Cookie::build(oauth_state_cookie_name(&id))
            .path("/")
            .secure(true)
            .http_only(true)
            .build(),
    );
    let providers_page = format!(
        "{}/models?tab=providers",
        state.auth.config.frontend_url.trim_end_matches('/'),
    );
    match finish_oauth_callback(&state, &jar, &id, query).await {
        Ok(provider_type) => (
            clear_cookie,
            Redirect::to(&format!(
                "{providers_page}&connected={}",
                urlencoding::encode(provider_type.as_str()),
            )),
        ),
        Err(failure) => {
            tracing::warn!(
                provider_id = %id,
                reason = failure.reason,
                "OAuth provider connection failed"
            );
            (
                clear_cookie,
                Redirect::to(&format!("{providers_page}&connect_error={}", failure.code)),
            )
        }
    }
}

/// Why a provider OAuth callback failed: a stable `connect_error` code the UI
/// turns into a sentence, and a log-only reason.
#[derive(Debug)]
struct OAuthCallbackFailure {
    code: &'static str,
    reason: &'static str,
}

impl OAuthCallbackFailure {
    fn new(code: &'static str, reason: &'static str) -> Self {
        Self { code, reason }
    }
}

async fn finish_oauth_callback(
    state: &AppState,
    jar: &CookieJar,
    id: &str,
    query: ProviderOAuthCallbackQuery,
) -> Result<DriverId, OAuthCallbackFailure> {
    let pending = validate_pending_oauth(
        jar,
        id,
        query.state.as_deref(),
        &state.auth.config.jwt.secret,
        chrono::Utc::now().timestamp(),
    )
    .map_err(|_| OAuthCallbackFailure::new("expired", "invalid or expired OAuth state"))?;
    if query.error.is_some() {
        return Err(OAuthCallbackFailure::new(
            "provider_refused",
            "provider returned an error",
        ));
    }
    let code = query
        .code
        .filter(|code| !code.is_empty())
        .ok_or_else(|| OAuthCallbackFailure::new("provider_refused", "no authorization code"))?;

    let caller = oauth_callback_caller(state, &pending).await?;
    Policy::evaluate_with(
        &LLM_PROVIDER_MANAGE,
        state.auth.permission_resolver.as_ref(),
        &caller,
    )
    .map_err(|_| OAuthCallbackFailure::new("failed", "caller can no longer manage providers"))?;

    let (provider, oauth) = resolve_oauth_provider(state, &caller, id)
        .await
        .map_err(|_| OAuthCallbackFailure::new("failed", "provider or OAuth flow not found"))?;
    // Storing the obtained key requires encryption.
    require_encryption_configured(state)
        .map_err(|_| OAuthCallbackFailure::new("failed", "encryption not configured"))?;

    let api_key = match oauth.flow {
        DriverOAuthFlow::ChatGptPlan => {
            return Err(OAuthCallbackFailure::new(
                "failed",
                "ChatGPT uses its own connection endpoint",
            ));
        }
        DriverOAuthFlow::OpenRouterPkce => {
            exchange_openrouter_code(&oauth.token_url, &code, &pending.code_verifier)
                .await
                .map_err(|_| {
                    OAuthCallbackFailure::new("provider_refused", "token exchange failed")
                })?
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
                api: None,
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
        .map_err(|e| {
            tracing::error!(error = %e, "Failed to store OAuth provider credential");
            OAuthCallbackFailure::new("failed", "could not store credential")
        })?
        .ok_or_else(|| OAuthCallbackFailure::new("failed", "provider not found"))?;

    tracing::info!(
        org_id = pending.org_id,
        provider = %provider.provider_type,
        "Stored credential from OAuth provider connection"
    );
    Ok(provider.provider_type)
}

/// Rebuild the caller who started the flow from the signed state: the admin's
/// current membership and role (so a removal or demotion since the start is
/// honored), or the anonymous owner in no-auth local development.
async fn oauth_callback_caller(
    state: &AppState,
    pending: &PendingProviderOAuth,
) -> Result<Caller, OAuthCallbackFailure> {
    if state.auth.config.mode == AuthMode::None {
        let org = state
            .db
            .get_organization(pending.org_id)
            .await
            .ok()
            .flatten()
            .ok_or_else(|| OAuthCallbackFailure::new("failed", "organization not found"))?;
        let anonymous = AuthUser::anonymous();
        return Ok(Caller {
            org_id: org.org_id,
            org_public_id: org.public_id,
            user_id: Some(anonymous.id),
            role: everruns_core::OrgRole::Owner,
            is_platform_user: anonymous.is_platform_user,
            is_internal: false,
            read_only: false,
        });
    }
    let user_id = pending
        .user_id
        .ok_or_else(|| OAuthCallbackFailure::new("expired", "state names no user"))?;
    crate::auth::caller_resolution::caller_for_user(&state.db, pending.org_id, user_id)
        .await
        .map_err(|_| OAuthCallbackFailure::new("failed", "user is no longer an org member"))
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
        everruns_contracts::driver_registry::DriverOAuthConfig,
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

/// OpenRouter authorize URL for the PKCE connect flow.
///
/// `key_label` prefills the consent screen and becomes the created key's name.
fn openrouter_authorize_url(
    authorize_url: &str,
    callback_url: &str,
    code_challenge: &str,
    org_name: &str,
) -> Result<String, url::ParseError> {
    let mut url = reqwest::Url::parse(authorize_url)?;
    url.query_pairs_mut()
        .append_pair("callback_url", callback_url)
        .append_pair("code_challenge", code_challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("key_label", &openrouter_key_label(org_name));
    Ok(url.to_string())
}

/// OpenRouter's authorize page prefills the created key's label from `key_label`.
/// The API caps that field at 100 characters.
const OPENROUTER_KEY_LABEL_LIMIT: usize = 100;

/// Label shown on OpenRouter's consent screen and stored on the created key:
/// `Everruns <org name>`, so the key is identifiable in the OpenRouter dashboard.
fn openrouter_key_label(org_name: &str) -> String {
    let name = org_name.trim();
    let label = if name.is_empty() {
        "Everruns".to_string()
    } else {
        format!("Everruns {name}")
    };
    let mut truncated = String::new();
    for ch in label.chars().take(OPENROUTER_KEY_LABEL_LIMIT) {
        truncated.push(ch);
    }
    truncated
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
    query_state: Option<&str>,
    signing_secret: &str,
    now: i64,
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
    if pending.state != callback_state || pending.provider_id != provider_id {
        return Err((StatusCode::BAD_REQUEST, "Invalid OAuth state".to_string()));
    }
    if now > pending.expires_at {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid or expired OAuth state".to_string(),
        ));
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
        .route(
            "/v1/providers/{id}/chatgpt",
            get(super::chatgpt::status).delete(super::chatgpt::disconnect),
        )
        .route(
            "/v1/providers/{id}/chatgpt/login",
            post(super::chatgpt::login),
        )
        .route(
            "/v1/providers/{id}/chatgpt/import",
            post(super::chatgpt::import),
        )
        .route("/v1/providers", post(create_provider).get(list_providers))
        .route(
            "/v1/providers/{id}",
            get(get_provider)
                .patch(update_provider)
                .delete(delete_provider),
        )
        .route("/v1/providers/check-credentials", post(check_credentials))
        .route("/v1/providers/{id}/sync-models", post(sync_models))
        .route("/v1/providers/{id}/models/review", post(review_models))
        .route("/v1/providers/{id}/oauth/authorize", get(oauth_authorize))
        .route("/v1/providers/{id}/oauth/callback", get(oauth_callback))
        .with_state(state)
}

#[cfg(test)]
mod oauth_tests {
    use super::*;

    const TEST_SIGNING_SECRET: &str = "test-oauth-cookie-signing-secret";
    const NOW: i64 = 1_800_000_000;

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
            user_id: Some(uuid::Uuid::nil()),
            expires_at: NOW + 600,
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
    fn openrouter_authorize_url_prefills_the_org_key_label() {
        let url = openrouter_authorize_url(
            "https://openrouter.ai/auth",
            "https://app.everruns.com/v1/providers/provider_abc/oauth/callback?state=csrf",
            "challenge",
            "Acme & Co",
        )
        .unwrap();
        let parsed = reqwest::Url::parse(&url).unwrap();
        let query: std::collections::HashMap<String, String> =
            parsed.query_pairs().into_owned().collect();
        assert_eq!(
            query.get("key_label").map(String::as_str),
            Some("Everruns Acme & Co")
        );
        assert_eq!(
            query.get("code_challenge_method").map(String::as_str),
            Some("S256")
        );
        assert_eq!(parsed.host_str(), Some("openrouter.ai"));
    }

    #[test]
    fn openrouter_key_label_uses_the_org_name() {
        assert_eq!(openrouter_key_label("Acme"), "Everruns Acme");
        assert_eq!(openrouter_key_label("  Acme  "), "Everruns Acme");
        assert_eq!(openrouter_key_label("   "), "Everruns");
    }

    #[test]
    fn openrouter_key_label_stays_within_openrouter_limit() {
        let org = "界".repeat(200);
        let label = openrouter_key_label(&org);
        assert_eq!(label.chars().count(), OPENROUTER_KEY_LABEL_LIMIT);
        assert!(label.starts_with("Everruns 界"));
        assert!(label.is_char_boundary(label.len()));
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
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap();
        assert_eq!(result.code_verifier, "verifier");
    }

    #[test]
    fn validate_pending_rejects_state_mismatch() {
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            Some("wrong"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_expired_state() {
        // The browser may keep the cookie past its max-age; the signed payload's
        // own expiry still refuses it.
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(
            &jar,
            "provider_abc",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW + 601,
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
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_requires_query_state() {
        let jar = jar_with_pending(&sample_pending());
        let err = validate_pending_oauth(&jar, "provider_abc", None, TEST_SIGNING_SECRET, NOW)
            .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    #[test]
    fn validate_pending_rejects_missing_cookie() {
        let err = validate_pending_oauth(
            &CookieJar::new(),
            "provider_abc",
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
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
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
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
            Some("csrf-token"),
            TEST_SIGNING_SECRET,
            NOW,
        )
        .unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
    }

    fn callback_state(mode: AuthMode) -> AppState {
        let db = Arc::new(StorageBackend::test_database());
        let config = crate::auth::AuthConfig {
            mode,
            jwt: crate::auth::config::JwtConfig {
                secret: TEST_SIGNING_SECRET.to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        AppState::new(
            db.clone(),
            None,
            Arc::new(DriverRegistry::new()),
            AuthState::builtin(config, db),
            None,
        )
    }

    fn live_pending() -> PendingProviderOAuth {
        PendingProviderOAuth {
            expires_at: chrono::Utc::now().timestamp() + 600,
            ..sample_pending()
        }
    }

    async fn callback_location(
        state: AppState,
        jar: CookieJar,
        query: ProviderOAuthCallbackQuery,
    ) -> String {
        use axum::response::IntoResponse;
        let response = oauth_callback(
            State(state),
            jar,
            Path("provider_abc".to_string()),
            Query(query),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        response.headers()[axum::http::header::LOCATION]
            .to_str()
            .unwrap()
            .to_string()
    }

    // Regression: the provider's redirect back carries no session cookie once
    // the 15-minute access token lapses during consent. The callback must not
    // answer with a bare 401 JSON page; it returns the browser to the providers
    // page with a reason the UI can show.
    #[tokio::test]
    async fn callback_without_session_or_state_returns_to_the_ui() {
        let location = callback_location(
            callback_state(AuthMode::Admin),
            CookieJar::new(),
            ProviderOAuthCallbackQuery {
                code: Some("code".into()),
                state: Some("csrf-token".into()),
                error: None,
            },
        )
        .await;
        assert_eq!(
            location,
            "http://localhost:9300/models?tab=providers&connect_error=expired"
        );
    }

    #[tokio::test]
    async fn callback_reports_declined_consent() {
        let location = callback_location(
            callback_state(AuthMode::Admin),
            jar_with_pending(&live_pending()),
            ProviderOAuthCallbackQuery {
                code: None,
                state: Some("csrf-token".into()),
                error: Some("access_denied".into()),
            },
        )
        .await;
        assert!(
            location.ends_with("connect_error=provider_refused"),
            "{location}"
        );
    }

    // With a valid signed state and no session, the callback acts as the admin
    // the state names, re-resolved from the database: a user who is no longer a
    // member (here: unknown) cannot finish the flow.
    #[tokio::test]
    async fn callback_rechecks_the_admin_named_by_the_state() {
        let location = callback_location(
            callback_state(AuthMode::Admin),
            jar_with_pending(&live_pending()),
            ProviderOAuthCallbackQuery {
                code: Some("code".into()),
                state: Some("csrf-token".into()),
                error: None,
            },
        )
        .await;
        assert!(location.ends_with("connect_error=failed"), "{location}");
    }

    #[tokio::test]
    async fn callback_refuses_expired_state_even_if_the_browser_kept_it() {
        let expired = PendingProviderOAuth {
            expires_at: chrono::Utc::now().timestamp() - 1,
            ..sample_pending()
        };
        let location = callback_location(
            callback_state(AuthMode::Admin),
            jar_with_pending(&expired),
            ProviderOAuthCallbackQuery {
                code: Some("code".into()),
                state: Some("csrf-token".into()),
                error: None,
            },
        )
        .await;
        assert!(location.ends_with("connect_error=expired"), "{location}");
    }

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.
}

// Trivial ErrorResponse::new() + serialize round-trips removed; covered by
// crates/server/src/api/common.rs::tests::test_error_response_into_response.

#[cfg(test)]
mod creation_tests {
    use super::*;

    /// The provider command context must carry both the sync service *and* the
    /// model service: `provision_provider_models` bootstraps the org's enabled
    /// models and default model through the latter, and its best-effort
    /// `if let Some(..)` would otherwise skip that silently — discovery would
    /// run, every model would stay disabled, and chat would still resolve
    /// nothing. Regression guard for exactly that wiring gap.
    #[tokio::test]
    async fn provider_ctx_carries_the_services_provisioning_needs() {
        let db = Arc::new(StorageBackend::test_database());
        let state = AppState::new(
            db.clone(),
            None,
            Arc::new(DriverRegistry::new()),
            AuthState::builtin(crate::auth::AuthConfig::default(), db),
            None,
        );
        let org = ResolvedOrg {
            org_id: everruns_core::DEFAULT_ORG_ID,
            public_id: "org_test".into(),
            name: "Test".into(),
            user_id: None,
            role: everruns_core::OrgRole::Owner,
            is_platform_user: false,
            feature_flags: Default::default(),
            read_only: false,
        };

        let ctx = state.ctx(&org);

        assert!(
            ctx.model_sync_service.is_some(),
            "model sync service missing"
        );
        assert!(ctx.model_service.is_some(), "model service missing");
    }

    #[tokio::test]
    async fn create_rejects_invalid_base_urls_as_client_errors() {
        let db = Arc::new(StorageBackend::test_database());
        let state = AppState::new(
            db.clone(),
            None,
            Arc::new(DriverRegistry::new()),
            AuthState::builtin(crate::auth::AuthConfig::default(), db),
            None,
        );
        let org = ResolvedOrg {
            org_id: everruns_core::DEFAULT_ORG_ID,
            public_id: "org_test".into(),
            name: "Test".into(),
            user_id: None,
            role: everruns_core::OrgRole::Owner,
            is_platform_user: false,
            feature_flags: Default::default(),
            read_only: false,
        };
        for (provider_type, base_url) in [
            (DriverId::OpenAI, Some("https://127.0.0.1/v1")),
            (DriverId::OpenAI, Some("https://10.0.0.1/v1")),
            (
                DriverId::OpenAI,
                Some("https://169.254.169.254/latest/meta-data/"),
            ),
            (DriverId::AzureOpenAI, None),
            (
                DriverId::AzureOpenAI,
                Some("https://api.example.com/openai/v1"),
            ),
            (
                DriverId::AzureOpenAI,
                Some("https://resource.openai.azure.com/wrong-path"),
            ),
        ] {
            let (status, Json(error)) = create_provider(
                org.clone(),
                State(state.clone()),
                Json(CreateProviderRequest {
                    api: None,
                    name: "Unsafe provider".into(),
                    provider_type,
                    base_url: base_url.map(str::to_string),
                    api_key: None,
                    credentials: None,
                    trace: None,
                    request_options: None,
                }),
            )
            .await
            .unwrap_err();
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(
                error
                    .detail
                    .as_deref()
                    .is_some_and(|detail| detail.contains("Invalid base URL")),
                "{error:?}"
            );
        }
        assert!(
            state
                .service
                .list(&Caller::from(&org))
                .await
                .unwrap()
                .is_empty()
        );
    }
}
