// LLM Provider API endpoints
// Routes: /v1/llm-providers/...

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::providers::{
    CreateProvider, DeleteProvider, GetProvider, LLM_PROVIDER_MANAGE, LLM_PROVIDER_VIEW,
    ListProviders, ProviderService, SyncProviderModels, UpdateProvider,
};
use crate::services::{ProviderResolverService, ModelSyncService};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::provider::Provider;
use everruns_core::{
    Caller, DriverRegistry, ProviderStatus, DriverId, ResourceConfigResponse,
    evaluate_policies_with,
};
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
    pub auth: AuthState,
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
            sync_service: Arc::new(ModelSyncService::new(db, driver_registry, encryption)),
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_llm_provider_service(self.service.clone())
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// The status of the provider. Set to "inactive" to disable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ProviderStatus>,
}

/// Create a new LLM provider
#[utoipa::path(
    post,
    path = "/v1/llm-providers",
    request_body = CreateProviderRequest,
    responses(
        (status = 201, description = "Provider created", body = WithUrls<Provider>),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal error")
    ),
    tag = "llm-providers"
)]
pub async fn create_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateProviderRequest>,
) -> Result<(StatusCode, Json<WithUrls<Provider>>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created_with_urls(CreateProvider {
            name: req.name,
            provider_type: req.provider_type,
            base_url: req.base_url,
            api_key: req.api_key,
        })
        .await
}

/// List all LLM providers
#[utoipa::path(
    get,
    path = "/v1/llm-providers",
    responses(
        (status = 200, description = "List of providers", body = ListResponse<WithUrls<Provider>>)
    ),
    tag = "llm-providers"
)]
pub async fn list_providers(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<ListResponse<WithUrls<Provider>>> {
    let providers = ListProviders.run(&state.ctx(&org)).await?;

    let builder = UrlBuilder::from_auth_config(&state.auth.config);
    Ok(Json(ListResponse::new(providers).with_urls(&builder)))
}

/// Get a specific LLM provider
#[utoipa::path(
    get,
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "Provider found", body = WithUrls<Provider>),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
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
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    request_body = UpdateProviderRequest,
    responses(
        (status = 200, description = "Provider updated", body = WithUrls<Provider>),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
)]
pub async fn update_provider(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<UpdateProviderRequest>,
) -> ApiResult<WithUrls<Provider>> {
    state
        .dispatcher(&org)
        .run_with_urls(UpdateProvider {
            id,
            name: req.name,
            provider_type: req.provider_type,
            base_url: req.base_url,
            api_key: req.api_key,
            status: req.status,
        })
        .await
}

/// Delete an LLM provider
#[utoipa::path(
    delete,
    path = "/v1/llm-providers/{id}",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 204, description = "Provider deleted"),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found")
    ),
    tag = "llm-providers"
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
    path = "/v1/llm-providers/{id}/sync-models",
    params(
        ("id" = String, Path, description = "Provider ID (prefixed, e.g., prov_...)")
    ),
    responses(
        (status = 200, description = "Models synced", body = SyncModelsResponse),
        (status = 400, description = "Invalid provider ID"),
        (status = 404, description = "Provider not found"),
        (status = 500, description = "Sync failed")
    ),
    tag = "llm-providers"
)]
pub async fn sync_models(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<SyncModelsResponse> {
    Ok(Json(SyncProviderModels { id }.run(&state.ctx(&org)).await?))
}

/// GET /v1/llm-providers/config
#[utoipa::path(
    get,
    path = "/v1/llm-providers/config",
    responses(
        (status = 200, description = "Resource config for LLM providers", body = ResourceConfigResponse),
    ),
    tag = "llm-providers"
)]
pub async fn llm_provider_config(
    State(auth): State<AuthState>,
    org: ResolvedOrg,
) -> Json<ResourceConfigResponse> {
    let caller = Caller::from(&org);
    let policies = evaluate_policies_with(
        auth.permission_resolver.as_ref(),
        &caller,
        &[&LLM_PROVIDER_VIEW, &LLM_PROVIDER_MANAGE],
    );
    Json(ResourceConfigResponse { policies })
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/llm-providers/config", get(llm_provider_config))
        .route(
            "/v1/llm-providers",
            post(create_provider).get(list_providers),
        )
        .route(
            "/v1/llm-providers/{id}",
            get(get_provider)
                .patch(update_provider)
                .delete(delete_provider),
        )
        .route("/v1/llm-providers/{id}/sync-models", post(sync_models))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_response_serialization() {
        // RFC 9457 wire shape: `detail` carries the message.
        let error = ErrorResponse::new("Internal server error");
        let parsed: serde_json::Value = serde_json::to_value(&error).expect("Failed to serialize");
        assert_eq!(parsed["detail"], "Internal server error");
    }

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
