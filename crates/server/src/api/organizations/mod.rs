// Organization CRUD HTTP routes (Multitenancy)
//
// Note: Organization routes are NOT org-scoped (they are at the root level)
// because they manage organizations themselves.

use crate::auth::audit;
use crate::auth::middleware::{AuthState, AuthUser, OrgAdmin, OrgContext};
use crate::auth::rate_limit::OrgRateLimiter;
use crate::domains::audit_logs::record::{AuditEvent, ManagementAction};
use crate::domains::harnesses::record::BuiltInHarnessDefinition;
use crate::domains::organizations::record::{
    Organization, generate_org_public_id, validate_org_public_id,
};
pub use crate::domains::organizations::types::OrganizationResponse;
use crate::storage::{AddOrganizationMemberOutcome, StorageBackend};
use axum::{
    Json, Router,
    extract::{ConnectInfo, Extension, Path, State},
    http::{HeaderMap, StatusCode},
    routing::get,
};
#[cfg(test)]
use everruns_core::DEFAULT_ORG_ID;
use everruns_core::OrgRole;

use crate::api::command_http::CommandRouterExt;

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
/// the owner of org creation; wrappers only supply policy.
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
    pub org_initializers: Vec<Arc<dyn crate::setup::org_init::OrgInitializer>>,
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

impl AppState {
    /// Domain context for organization settings and account metadata commands.
    pub fn ctx(&self, org: &crate::auth::ResolvedOrg) -> crate::domains::common::Ctx {
        crate::domains::common::Ctx::minimal(
            everruns_core::Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
    }
}

impl_auth_state!(AppState);
super::dispatch::impl_dispatchable!(AppState);

/// Request to create a new organization
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateOrganizationRequest {
    /// The display name of the organization.
    #[schema(example = "Acme Corp")]
    pub name: String,
}

pub use crate::domains::organizations::types::UpdateOrganizationRequest;

/// Build organization routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .command::<crate::domains::organizations::provider_accounts::GetOrganizationAccount>()
        .command::<crate::domains::organizations::provider_accounts::UpdateOrganizationAccountName>(
        )
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
        .command::<crate::domains::organizations::egress_allowlist::GetOrgEgressAllowlist>()
        .command::<crate::domains::organizations::egress_allowlist::SetOrgEgressAllowlist>()
        .command::<crate::domains::organizations::egress_allowlist::SetOrgEgressAllowlistGrant>()
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
    use crate::storage::CreateOrganizationRow;

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

    // Validate input. Whitespace-only counts as empty — otherwise "   " would
    // create an org with a blank-looking, unrenamable-to-empty display name.
    if req.name.trim().is_empty() {
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
    if let Err(e) = crate::setup::org_init::initialize_org_harnesses_with_definitions(
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

    // Seed the default plugin marketplace (everruns/plugins) for the new org.
    // Non-fatal: if it fails (e.g. name conflict), org creation still succeeds.
    crate::setup::org_init::seed_default_plugin_marketplace(&state.db, row.org_id).await;

    // Post-create org initializers (EVE-811). Runs after built-in harnesses and
    // the default marketplace are provisioned, so embedder-provisioned per-org
    // resources (a managed provider, a default budget, an external tenant record)
    // are set up as part of org creation instead of by a follow-up reconciler.
    // No-op when no initializer is registered (default OSS behavior).
    //
    // A required initializer that fails aborts creation: the org row is rolled
    // back best-effort and a 500 is returned, so a caller never sees a "created"
    // org missing host-mandated resources. Optional initializers only log.
    if let Err(e) = crate::setup::org_init::run_org_initializers(
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
    use crate::domains::common::{Command, Ctx};
    use crate::domains::organizations::settings::UpdateOrg;
    if !validate_org_public_id(&org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }
    let membership = state
        .db
        .list_user_organizations(user.id)
        .await
        .log_internal_error_json("list user organizations")?
        .into_iter()
        .find(|org| org.public_id == org_public_id)
        .ok_or_not_found_json("Organization")?;
    let role = membership
        .role
        .parse::<OrgRole>()
        .unwrap_or(OrgRole::Member);
    let caller = everruns_core::Caller {
        org_id: membership.org_id,
        org_public_id: org_public_id.clone(),
        user_id: Some(user.id),
        role,
        is_platform_user: user.is_platform_user,
        is_internal: false,
        read_only: crate::auth::pat_scope::PatScope::restricts_writes(user.pat_scope),
    };
    let ctx = Ctx::minimal(
        caller,
        state.db.clone(),
        None,
        state.auth.permission_resolver.clone(),
    );
    let response = UpdateOrg {
        org: org_public_id.clone(),
        request: req,
    }
    .run(&ctx)
    .await?;
    let mut builder = AuditEvent::management(
        ManagementAction::OrgUpdated,
        membership.org_id,
        Some(user.id),
    )
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
    let agentid_agents_per_owner = db
        .agentid_agents_per_owner_setting(org_id)
        .await
        .log_internal_error_json("get AgentID owner cap")?;
    let web_search_defaults = db
        .org_web_search_defaults(org_id)
        .await
        .log_internal_error_json("get web search defaults")?;
    let background_limits = db
        .org_background_limits(org_id)
        .await
        .log_internal_error_json("get background limits")?;

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
        small_model_id: settings.as_ref().and_then(|s| s.small_model_id),
        chat_model_id: settings.as_ref().and_then(|s| s.chat_model_id),
        default_harness_id: settings.as_ref().and_then(|s| s.default_harness_id),
        base_harness_id: settings.as_ref().and_then(|s| s.base_harness_id),
        default_provider_per_service: settings
            .as_ref()
            .map(|s| s.default_provider_per_service.0.clone())
            .unwrap_or_default(),
        system_decisions: settings
            .as_ref()
            .map(|s| crate::storage::SystemDecisions::from_db(&s.system_decisions))
            .unwrap_or_default(),
        agentid_agents_per_owner,
        web_search_defaults,
        background_limits,
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

    let member_row = match state
        .db
        .add_organization_member_with_capacity(
            org.org_id,
            target_user_id,
            role.as_str(),
            state.resource_limits.max_members_per_org,
        )
        .await
        .log_internal_error_json("add organization member")?
    {
        AddOrganizationMemberOutcome::Added(member) => member,
        AddOrganizationMemberOutcome::AlreadyMember(_) => {
            return Err(
                ErrorResponse::new("User is already a member").into_response(StatusCode::CONFLICT)
            );
        }
        AddOrganizationMemberOutcome::MemberLimitReached => {
            return Err(ErrorResponse::new(format!(
                "Member limit reached (max {})",
                state.resource_limits.max_members_per_org
            ))
            .into_response(StatusCode::CONFLICT));
        }
    };

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
mod tests;
