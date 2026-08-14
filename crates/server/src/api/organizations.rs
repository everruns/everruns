// Organization CRUD HTTP routes (Multitenancy)
//
// Note: Organization routes are NOT org-scoped (they are at the root level)
// because they manage organizations themselves.

use crate::auth::audit;
use crate::auth::middleware::{AuthState, AuthUser, OrgAdmin, OrgContext};
use crate::auth::rate_limit::OrgRateLimiter;
use crate::storage::{StorageBackend, models::UpdateOrganizationSettings};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
use everruns_core::{DEFAULT_ORG_ID, OrgRole};
use everruns_durable::UpdateField;
use everruns_platform::{
    AuditEvent, BuiltInHarnessDefinition, ManagementAction, Organization, generate_org_public_id,
    validate_org_public_id,
};

use super::common::{
    ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, ListResponse, impl_auth_state,
};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::Arc;
use utoipa::ToSchema;

// ============================================================================
// Org creation policy extension point (EVE-607)
// ============================================================================

/// Context handed to an [`OrgCreatePolicy`] before any org or membership row is
/// written.
///
/// Wrappers (e.g. the SaaS distribution) use this to gate org creation on product
/// policy — verified email, account/resource limits — without forking the OSS
/// create-org handler or mounting a parallel `/v1/saas/orgs` endpoint. OSS remains
/// the owner of org creation; wrappers only supply policy. See `knowledge/foundations/embedding.md`.
pub struct OrgCreateContext<'a> {
    /// The authenticated user requesting creation.
    pub user: &'a AuthUser,
    /// The requested organization display name (already validated non-empty and
    /// ≤255 chars by the handler).
    pub org_name: &'a str,
}

/// Fail-closed rejection returned by an [`OrgCreatePolicy`].
///
/// The `status` and `message` are surfaced directly to the API client, so the
/// message must be safe and suitable for UI display — for example
/// `403 Please verify your email address before continuing.`
pub struct OrgCreateRejection {
    /// HTTP status returned to the client (e.g. `StatusCode::FORBIDDEN`).
    pub status: StatusCode,
    /// User-facing message rendered by the UI.
    pub message: String,
}

impl OrgCreateRejection {
    /// Reject with an explicit status and user-facing message.
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self {
            status,
            message: message.into(),
        }
    }

    /// Reject with `403 Forbidden` — the common case for policy gating.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(StatusCode::FORBIDDEN, message)
    }
}

/// Pre-create policy hook for organization creation.
///
/// Registered via [`ServerAppBuilder::org_create_policy`](crate::ServerAppBuilder::org_create_policy).
/// When a policy is present, OSS runs [`check`](OrgCreatePolicy::check) before
/// persisting any org or membership row; returning `Err` aborts creation with the
/// rejection's status and body and writes nothing. When no policy is registered,
/// default OSS create-org behavior is unchanged.
#[async_trait]
pub trait OrgCreatePolicy: Send + Sync {
    /// Decide whether the given user may create the requested organization.
    async fn check(&self, ctx: OrgCreateContext<'_>) -> Result<(), OrgCreateRejection>;
}

/// App state for organization routes
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub built_in_harnesses: Vec<BuiltInHarnessDefinition>,
    pub resource_limits: crate::server::ResourceLimitsConfig,
    pub org_rate_limiter: OrgRateLimiter,
    /// Optional wrapper-supplied pre-create policy (EVE-607). Runs before any
    /// org/membership row is written; `None` keeps default OSS behavior.
    pub org_create_policy: Option<Arc<dyn OrgCreatePolicy>>,
    /// Wrapper-supplied post-create initializers (EVE-811). Run after built-in
    /// harnesses and the default marketplace are provisioned for a new org; empty
    /// keeps default OSS behavior.
    pub org_initializers: Vec<Arc<dyn crate::org_init::OrgInitializer>>,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            db,
            auth,
            built_in_harnesses: crate::platform::oss_built_in_harnesses(),
            resource_limits: crate::server::ResourceLimitsConfig::from_env(),
            org_rate_limiter: OrgRateLimiter::default(),
            org_create_policy: None,
            org_initializers: Vec::new(),
        }
    }

    pub fn with_harnesses(
        db: Arc<StorageBackend>,
        auth: AuthState,
        built_in_harnesses: Vec<BuiltInHarnessDefinition>,
    ) -> Self {
        Self {
            db,
            auth,
            built_in_harnesses,
            resource_limits: crate::server::ResourceLimitsConfig::from_env(),
            org_rate_limiter: OrgRateLimiter::default(),
            org_create_policy: None,
            org_initializers: Vec::new(),
        }
    }
}

impl_auth_state!(AppState);

/// Request to create a new organization
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateOrganizationRequest {
    /// The display name of the organization.
    #[schema(example = "Acme Corp")]
    pub name: String,
}

/// Request to update an organization
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateOrganizationRequest {
    /// The display name of the organization.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "Acme Corporation")]
    pub name: Option<String>,
    /// Default LLM model for this organization. Must be an enabled model.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "model_01933b5a00007000800000000000001")]
    pub default_model_id: Option<everruns_provider::typed_id::ModelId>,
    /// Default harness to preselect in the UI for new sessions.
    /// Mutually exclusive with `default_harness_name`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000602")]
    pub default_harness_id: Option<everruns_provider::typed_id::HarnessId>,
    /// Alternative to `default_harness_id` — looked up by stable name within the org.
    /// Mutually exclusive with `default_harness_id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = "generic")]
    pub default_harness_name: Option<String>,
    /// Base harness to use when a session is started without an explicit harness_id.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>, example = "harness_01933b5a000070008000000000000601")]
    pub base_harness_id: Option<everruns_provider::typed_id::HarnessId>,
    /// Org-level default provider per service (EVE-569). Maps a service kind
    /// (`chat`, `embeddings`, `realtime`, `images`, `rerank`) to the provider id
    /// used as that service's default, consulted after an explicit binding and
    /// before the single-active-provider fallback. When present it **replaces**
    /// the whole map; each referenced provider must exist in the org.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<std::collections::HashMap<String, String>>)]
    pub default_provider_per_service: Option<
        std::collections::HashMap<
            everruns_provider::driver_registry::ServiceKind,
            everruns_provider::typed_id::ProviderId,
        >,
    >,
}

