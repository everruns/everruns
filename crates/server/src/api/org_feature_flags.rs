// Organization feature flag opt-in API
//
// Decision: Effective flags = deployment/system gate AND org opt-in (default off).
// Decision: GET settings exposes catalog for admin UI; PATCH requires OrgAdmin.
// Decision: platform-managed flags are org-scoped but not the org's to set. A
// separate PlatformUser-gated route owns them, so the operator console can
// enrol one tenant without the tenant being able to enrol itself, and neither
// actor can quietly do the other's job.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use everruns_platform::validate_org_public_id;
use everruns_platform::{FeatureFlagMap, FeatureFlags};

use crate::auth::middleware::{AuthState, OrgAdmin, PlatformUser};
use crate::services::org_feature_flags::{
    OrgFeatureFlagsSettingsResponse, build_all_feature_flag_settings,
    build_org_feature_flag_settings, validate_org_feature_flag_updates,
    validate_platform_feature_flag_updates,
};
use crate::storage::StorageBackend;

use super::common::{ApiOptionExt, ApiResult, ApiResultExt, ErrorResponse, impl_auth_state};
use super::organizations::is_member_of_public_db;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub system_feature_flags: FeatureFlags,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        system_feature_flags: FeatureFlags,
    ) -> Self {
        Self {
            db,
            auth,
            system_feature_flags,
        }
    }
}

impl_auth_state!(AppState);

#[derive(Debug, Clone, serde::Deserialize, utoipa::ToSchema)]
pub struct UpdateOrgFeatureFlagsRequest {
    /// Map of flag name -> enabled. Omitted flags are unchanged.
    pub flags: HashMap<String, bool>,
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/orgs/{org}/feature-flags",
            get(get_org_feature_flags).patch(update_org_feature_flags),
        )
        .route(
            "/v1/orgs/{org}/feature-flags/settings",
            get(get_org_feature_flag_settings),
        )
        .route(
            "/v1/orgs/{org}/feature-flags/platform",
            get(get_platform_feature_flag_settings).patch(update_platform_feature_flags),
        )
        .with_state(state)
}

async fn resolve_path_org(
    state: &AppState,
    user_id: uuid::Uuid,
    org_public_id: &str,
) -> Result<i64, (axum::http::StatusCode, Json<ErrorResponse>)> {
    if !validate_org_public_id(org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }
    if !is_member_of_public_db(&state.db, user_id, org_public_id).await? {
        return Err(ErrorResponse::not_found("Organization"));
    }
    let row = state
        .db
        .get_organization_by_public_id(org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;
    Ok(row.org_id)
}

/// GET /v1/orgs/{org}/feature-flags — effective flags for the organization.
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/feature-flags",
    tag = "Organizations",
    params(("org" = String, Path, description = "Organization public id")),
    responses(
        (status = 200, description = "Effective feature flags", body = FeatureFlagMap),
        (status = 404, description = "Organization not found", body = ErrorResponse),
    ),
    security(("bearerAuth" = []), ("cookieAuth" = []))
)]
pub async fn get_org_feature_flags(
    State(state): State<AppState>,
    Path(org_public_id): Path<String>,
    user: crate::auth::AuthUser,
) -> ApiResult<FeatureFlagMap> {
    let org_id = resolve_path_org(&state, user.id, &org_public_id).await?;
    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org_id,
        &state.system_feature_flags,
    )
    .await
    .log_internal_error_json("resolve org feature flags")?;
    Ok(Json(flags.to_map()))
}

/// GET /v1/orgs/{org}/feature-flags/settings — catalog with system/org/effective state.
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/feature-flags/settings",
    tag = "Organizations",
    params(("org" = String, Path, description = "Organization public id")),
    responses(
        (status = 200, description = "Feature flag settings", body = OrgFeatureFlagsSettingsResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse),
    ),
    security(("bearerAuth" = []), ("cookieAuth" = []))
)]
pub async fn get_org_feature_flag_settings(
    State(state): State<AppState>,
    Path(org_public_id): Path<String>,
    user: crate::auth::AuthUser,
) -> ApiResult<OrgFeatureFlagsSettingsResponse> {
    let org_id = resolve_path_org(&state, user.id, &org_public_id).await?;
    let org_enabled = state
        .db
        .list_org_feature_flags(org_id)
        .await
        .log_internal_error_json("list org feature flags")?;
    Ok(Json(OrgFeatureFlagsSettingsResponse {
        flags: build_org_feature_flag_settings(&state.system_feature_flags, &org_enabled),
    }))
}

/// PATCH /v1/orgs/{org}/feature-flags — update org opt-in (admin only).
#[utoipa::path(
    patch,
    path = "/v1/orgs/{org}/feature-flags",
    tag = "Organizations",
    params(("org" = String, Path, description = "Organization public id")),
    request_body = UpdateOrgFeatureFlagsRequest,
    responses(
        (status = 200, description = "Updated effective flags", body = FeatureFlagMap),
        (status = 400, description = "Invalid flag", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse),
    ),
    security(("bearerAuth" = []), ("cookieAuth" = []))
)]
pub async fn update_org_feature_flags(
    State(state): State<AppState>,
    Path(org_public_id): Path<String>,
    OrgAdmin(org): OrgAdmin,
    Json(req): Json<UpdateOrgFeatureFlagsRequest>,
) -> ApiResult<FeatureFlagMap> {
    if org.public_id != org_public_id {
        return Err(ErrorResponse::not_found("Organization"));
    }
    if let Err(msg) = validate_org_feature_flag_updates(&state.system_feature_flags, &req.flags) {
        return Err(ErrorResponse::new(msg).into_response(axum::http::StatusCode::BAD_REQUEST));
    }

    state
        .db
        .replace_org_feature_flags(org.org_id, &req.flags)
        .await
        .log_internal_error_json("update org feature flags")?;

    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org.org_id,
        &state.system_feature_flags,
    )
    .await
    .log_internal_error_json("resolve org feature flags")?;
    Ok(Json(flags.to_map()))
}

