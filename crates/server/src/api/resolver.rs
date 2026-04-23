// Cross-org resource resolution endpoint.
//
// Lets the UI recover from a direct link into a resource owned by another
// org the caller is a member of. The API's 404-on-cross-org behaviour is
// preserved for every resource route — this endpoint is the only place that
// leaks an org_id across org boundaries, and only to orgs the caller
// already belongs to. See specs/multitenancy.md (Cross-Org Resource
// Resolution).

use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Query, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::common::impl_auth_state;
use crate::auth::middleware::{AuthState, AuthUser};
use crate::domains::org_resolver;
use crate::storage::StorageBackend;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl_auth_state!(AppState);

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResolveOrgQuery {
    /// Prefixed public ID of a top-level entity (e.g. a session, agent, app).
    #[schema(example = "session_019db85695a8785e87e8203109109343")]
    pub id: String,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct ResolveOrgResponse {
    /// Public ID of the organization that owns the resource.
    pub org_id: String,
    /// Organization name (for UX messaging).
    pub org_name: String,
}

/// GET /v1/resolve-org — resolve the owning org for a resource id.
///
/// Returns the owning organization only when it is one the authenticated
/// caller already belongs to. For every other case (unknown id, unknown
/// prefix, resource belongs to a non-member org) the endpoint returns 404 —
/// this preserves the existing org-enumeration guarantee documented in
/// specs/multitenancy.md.
#[utoipa::path(
    get,
    path = "/v1/resolve-org",
    params(
        ("id" = String, Query, description = "Prefixed public ID of the resource")
    ),
    responses(
        (status = 200, description = "Owning org resolved", body = ResolveOrgResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Unknown id, unknown prefix, or caller is not a member of the owning org")
    ),
    tag = "users"
)]
pub async fn resolve_org(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ResolveOrgQuery>,
) -> Result<Json<ResolveOrgResponse>, StatusCode> {
    let trimmed = query.id.trim();
    if trimmed.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let owning_org_id = org_resolver::resolve_resource_org(&state.db, trimmed)
        .await
        .map_err(|e| {
            tracing::error!("resolve-org lookup failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let Some(owning_org_id) = owning_org_id else {
        return Err(StatusCode::NOT_FOUND);
    };

    // Gate: only reveal the org if the caller is already a member. Without
    // this filter the endpoint would be a generic resource→org oracle.
    // THREAT[TM-TENANT-010]: membership gate — DO NOT remove.
    let is_member = state
        .db
        .is_organization_member(owning_org_id, auth.id)
        .await
        .map_err(|e| {
            tracing::error!("resolve-org membership lookup failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;
    if !is_member {
        return Err(StatusCode::NOT_FOUND);
    }

    // Only load the org record once membership is established.
    let Some(org) = state
        .db
        .get_organization(owning_org_id)
        .await
        .map_err(|e| {
            tracing::error!("resolve-org org lookup failed: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    else {
        // Theoretically unreachable (membership implies the org exists),
        // but treat a vanished row as 404 rather than 500.
        return Err(StatusCode::NOT_FOUND);
    };

    Ok(Json(ResolveOrgResponse {
        org_id: org.public_id,
        org_name: org.name,
    }))
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/resolve-org", get(resolve_org))
        .with_state(state)
}