/// Response for organization operations
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct OrganizationResponse {
    /// External identifier (org_<32-hex-chars>)
    pub id: String,
    /// Display name
    pub name: String,
    /// Default LLM model for the organization.
    #[schema(value_type = Option<String>)]
    pub default_model_id: Option<everruns_provider::typed_id::ModelId>,
    /// Default harness to preselect in the UI.
    #[schema(value_type = Option<String>)]
    pub default_harness_id: Option<everruns_provider::typed_id::HarnessId>,
    /// Base harness used when session creation omits harness_id.
    #[schema(value_type = Option<String>)]
    pub base_harness_id: Option<everruns_provider::typed_id::HarnessId>,
    /// Org-level default provider per service (EVE-569), keyed by service kind.
    /// Empty when no org defaults are configured.
    #[schema(value_type = std::collections::HashMap<String, String>)]
    pub default_provider_per_service: std::collections::HashMap<
        everruns_provider::driver_registry::ServiceKind,
        everruns_provider::typed_id::ProviderId,
    >,
    /// When the organization was created
    pub created_at: chrono::DateTime<chrono::Utc>,
    /// When the organization was last updated
    pub updated_at: chrono::DateTime<chrono::Utc>,
    /// When the org's creator finished or skipped the setup wizard. `null` means
    /// onboarding is still incomplete, which the UI uses to resume the user at
    /// `/orgs/{id}/setup`. Seeded/default and externally-synced orgs are complete.
    pub onboarding_completed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Build organization routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs",
            get(list_organizations).post(create_organization),
        )
        .route(
            "/v1/orgs/{org}",
            get(get_organization).patch(update_organization),
        )
        .route(
            "/v1/orgs/{org}/onboarding/complete",
            axum::routing::post(complete_org_onboarding),
        )
        // Organization members
        .route("/v1/orgs/{org}/members", get(list_members).post(add_member))
        .route(
            "/v1/orgs/{org}/members/{user_id}",
            axum::routing::patch(update_member_role).delete(remove_member),
        )
        .with_state(state)
}

