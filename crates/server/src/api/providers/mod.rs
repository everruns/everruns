// LLM Provider API endpoints
// Routes: /v1/providers/...

use crate::auth::{AuthState, ResolvedOrg};
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
    Caller, contracts::driver_registry::DriverRegistry, contracts::provider::DriverId,
    evaluate_policies_with,
};
use crate::services::ProviderResolverService;
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_contracts::typed_id::ProviderId;
use serde::{Deserialize, Serialize};
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

mod oauth;
use oauth::{oauth_authorize, oauth_callback};

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