/// GET /v1/orgs/{org}/feature-flags/platform — every flag, including the
/// platform-managed ones, for the operator console.
///
/// Platform users only. The tenant-facing settings route deliberately omits
/// these rows, so this is where an operator sees what a tenant is enrolled in.
#[utoipa::path(
    get,
    path = "/v1/orgs/{org}/feature-flags/platform",
    tag = "Organizations",
    params(("org" = String, Path, description = "Organization public id")),
    responses(
        (status = 200, description = "Feature flag settings, platform view", body = OrgFeatureFlagsSettingsResponse),
        (status = 403, description = "Platform user access required", body = ErrorResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse),
    ),
    security(("bearerAuth" = []), ("cookieAuth" = []))
)]
pub async fn get_platform_feature_flag_settings(
    State(state): State<AppState>,
    Path(org_public_id): Path<String>,
    _platform_user: PlatformUser,
) -> ApiResult<OrgFeatureFlagsSettingsResponse> {
    let org_id = resolve_org_for_platform(&state, &org_public_id).await?;
    let org_enabled = state
        .db
        .list_org_feature_flags(org_id)
        .await
        .log_internal_error_json("list org feature flags")?;
    Ok(Json(OrgFeatureFlagsSettingsResponse {
        flags: build_all_feature_flag_settings(&state.system_feature_flags, &org_enabled),
    }))
}

/// PATCH /v1/orgs/{org}/feature-flags/platform — enrol an organization in a
/// platform-managed feature.
///
/// Platform users only, and limited to platform-managed flags: an operator
/// setting a tenant's own preferences would be acting as the tenant, which this
/// surface does not do. Omitted flags are unchanged, so enrolling one org in one
/// feature cannot disturb another setting.
#[utoipa::path(
    patch,
    path = "/v1/orgs/{org}/feature-flags/platform",
    tag = "Organizations",
    params(("org" = String, Path, description = "Organization public id")),
    request_body = UpdateOrgFeatureFlagsRequest,
    responses(
        (status = 200, description = "Updated effective flags", body = FeatureFlagMap),
        (status = 400, description = "Not a platform-managed flag", body = ErrorResponse),
        (status = 403, description = "Platform user access required", body = ErrorResponse),
        (status = 404, description = "Organization not found", body = ErrorResponse),
    ),
    security(("bearerAuth" = []), ("cookieAuth" = []))
)]
pub async fn update_platform_feature_flags(
    State(state): State<AppState>,
    Path(org_public_id): Path<String>,
    platform_user: PlatformUser,
    Json(req): Json<UpdateOrgFeatureFlagsRequest>,
) -> ApiResult<FeatureFlagMap> {
    let org_id = resolve_org_for_platform(&state, &org_public_id).await?;
    if let Err(msg) =
        validate_platform_feature_flag_updates(&state.system_feature_flags, &req.flags)
    {
        return Err(ErrorResponse::new(msg).into_response(axum::http::StatusCode::BAD_REQUEST));
    }

    let mut merged = state
        .db
        .list_org_feature_flags(org_id)
        .await
        .log_internal_error_json("list org feature flags")?;
    // `replace_org_feature_flags` writes the whole set, so merge first: an
    // operator enrolling an org must not silently clear the org's own opt-ins.
    merged.extend(req.flags.iter().map(|(name, on)| (name.clone(), *on)));

    state
        .db
        .replace_org_feature_flags(org_id, &merged)
        .await
        .log_internal_error_json("update platform feature flags")?;

    for (name, enabled) in &req.flags {
        tracing::info!(
            org = %org_public_id,
            flag = %name,
            enabled = *enabled,
            by = %platform_user.0.id,
            "platform feature flag updated for organization"
        );
    }

    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        &state.db,
        org_id,
        &state.system_feature_flags,
    )
    .await
    .log_internal_error_json("resolve org feature flags")?;
    Ok(Json(flags.to_map()))
}

/// Resolve an org for a platform user, who is not a member of it.
///
/// The tenant path resolves through membership; that check is exactly what a
/// cross-tenant operator does not pass, so this one goes by public id alone and
/// relies on the `PlatformUser` extractor for authorization.
async fn resolve_org_for_platform(
    state: &AppState,
    org_public_id: &str,
) -> Result<i64, (axum::http::StatusCode, Json<ErrorResponse>)> {
    if !validate_org_public_id(org_public_id) {
        return Err(ErrorResponse::not_found("Organization"));
    }
    let row = state
        .db
        .get_organization_by_public_id(org_public_id)
        .await
        .log_internal_error_json("get organization")?
        .ok_or_not_found_json("Organization")?;
    Ok(row.org_id)
}
