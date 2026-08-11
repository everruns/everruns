// Organization feature flag opt-in API
//
// Decision: Effective flags = deployment/system gate AND org opt-in (default off).
// Decision: GET settings exposes catalog for admin UI; PATCH requires OrgAdmin.

use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use everruns_platform::validate_org_public_id;
use everruns_platform::{FeatureFlagMap, FeatureFlags};

use crate::auth::middleware::{AuthState, OrgAdmin};
use crate::services::org_feature_flags::{
    OrgFeatureFlagsSettingsResponse, build_org_feature_flag_settings,
    validate_org_feature_flag_updates,
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