/// GET /v1/orgs - List organizations the current user belongs to
#[utoipa::path(
    get,
    path = "/v1/orgs",
    tag = "Organizations",
    responses(
        (status = 200, description = "List of organizations", body = ListResponse<OrganizationResponse>)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn list_organizations(
    State(state): State<AppState>,
    user: AuthUser,
) -> ApiResult<ListResponse<OrganizationResponse>> {
    // Query the database for fresh membership data.
    // Previously this read from user.organizations (populated at auth time),
    // which meant newly created orgs were invisible until re-login.
    let org_rows = state
        .db
        .list_user_organizations(user.id)
        .await
        .log_internal_error_json("list user organizations")?;

    let mut orgs = Vec::with_capacity(org_rows.len());
    for row in &org_rows {
        // Fetch full org details (including settings, timestamps) per org
        if let Some(org_row) = state
            .db
            .get_organization(row.org_id)
            .await
            .log_internal_error_json("get organization")?
        {
            orgs.push(build_organization_response(&state.db, row.org_id, org_row).await?);
        }
    }

    Ok(Json(ListResponse::new(orgs)))
}

/// POST /v1/orgs - Create a new organization
#[utoipa::path(
    post,
    path = "/v1/orgs",
    tag = "Organizations",
    request_body = CreateOrganizationRequest,
    responses(
        (status = 201, description = "Organization created", body = OrganizationResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn create_organization(
    State(state): State<AppState>,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(req): Json<CreateOrganizationRequest>,
) -> Result<(StatusCode, Json<OrganizationResponse>), (StatusCode, Json<ErrorResponse>)> {
    use crate::storage::models::CreateOrganizationRow;

    // Check per-user org creation rate limit before any DB work
    if state
        .org_rate_limiter
        .check_org_create(user.id)
        .await
        .is_err()
    {
        return Err(
            ErrorResponse::new("Too many requests. Please try again later.")
                .with_code("rate_limited")
                .with_retry_after(3600)
                .into_response(StatusCode::TOO_MANY_REQUESTS),
        );
    }

    // Validate input
    if req.name.is_empty() {
        return Err(ErrorResponse::new("Organization name cannot be empty")
            .into_response(StatusCode::BAD_REQUEST));
    }

    if req.name.len() > 255 {
        return Err(
            ErrorResponse::new("Organization name cannot exceed 255 characters")
                .into_response(StatusCode::BAD_REQUEST),
        );
    }

    // Pre-create policy hook (EVE-607). Wrappers gate creation here — before any
    // org or membership row is written — and may fail closed with a UI-facing
    // status/body. No-op when no policy is registered (default OSS behavior).
    if let Some(policy) = &state.org_create_policy
        && let Err(rejection) = policy
            .check(OrgCreateContext {
                user: &user,
                org_name: &req.name,
            })
            .await
    {
        return Err(ErrorResponse::new(rejection.message).into_response(rejection.status));
    }

    // Enforce org-per-user limit (counts orgs created by this user, not memberships)
    let org_count = state
        .db
        .count_user_created_organizations(user.id)
        .await
        .log_internal_error_json("count user created organizations")?;
    if org_count >= state.resource_limits.max_orgs_per_user {
        return Err(ErrorResponse::new(format!(
            "Organization limit reached (max {})",
            state.resource_limits.max_orgs_per_user
        ))
        .into_response(StatusCode::CONFLICT));
    }

    // Generate public_id
    let public_id = generate_org_public_id();

    // Create organization
    let row = state
        .db
        .create_organization(CreateOrganizationRow {
            public_id: public_id.clone(),
            name: req.name,
            created_by: Some(user.id),
        })
        .await
        .log_internal_error_json("create organization")?;

    // Add creator as organization owner
    state
        .db
        .add_organization_member(row.org_id, user.id, "owner")
        .await
        .log_internal_error_json("add organization member")?;

    // Initialize built-in harnesses for the new organization
    if let Err(e) = crate::org_init::initialize_org_harnesses_with_definitions(
        &state.db,
        row.org_id,
        &state.built_in_harnesses,
    )
    .await
    {
        tracing::warn!(
            org_id = row.org_id,
            error = %e,
            "Failed to initialize built-in harnesses for new org (non-fatal)"
        );
    }

    // Seed the default plugin marketplace (everruns/everruns) for the new org.
    // Non-fatal: if it fails (e.g. name conflict), org creation still succeeds.
    crate::org_init::seed_default_plugin_marketplace(&state.db, row.org_id).await;

    // Post-create org initializers (EVE-811). Runs after built-in harnesses and
    // the default marketplace are provisioned, so embedder-provisioned per-org
    // resources (a managed provider, a default budget, an external tenant record)
    // are set up as part of org creation instead of by a follow-up reconciler.
    // No-op when no initializer is registered (default OSS behavior).
    //
    // A required initializer that fails aborts creation: the org row is rolled
    // back best-effort and a 500 is returned, so a caller never sees a "created"
    // org missing host-mandated resources. Optional initializers only log.
    if let Err(e) = crate::org_init::run_org_initializers(
        &state.org_initializers,
        &state.db,
        row.org_id,
        Some(user.id),
    )
    .await
    {
        tracing::error!(
            org_id = row.org_id,
            initializer = %e.initializer,
            error = %e.source,
            "Required org initializer failed; rolling back org creation"
        );
        // Best-effort rollback so a failed provisioning does not leave a dangling
        // org the user owns. Cleanup failure is logged but the request still fails.
        match state.db.delete_organization(row.org_id).await {
            Ok(_) => {}
            Err(cleanup_err) => tracing::error!(
                org_id = row.org_id,
                error = %cleanup_err,
                "Failed to roll back org after initializer failure"
            ),
        }
        return Err(ErrorResponse::new("Failed to initialize organization")
            .into_response(StatusCode::INTERNAL_SERVER_ERROR));
    }

    // Seed agents are available as examples (GET /v1/agent-examples) and adopted
    // on demand via POST /v1/agent-examples/{slug}/use. No automatic seeding —
    // this prevents duplicate agents when users adopt from the examples gallery.

    let org_id = row.org_id;
    let org_public_id = row.public_id.clone();
    let response = build_organization_response(&state.db, org_id, row).await?;

    let mut builder = AuditEvent::management(ManagementAction::OrgCreated, org_id, Some(user.id))
        .target("org", &org_public_id)
        .detail("name", response.name.clone());
    if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
        builder = builder.ip(ip);
    }
    audit::emit_event(state.db.clone(), builder.build());

    Ok((StatusCode::CREATED, Json(response)))
}

/// GET /v1/orgs/:org - Get organization details
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}",
    tag = "Organizations",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    responses(
        (status = 200, description = "Organization details", body = OrganizationResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn get_organization(
    State(state): State<AppState>,
    user: AuthUser,
    Path(org_public_id): Path<String>,
) -> ApiResult<OrganizationResponse> {
    // Validate format
    if !validate_org_public_id(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Check user membership from DB (return 404 for non-members to prevent enumeration)
    if !is_member_of_public_db(&state.db, user.id, &org_public_id).await? {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Fetch organization details
    let row = state
        .db
        .get_organization_by_public_id(&org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;

    Ok(Json(
        build_organization_response(&state.db, row.org_id, row).await?,
    ))
}

/// PATCH /v1/orgs/:org - Update organization
#[utoipa::path(
    patch,
    path = "/v1/orgs/{org}",
    tag = "Organizations",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    request_body = UpdateOrganizationRequest,
    responses(
        (status = 200, description = "Organization updated", body = OrganizationResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn update_organization(
    State(state): State<AppState>,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Path(org_public_id): Path<String>,
    Json(req): Json<UpdateOrganizationRequest>,
) -> ApiResult<OrganizationResponse> {
    use crate::storage::models::UpdateOrganization;

    let updates_org_settings = req.default_model_id.is_some()
        || req.default_harness_id.is_some()
        || req.default_harness_name.is_some()
        || req.base_harness_id.is_some()
        || req.default_provider_per_service.is_some();

    // Validate format
    if !validate_org_public_id(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Check user membership from DB
    if !is_member_of_public_db(&state.db, user.id, &org_public_id).await? {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Only admin+ can update org-level defaults.
    if updates_org_settings
        && !is_org_admin_of_public_db(&state.db, user.id, &org_public_id).await?
    {
        return Err(ErrorResponse::new(
            "Only organization admins can update organization settings",
        )
        .into_response(StatusCode::FORBIDDEN));
    }

    // Validate input
    if let Some(ref name) = req.name {
        if name.is_empty() {
            return Err(ErrorResponse::new("Organization name cannot be empty")
                .into_response(StatusCode::BAD_REQUEST));
        }
        if name.len() > 255 {
            return Err(
                ErrorResponse::new("Organization name cannot exceed 255 characters")
                    .into_response(StatusCode::BAD_REQUEST),
            );
        }
    }

    // Get org_id from public_id
    let org_row = state
        .db
        .get_organization_by_public_id(&org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;

    // The built-in organization's name is protected, but an idempotent PATCH
    // that repeats the current name must not block unrelated settings updates.
    if org_row.org_id == DEFAULT_ORG_ID
        && req.name.as_deref().is_some_and(|name| name != org_row.name)
    {
        return Err(ErrorResponse::new("Cannot update default organization")
            .into_response(StatusCode::BAD_REQUEST));
    }

    let UpdateOrganizationRequest {
        name,
        default_model_id,
        mut default_harness_id,
        default_harness_name,
        base_harness_id,
        default_provider_per_service,
    } = req;

    // Resolve default_harness_name to default_harness_id (mutually exclusive)
    if default_harness_id.is_some() && default_harness_name.is_some() {
        return Err(ErrorResponse::new(
            "Cannot specify both default_harness_id and default_harness_name",
        )
        .into_response(StatusCode::BAD_REQUEST));
    }
    if let Some(ref harness_name) = default_harness_name {
        super::validation::validate_harness_name_strict(harness_name)?;
        let row = state
            .db
            .get_harness_by_name(org_row.org_id, harness_name)
            .await
            .log_internal_error_json("resolve default harness by name")?
            .ok_or_not_found_json("Harness")?;
        default_harness_id = Some(row.id);
    }

    // Validate referenced IDs exist (skip if already resolved from name)
    if let Some(ref model_id) = default_model_id {
        // Verify the model exists and is enabled
        let model = state
            .db
            .get_model_with_provider(org_row.org_id, model_id.uuid())
            .await
            .log_internal_error_json("resolve default model")?
            .ok_or_else(|| {
                ErrorResponse::new("Model not found").into_response(StatusCode::BAD_REQUEST)
            })?;
        if !model.enabled {
            return Err(ErrorResponse::new("Default model must be an enabled model")
                .into_response(StatusCode::BAD_REQUEST));
        }
    }
    if default_harness_name.is_none()
        && let Some(default_harness_id) = default_harness_id
    {
        state
            .db
            .get_harness(org_row.org_id, default_harness_id)
            .await
            .log_internal_error_json("resolve default harness")?
            .ok_or_not_found_json("Harness")?;
    }
    if let Some(base_harness_id) = base_harness_id {
        state
            .db
            .get_harness(org_row.org_id, base_harness_id)
            .await
            .log_internal_error_json("resolve base harness")?
            .ok_or_not_found_json("Harness")?;
    }
    // Validate every pinned provider exists in the org. Capability (the driver
    // actually declaring the service) is enforced fail-closed at resolve time in
    // ProviderResolverService::resolve_service, so we only guard existence here.
    if let Some(ref defaults) = default_provider_per_service {
        for provider_id in defaults.values() {
            state
                .db
                .get_provider(org_row.org_id, provider_id.uuid())
                .await
                .log_internal_error_json("resolve org default provider")?
                .ok_or_else(|| {
                    ErrorResponse::new(format!("Provider {provider_id} not found"))
                        .into_response(StatusCode::BAD_REQUEST)
                })?;
        }
    }

    // Update organization
    let input = UpdateOrganization { name };

    let row = state
        .db
        .update_organization(org_row.org_id, input)
        .await
        .log_internal_error_json("update organization")?
        .ok_or_not_found_json("Organization")?;

    if default_model_id.is_some()
        || default_harness_id.is_some()
        || base_harness_id.is_some()
        || default_provider_per_service.is_some()
    {
        state
            .db
            .patch_organization_settings(
                org_row.org_id,
                UpdateOrganizationSettings {
                    default_model_id: default_model_id
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                    default_harness_id: default_harness_id
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                    base_harness_id: base_harness_id
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                    default_provider_per_service: default_provider_per_service
                        .map_or(UpdateField::Unchanged, UpdateField::Set),
                },
            )
            .await
            .log_internal_error_json("update organization settings")?;
    }

    let response = build_organization_response(&state.db, row.org_id, row).await?;

    let mut builder =
        AuditEvent::management(ManagementAction::OrgUpdated, org_row.org_id, Some(user.id))
            .target("org", &org_public_id);
    if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
        builder = builder.ip(ip);
    }
    audit::emit_event(state.db.clone(), builder.build());

    Ok(Json(response))
}

/// POST /v1/orgs/:org/onboarding/complete - Mark the org's onboarding wizard as
/// finished (or skipped). Idempotent: the timestamp is set only when NULL.
///
/// Authz mirrors the org-scoped mutations above (admin+), but resolves
/// membership/role from the DB rather than the auth token — a brand-new org may
/// not yet appear in the caller's token, and onboarding completion is exactly
/// that just-created case.
#[utoipa::path(
    post,
    path = "/v1/orgs/{org}/onboarding/complete",
    tag = "Organizations",
    params(
        ("org" = String, Path, description = "Organization public ID")
    ),
    responses(
        (status = 200, description = "Onboarding marked complete", body = OrganizationResponse),
        (status = 403, description = "Not an admin of the organization", body = ErrorResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse)
    ),
    security(
        ("bearerAuth" = []),
        ("cookieAuth" = [])
    )
)]
pub async fn complete_org_onboarding(
    State(state): State<AppState>,
    user: AuthUser,
    Path(org_public_id): Path<String>,
) -> ApiResult<OrganizationResponse> {
    // Validate format (404 on bad shape to avoid enumeration).
    if !validate_org_public_id(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Membership check from DB (404 for non-members, prevents enumeration).
    if !is_member_of_public_db(&state.db, user.id, &org_public_id).await? {
        return Err(ErrorResponse::not_found("Organization"));
    }

    // Only admin+ (owner is admin+) may complete onboarding.
    if !is_org_admin_of_public_db(&state.db, user.id, &org_public_id).await? {
        return Err(
            ErrorResponse::new("Only organization admins can complete onboarding")
                .into_response(StatusCode::FORBIDDEN),
        );
    }

    let org_row = state
        .db
        .get_organization_by_public_id(&org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;

    state
        .db
        .mark_org_onboarding_complete(org_row.org_id)
        .await
        .log_internal_error_json("mark org onboarding complete")?;

    // Re-read so the response reflects the persisted completion timestamp.
    let row = state
        .db
        .get_organization_by_public_id(&org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;

    Ok(Json(
        build_organization_response(&state.db, row.org_id, row).await?,
    ))
}

/// Check membership by querying the DB (avoids stale auth context).
pub(crate) async fn is_member_of_public_db(
    db: &StorageBackend,
    user_id: uuid::Uuid,
    org_public_id: &str,
) -> Result<bool, (StatusCode, Json<ErrorResponse>)> {
    let orgs = db
        .list_user_organizations(user_id)
        .await
        .log_internal_error_json("list user organizations")?;
    Ok(orgs.iter().any(|o| o.public_id == org_public_id))
}

async fn is_org_admin_of_public_db(
    db: &StorageBackend,
    user_id: uuid::Uuid,
    org_public_id: &str,
) -> Result<bool, (StatusCode, Json<ErrorResponse>)> {
    let orgs = db
        .list_user_organizations(user_id)
        .await
        .log_internal_error_json("list user organizations")?;
    Ok(orgs
        .iter()
        .find(|o| o.public_id == org_public_id)
        .and_then(|o| o.role.parse::<OrgRole>().ok())
        .is_some_and(|role| role.has_permission(OrgRole::Admin)))
}

async fn build_organization_response(
    db: &StorageBackend,
    org_id: i64,
    row: crate::storage::OrganizationRow,
) -> Result<OrganizationResponse, (StatusCode, Json<ErrorResponse>)> {
    let settings = db
        .get_organization_settings(org_id)
        .await
        .log_internal_error_json("get organization settings")?;

    let onboarding_completed_at = row.onboarding_completed_at;
    let org = Organization {
        public_id: row.public_id,
        name: row.name,
        created_at: row.created_at,
        updated_at: row.updated_at,
    };

    Ok(OrganizationResponse {
        id: org.public_id,
        name: org.name,
        default_model_id: settings.as_ref().and_then(|s| s.default_model_id),
        default_harness_id: settings.as_ref().and_then(|s| s.default_harness_id),
        base_harness_id: settings.as_ref().and_then(|s| s.base_harness_id),
        default_provider_per_service: settings
            .as_ref()
            .map(|s| s.default_provider_per_service.0.clone())
            .unwrap_or_default(),
        created_at: org.created_at,
        updated_at: org.updated_at,
        onboarding_completed_at,
    })
}

// ============================================================================
// Organization Members
// ============================================================================

/// Response for organization member
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct MemberResponse {
    /// Owning user's UUID.
    pub user_id: String,
    pub email: String,
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    pub avatar_url: Option<String>,
    pub role: String,
    pub joined_at: String,
}

/// Request to add a member
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddMemberRequest {
    /// Owning user's UUID.
    pub user_id: String,
    #[serde(default = "default_member_role")]
    pub role: String,
}

fn default_member_role() -> String {
    "member".to_string()
}

/// Request to update member role
#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateMemberRoleRequest {
    pub role: String,
}

/// GET /v1/orgs/:org/members - List organization members
pub async fn list_members(
    State(state): State<AppState>,
    org: OrgContext,
) -> ApiResult<ListResponse<MemberResponse>> {
    let members = state
        .db
        .list_organization_members_with_users(org.org_id)
        .await
        .log_internal_error_json("list organization members")?;

    let items: Vec<MemberResponse> = members
        .into_iter()
        .map(|m| MemberResponse {
            user_id: m.user_id.to_string(),
            email: m.email,
            name: m.name,
            avatar_url: m.avatar_url,
            role: m.role,
            joined_at: m.joined_at.to_rfc3339(),
        })
        .collect();

    Ok(Json(ListResponse::new(items)))
}

/// POST /v1/orgs/:org/members - Add a member (Admin+)
pub async fn add_member(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(req): Json<AddMemberRequest>,
) -> Result<(StatusCode, Json<MemberResponse>), (StatusCode, Json<ErrorResponse>)> {
    // Parse and validate role
    let role: OrgRole = req.role.parse().map_err(|_| {
        ErrorResponse::new("Invalid role. Must be 'owner', 'admin', or 'member'")
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    // Only owners can add owners
    if role == OrgRole::Owner && !org.role.has_permission(OrgRole::Owner) {
        return Err(
            ErrorResponse::new("Only owners can add owners").into_response(StatusCode::FORBIDDEN)
        );
    }

    let target_user_id: uuid::Uuid = req.user_id.parse().map_err(|_| {
        ErrorResponse::new("Invalid user_id").into_response(StatusCode::BAD_REQUEST)
    })?;

    // Verify user exists
    let target_user = state
        .db
        .get_user(target_user_id)
        .await
        .log_internal_error_json("get user")?
        .ok_or_else(|| ErrorResponse::new("User not found").into_response(StatusCode::NOT_FOUND))?;

    // Check if already a member
    let existing = state
        .db
        .get_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("check membership")?;

    if existing.is_some() {
        return Err(
            ErrorResponse::new("User is already a member").into_response(StatusCode::CONFLICT)
        );
    }

    // Enforce member-per-org limit
    let member_count = state
        .db
        .count_organization_members(org.org_id)
        .await
        .log_internal_error_json("count organization members")?;
    if member_count >= state.resource_limits.max_members_per_org {
        return Err(ErrorResponse::new(format!(
            "Member limit reached (max {})",
            state.resource_limits.max_members_per_org
        ))
        .into_response(StatusCode::CONFLICT));
    }

    // Add member
    let member_row = state
        .db
        .add_organization_member(org.org_id, target_user_id, role.as_str())
        .await
        .log_internal_error_json("add organization member")?;

    let mut builder =
        AuditEvent::management(ManagementAction::MemberInvited, org.org_id, Some(user.id))
            .target("member", target_user_id.to_string())
            .detail("role", member_row.role.clone());
    if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
        builder = builder.ip(ip);
    }
    audit::emit_event(state.db.clone(), builder.build());

    Ok((
        StatusCode::CREATED,
        Json(MemberResponse {
            user_id: target_user_id.to_string(),
            email: target_user.email,
            name: target_user.name,
            avatar_url: target_user.avatar_url,
            role: member_row.role,
            joined_at: member_row.created_at.to_rfc3339(),
        }),
    ))
}

/// PATCH /v1/orgs/:org/members/:user_id - Update member role
pub async fn update_member_role(
    State(state): State<AppState>,
    OrgAdmin(org): OrgAdmin,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Path((_org_public_id, user_id_str)): Path<(String, String)>,
    Json(req): Json<UpdateMemberRoleRequest>,
) -> ApiResult<MemberResponse> {
    let new_role: OrgRole = req.role.parse().map_err(|_| {
        ErrorResponse::new("Invalid role. Must be 'owner', 'admin', or 'member'")
            .into_response(StatusCode::BAD_REQUEST)
    })?;

    let target_user_id: uuid::Uuid = user_id_str.parse().map_err(|_| {
        ErrorResponse::new("Invalid user_id").into_response(StatusCode::BAD_REQUEST)
    })?;

    // Get current member info
    let current = state
        .db
        .get_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("get member")?
        .ok_or_else(|| {
            ErrorResponse::new("Member not found").into_response(StatusCode::NOT_FOUND)
        })?;

    let current_role: OrgRole = current.role.parse().unwrap_or(OrgRole::Member);

    // Only owners can change owner roles
    if (current_role == OrgRole::Owner || new_role == OrgRole::Owner)
        && !org.role.has_permission(OrgRole::Owner)
    {
        return Err(ErrorResponse::new("Only owners can change owner roles")
            .into_response(StatusCode::FORBIDDEN));
    }

    // Cannot demote last owner
    if current_role == OrgRole::Owner && new_role != OrgRole::Owner {
        let owner_count = state
            .db
            .count_organization_owners(org.org_id)
            .await
            .log_internal_error_json("count owners")?;
        if owner_count <= 1 {
            return Err(ErrorResponse::new("Cannot remove the last owner")
                .into_response(StatusCode::BAD_REQUEST));
        }
    }

    // Update role
    let updated = state
        .db
        .update_organization_member_role(org.org_id, target_user_id, new_role.as_str())
        .await
        .log_internal_error_json("update member role")?
        .ok_or_else(|| {
            ErrorResponse::new("Member not found").into_response(StatusCode::NOT_FOUND)
        })?;

    let mut builder = AuditEvent::management(
        ManagementAction::MemberRoleChanged,
        org.org_id,
        Some(user.id),
    )
    .target("member", target_user_id.to_string())
    .detail("old_role", current_role.as_str())
    .detail("new_role", updated.role.clone());
    if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
        builder = builder.ip(ip);
    }
    audit::emit_event(state.db.clone(), builder.build());

    Ok(Json(MemberResponse {
        user_id: target_user_id.to_string(),
        email: current.email,
        name: current.name,
        avatar_url: current.avatar_url,
        role: updated.role,
        joined_at: current.joined_at.to_rfc3339(),
    }))
}

/// DELETE /v1/orgs/:org/members/:user_id - Remove member (Owner or self)
pub async fn remove_member(
    State(state): State<AppState>,
    org: OrgContext,
    user: AuthUser,
    connect_info: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Path((_org_public_id, user_id_str)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let target_user_id: uuid::Uuid = user_id_str.parse().map_err(|_| {
        ErrorResponse::new("Invalid user_id").into_response(StatusCode::BAD_REQUEST)
    })?;

    let is_self = target_user_id == user.id;

    // Must be owner to remove others (self-removal always allowed)
    if !is_self && !org.role.has_permission(OrgRole::Owner) {
        return Err(ErrorResponse::new("Only owners can remove members")
            .into_response(StatusCode::FORBIDDEN));
    }

    // Check if target is owner — cannot remove last owner
    let member = state
        .db
        .get_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("get member")?
        .ok_or_else(|| {
            ErrorResponse::new("Member not found").into_response(StatusCode::NOT_FOUND)
        })?;

    if member.role == "owner" {
        let owner_count = state
            .db
            .count_organization_owners(org.org_id)
            .await
            .log_internal_error_json("count owners")?;
        if owner_count <= 1 {
            return Err(ErrorResponse::new("Cannot remove the last owner")
                .into_response(StatusCode::BAD_REQUEST));
        }
    }

    let removed = state
        .db
        .remove_organization_member(org.org_id, target_user_id)
        .await
        .log_internal_error_json("remove organization member")?;

    if removed {
        let mut builder =
            AuditEvent::management(ManagementAction::MemberRemoved, org.org_id, Some(user.id))
                .target("member", target_user_id.to_string())
                .detail("removed_role", member.role.clone());
        if let Some(ip) = audit::client_ip_from_connect_info(connect_info, &headers) {
            builder = builder.ip(ip);
        }
        audit::emit_event(state.db.clone(), builder.build());

        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Member not found").into_response(StatusCode::NOT_FOUND))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::backend::AuthBackend;
    use crate::auth::config::{AuthConfig, AuthMode, JwtConfig};
    use crate::auth::middleware::{AuthError, AuthMethod};
    use crate::auth::routes::AuthConfigResponse;
    use axum::body::Body;
    use axum::http::Request;
    use everruns_platform::OrgMembership;
    use http_body_util::BodyExt;
    use tower::ServiceExt;
    use uuid::Uuid;

    // ---- Org create policy extension point (EVE-607) ----

    /// Minimal auth backend that authenticates every request as one fixed user.
    #[derive(Clone)]
    struct MockAuthBackend {
        user_id: Uuid,
    }

    #[async_trait]
    impl AuthBackend for MockAuthBackend {
        async fn validate_token(&self, _token: &str) -> Result<AuthUser, AuthError> {
            Ok(AuthUser {
                id: self.user_id,
                email: "test@example.com".to_string(),
                name: "Test User".to_string(),
                roles: vec!["user".to_string()],
                is_platform_user: true,
                auth_method: AuthMethod::Jwt,
                organizations: vec![OrgMembership {
                    org_id: DEFAULT_ORG_ID,
                    public_id: "org_00000000000000000000000000000001".to_string(),
                    name: "Default Organization".to_string(),
                    role: OrgRole::Owner,
                }],
            })
        }

        async fn validate_personal_access_token(
            &self,
            _token: &str,
        ) -> Result<AuthUser, AuthError> {
            Err(AuthError::unauthorized("not supported"))
        }

        fn auth_routes(&self) -> Option<Router> {
            None
        }

        fn auth_config_response(&self) -> AuthConfigResponse {
            AuthConfigResponse {
                mode: "full".to_string(),
                login_origin: None,
                password_auth_enabled: false,
                signup_enabled: false,
                oauth_providers: vec![],
                signup_email_confirm: false,
                captcha: None,
            }
        }
    }

    /// Policy that always rejects with `403` and a UI-facing message.
    struct RejectAllPolicy {
        message: &'static str,
    }

    #[async_trait]
    impl OrgCreatePolicy for RejectAllPolicy {
        async fn check(&self, _ctx: OrgCreateContext<'_>) -> Result<(), OrgCreateRejection> {
            Err(OrgCreateRejection::forbidden(self.message))
        }
    }

    /// Build a create-org router over an in-memory DB, optionally with a policy.
    fn create_org_app(
        policy: Option<Arc<dyn OrgCreatePolicy>>,
    ) -> (Router, Arc<StorageBackend>, Uuid) {
        let user_id = Uuid::now_v7();
        let db = Arc::new(StorageBackend::in_memory());
        let config = AuthConfig {
            mode: AuthMode::Full,
            jwt: JwtConfig {
                secret: "test-secret-for-unit-tests-only".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let auth = AuthState::new(config, Arc::new(MockAuthBackend { user_id }));
        let mut state = AppState::new(db.clone(), auth);
        state.org_create_policy = policy;
        (routes(state), db, user_id)
    }

    fn create_org_request(name: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/v1/orgs")
            .header("Authorization", "Bearer test-token")
            .header("content-type", "application/json")
            .body(Body::from(format!(r#"{{"name":"{name}"}}"#)))
            .unwrap()
    }

    fn update_default_org_request(name: &str) -> Request<Body> {
        update_default_org_json_request(format!(r#"{{"name":"{name}"}}"#))
    }

    fn update_default_org_json_request(body: impl Into<Body>) -> Request<Body> {
        Request::builder()
            .method("PATCH")
            .uri("/v1/orgs/org_00000000000000000000000000000001")
            .header("Authorization", "Bearer test-token")
            .header("content-type", "application/json")
            .body(body.into())
            .unwrap()
    }

    #[tokio::test]
    async fn default_organization_accepts_unchanged_name() {
        let (app, db, user_id) = create_org_app(None);
        db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
            .await
            .unwrap();

        let response = app
            .oneshot(update_default_org_request("Default Organization"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn default_organization_still_rejects_renames() {
        let (app, db, user_id) = create_org_app(None);
        db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
            .await
            .unwrap();

        let response = app
            .oneshot(update_default_org_request("Renamed"))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn organization_settings_require_database_admin_role() {
        let (app, db, user_id) = create_org_app(None);
        db.add_organization_member(DEFAULT_ORG_ID, user_id, "member")
            .await
            .unwrap();

        let response = app
            .oneshot(update_default_org_json_request(
                r#"{"default_model_id":"model_01933b5a00007000800000000000030b"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["detail"],
            "Only organization admins can update organization settings"
        );
    }

    #[tokio::test]
    async fn organization_rejects_stale_default_model() {
        let (app, db, user_id) = create_org_app(None);
        db.add_organization_member(DEFAULT_ORG_ID, user_id, "owner")
            .await
            .unwrap();

        let response = app
            .oneshot(update_default_org_json_request(
                r#"{"default_model_id":"model_01933b5a00007000800000000000030b"}"#,
            ))
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["detail"], "Model not found");
    }

    #[tokio::test]
    async fn create_organization_succeeds_without_policy() {
        // Default OSS behavior: no policy registered, creation proceeds.
        let (app, db, user_id) = create_org_app(None);

        let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        // The org row and owner membership were persisted.
        let count = db.count_user_created_organizations(user_id).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn org_create_policy_rejects_before_db_write() {
        let policy: Arc<dyn OrgCreatePolicy> = Arc::new(RejectAllPolicy {
            message: "Please verify your email address before continuing.",
        });
        let (app, db, user_id) = create_org_app(Some(policy));

        let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);

        let body = response.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            json["detail"],
            "Please verify your email address before continuing."
        );

        // Fail-closed: no org or membership row was written.
        let count = db.count_user_created_organizations(user_id).await.unwrap();
        assert_eq!(count, 0);
        assert!(
            db.list_user_organizations(user_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn org_create_policy_allows_creation() {
        // A policy that returns Ok must not change default behavior.
        struct AllowPolicy;
        #[async_trait]
        impl OrgCreatePolicy for AllowPolicy {
            async fn check(&self, _ctx: OrgCreateContext<'_>) -> Result<(), OrgCreateRejection> {
                Ok(())
            }
        }
        let policy: Arc<dyn OrgCreatePolicy> = Arc::new(AllowPolicy);
        let (app, db, user_id) = create_org_app(Some(policy));

        let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let count = db.count_user_created_organizations(user_id).await.unwrap();
        assert_eq!(count, 1);
    }

    // ------------------------------------------------------------------------
    // Post-create org initializers (EVE-811)
    // ------------------------------------------------------------------------

    use crate::org_init::{OrgInitContext, OrgInitializer};

    /// Build a create-org router with the given post-create initializers.
    fn create_org_app_with_initializers(
        initializers: Vec<Arc<dyn OrgInitializer>>,
    ) -> (Router, Arc<StorageBackend>, Uuid) {
        let user_id = Uuid::now_v7();
        let db = Arc::new(StorageBackend::in_memory());
        let config = AuthConfig {
            mode: AuthMode::Full,
            jwt: JwtConfig {
                secret: "test-secret-for-unit-tests-only".to_string(),
                ..Default::default()
            },
            ..Default::default()
        };
        let auth = AuthState::new(config, Arc::new(MockAuthBackend { user_id }));
        let mut state = AppState::new(db.clone(), auth);
        state.org_initializers = initializers;
        (routes(state), db, user_id)
    }

    #[tokio::test]
    async fn org_initializer_runs_after_org_created() {
        use std::sync::Mutex;

        /// (org_id, created_by) recorded by the initializer.
        type SeenOrg = Arc<Mutex<Option<(i64, Option<Uuid>)>>>;

        /// Records the org id and creating user it was invoked with.
        struct RecordingInitializer {
            seen: SeenOrg,
        }
        #[async_trait]
        impl OrgInitializer for RecordingInitializer {
            async fn on_org_created(&self, ctx: OrgInitContext<'_>) -> anyhow::Result<()> {
                // The org exists by the time the initializer runs, and its
                // built-in harnesses are already provisioned.
                let harnesses = ctx.db.list_harnesses(ctx.org_id, None, false).await?;
                assert!(
                    !harnesses.is_empty(),
                    "harnesses should be provisioned before initializers run"
                );
                *self.seen.lock().unwrap() = Some((ctx.org_id, ctx.created_by));
                Ok(())
            }
        }

        let seen = Arc::new(Mutex::new(None));
        let init: Arc<dyn OrgInitializer> = Arc::new(RecordingInitializer { seen: seen.clone() });
        let (app, db, user_id) = create_org_app_with_initializers(vec![init]);

        let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let recorded = seen.lock().unwrap().expect("initializer must have run");
        assert_eq!(
            recorded.1,
            Some(user_id),
            "created_by is the requesting user"
        );
        // The org persisted and the recorded id matches it.
        let orgs = db.list_user_organizations(user_id).await.unwrap();
        assert_eq!(orgs.len(), 1);
        assert_eq!(recorded.0, orgs[0].org_id);
    }

    #[tokio::test]
    async fn required_org_initializer_failure_aborts_and_rolls_back() {
        struct FailingRequired;
        #[async_trait]
        impl OrgInitializer for FailingRequired {
            async fn on_org_created(&self, _ctx: OrgInitContext<'_>) -> anyhow::Result<()> {
                anyhow::bail!("provisioning failed")
            }
            fn name(&self) -> &str {
                "failing-required"
            }
        }

        let init: Arc<dyn OrgInitializer> = Arc::new(FailingRequired);
        let (app, db, user_id) = create_org_app_with_initializers(vec![init]);

        let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);

        // The org was rolled back: no user-owned org survives.
        let count = db.count_user_created_organizations(user_id).await.unwrap();
        assert_eq!(
            count, 0,
            "failed required initializer must roll back the org"
        );
        assert!(
            db.list_user_organizations(user_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn optional_org_initializer_failure_is_non_fatal() {
        struct FailingOptional;
        #[async_trait]
        impl OrgInitializer for FailingOptional {
            async fn on_org_created(&self, _ctx: OrgInitContext<'_>) -> anyhow::Result<()> {
                anyhow::bail!("best-effort provisioning failed")
            }
            fn required(&self) -> bool {
                false
            }
            fn name(&self) -> &str {
                "failing-optional"
            }
        }

        let init: Arc<dyn OrgInitializer> = Arc::new(FailingOptional);
        let (app, db, user_id) = create_org_app_with_initializers(vec![init]);

        let response = app.oneshot(create_org_request("Acme Corp")).await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        // Org still created despite the optional initializer failing.
        let count = db.count_user_created_organizations(user_id).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn mark_org_onboarding_complete_is_idempotent() {
        use crate::storage::models::CreateOrganizationRow;

        let db = StorageBackend::in_memory();
        let org = db
            .create_organization(CreateOrganizationRow {
                public_id: generate_org_public_id(),
                name: "New Org".to_string(),
                created_by: Some(Uuid::now_v7()),
            })
            .await
            .unwrap();
        // A freshly created (user-owned) org starts un-onboarded.
        assert!(org.onboarding_completed_at.is_none());

        db.mark_org_onboarding_complete(org.org_id).await.unwrap();
        let after = db.get_organization(org.org_id).await.unwrap().unwrap();
        let first = after.onboarding_completed_at.expect("timestamp set");

        // A second call must be a no-op — the completion time never moves.
        db.mark_org_onboarding_complete(org.org_id).await.unwrap();
        let after2 = db.get_organization(org.org_id).await.unwrap().unwrap();
        assert_eq!(after2.onboarding_completed_at, Some(first));
    }

    #[tokio::test]
    async fn seeded_org_is_created_already_onboarded() {
        use crate::storage::models::CreateOrganizationRow;

        let db = StorageBackend::in_memory();

        // The pre-seeded default org is already onboarded.
        let default_org = db.get_organization(DEFAULT_ORG_ID).await.unwrap().unwrap();
        assert!(default_org.onboarding_completed_at.is_some());

        // Orgs created via the seeding path (`create_organization_with_id`) are
        // likewise created already-complete.
        let seeded = db
            .create_organization_with_id(
                4242,
                CreateOrganizationRow {
                    public_id: generate_org_public_id(),
                    name: "Seeded".to_string(),
                    created_by: None,
                },
            )
            .await
            .unwrap()
            .unwrap();
        assert!(seeded.onboarding_completed_at.is_some());
    }

    #[tokio::test]
    async fn complete_org_onboarding_marks_and_is_idempotent() {
        let (app, db, _user_id) = create_org_app(None);

        // Create an org — the caller becomes owner and onboarding starts NULL.
        let resp = app
            .clone()
            .oneshot(create_org_request("Acme Corp"))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let org_public_id = json["id"].as_str().unwrap().to_string();
        assert!(json["onboarding_completed_at"].is_null());

        let complete_req = |id: &str| {
            Request::builder()
                .method("POST")
                .uri(format!("/v1/orgs/{id}/onboarding/complete"))
                .header("Authorization", "Bearer test-token")
                .body(Body::empty())
                .unwrap()
        };

        let resp = app
            .clone()
            .oneshot(complete_req(&org_public_id))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = resp.into_body().collect().await.unwrap().to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert!(!json["onboarding_completed_at"].is_null());

        let first = db
            .get_organization_by_public_id(&org_public_id)
            .await
            .unwrap()
            .unwrap()
            .onboarding_completed_at
            .expect("marked complete");

        // Idempotent: a repeat call still returns 200 and never moves the time.
        let resp = app
            .clone()
            .oneshot(complete_req(&org_public_id))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let again = db
            .get_organization_by_public_id(&org_public_id)
            .await
            .unwrap()
            .unwrap()
            .onboarding_completed_at
            .expect("still complete");
        assert_eq!(first, again);
    }

    #[test]
    fn test_organization_response_fields() {
        let response = OrganizationResponse {
            id: "org_00000000000000000000000000000001".to_string(),
            name: "Test Org".to_string(),
            default_model_id: None,
            default_harness_id: Some("harness_01933b5a000070008000000000000602".parse().unwrap()),
            base_harness_id: Some("harness_01933b5a000070008000000000000601".parse().unwrap()),
            default_provider_per_service: std::collections::HashMap::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            onboarding_completed_at: None,
        };

        assert_eq!(response.id, "org_00000000000000000000000000000001");
        assert_eq!(response.name, "Test Org");
        assert!(response.default_harness_id.is_some());
        assert!(response.base_harness_id.is_some());
    }

    // Trivial derive-only serde round-trips removed; covered by the derive + handler tests.

    #[test]
    fn test_create_request_empty_name() {
        let json = r#"{"name": ""}"#;
        let req: CreateOrganizationRequest = serde_json::from_str(json).unwrap();
        assert!(req.name.is_empty());
    }

    #[test]
    fn test_update_request_partial() {
        let json = r#"{}"#;
        let req: UpdateOrganizationRequest = serde_json::from_str(json).unwrap();
        assert!(req.name.is_none());
        assert!(req.default_harness_id.is_none());
        assert!(req.base_harness_id.is_none());

        let json = r#"{
            "name": "New Name",
            "default_harness_id": "harness_01933b5a000070008000000000000602",
            "base_harness_id": "harness_01933b5a000070008000000000000601"
        }"#;
        let req: UpdateOrganizationRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.name.unwrap(), "New Name");
        assert!(req.default_harness_id.is_some());
        assert!(req.base_harness_id.is_some());
    }
}
