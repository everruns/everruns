// Workspace CRUD HTTP routes and the workspace-scoped filesystem alias.
//
// /v1/workspaces/* — CRUD on the new Workspace entity.
// /v1/workspaces/{workspace_id}/fs/* — delegates to the existing session
// filesystem service. Since the migration ensures each session's
// `workspace_id` equals its `session.id` for the default 1:1 case, we can
// pass the resolved internal UUID directly to the session_files service.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::workspaces::types::workspace_response;
pub use crate::domains::workspaces::types::{
    CreateWorkspaceRequest, ListWorkspacesQuery, UpdateWorkspaceRequest, WorkspaceResponse,
};
use crate::domains::workspaces::{WORKSPACE_MANAGE, WORKSPACE_VIEW};
use crate::storage::StorageBackend;
use crate::storage::models::{CreateWorkspaceRow, UpdateWorkspace};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::{Caller, Policy};
use everruns_provider::typed_id::WorkspaceId;
use std::sync::Arc;

use super::common::{ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse, impl_auth_state};

fn enforce(
    state: &AppState,
    org: &ResolvedOrg,
    policy: &Policy,
    operation: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(org);
    policy
        .evaluate_with(state.auth.permission_resolver.as_ref(), &caller)
        .map_err(anyhow::Error::from)
        .map_policy_or_internal(operation)
}

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route(
            "/v1/workspaces/{workspace_id}",
            get(get_workspace)
                .patch(update_workspace)
                .delete(delete_workspace),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "List Workspaces.",
    get,
    path = "/v1/workspaces",
    params(ListWorkspacesQuery),
    responses(
        (status = 200, description = "Workspaces", body = ListResponse<WorkspaceResponse>),
    ),
    tag = "workspace"
)]
pub async fn list_workspaces(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListWorkspacesQuery>,
) -> ApiResult<ListResponse<WorkspaceResponse>> {
    enforce(&state, &org, &WORKSPACE_VIEW, "authorize list workspaces")?;
    let rows = state
        .db
        .list_workspaces(org.org_id, query.search.as_deref(), query.include_archived)
        .await
        .map_err(internal_error)?;
    Ok(Json(ListResponse::new(
        rows.into_iter().map(workspace_response).collect(),
    )))
}

#[utoipa::path(
    description = "Create a Workspace.",
    post,
    path = "/v1/workspaces",
    request_body = CreateWorkspaceRequest,
    responses(
        (status = 201, description = "Created", body = WorkspaceResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Name conflict", body = ErrorResponse),
    ),
    tag = "workspace"
)]
pub async fn create_workspace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateWorkspaceRequest>,
) -> Result<(StatusCode, Json<WorkspaceResponse>), (StatusCode, Json<ErrorResponse>)> {
    enforce(
        &state,
        &org,
        &WORKSPACE_MANAGE,
        "authorize create workspace",
    )?;
    let name = req.name.trim();
    if name.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("name cannot be empty")),
        ));
    }
    if name.chars().count() > 255 {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("name must be at most 255 characters")),
        ));
    }
    // Pin the internal primary key to the public id's uuid so
    // `id.hex == public_id suffix` holds universally (matching default
    // per-session workspaces). This keeps `WorkspaceId::from_uuid(id)` equal to
    // the public id, so callers — e.g. the `workspace_id` rendered on a session
    // response — round-trip correctly.
    let workspace_id = WorkspaceId::new();
    let public_id = workspace_id.to_string();
    let row = state
        .db
        .create_workspace(
            org.org_id,
            CreateWorkspaceRow {
                id: Some(workspace_id.uuid()),
                public_id,
                name: name.to_string(),
                description: req.description,
                owner_principal_id: None,
                resolved_owner_user_id: None,
            },
        )
        .await
        .map_err(|e| {
            if e.to_string().contains("already exists") {
                (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse::new("workspace name already exists")),
                )
            } else {
                internal_error(e)
            }
        })?;
    Ok((StatusCode::CREATED, Json(workspace_response(row))))
}

#[utoipa::path(
    description = "Get a Workspace by ID.",
    get,
    path = "/v1/workspaces/{workspace_id}",
    params(("workspace_id" = String, Path, description = "Workspace ID")),
    responses(
        (status = 200, description = "Workspace", body = WorkspaceResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "workspace"
)]
pub async fn get_workspace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
) -> ApiResult<WorkspaceResponse> {
    enforce(&state, &org, &WORKSPACE_VIEW, "authorize get workspace")?;
    let id: WorkspaceId = workspace_id.parse().map_err(|_| not_found())?;
    let row = state
        .db
        .get_workspace(org.org_id, id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    Ok(Json(workspace_response(row)))
}

#[utoipa::path(
    description = "Update a Workspace.",
    patch,
    path = "/v1/workspaces/{workspace_id}",
    params(("workspace_id" = String, Path, description = "Workspace ID")),
    request_body = UpdateWorkspaceRequest,
    responses(
        (status = 200, description = "Updated", body = WorkspaceResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 409, description = "Name conflict", body = ErrorResponse),
    ),
    tag = "workspace"
)]
pub async fn update_workspace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
    Json(req): Json<UpdateWorkspaceRequest>,
) -> ApiResult<WorkspaceResponse> {
    enforce(
        &state,
        &org,
        &WORKSPACE_MANAGE,
        "authorize update workspace",
    )?;
    if let Some(status) = req.status.as_deref()
        && !matches!(status, "active" | "archived" | "deleted")
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new(
                "Invalid workspace status (expected one of: active, archived, deleted)",
            )),
        ));
    }
    let id: WorkspaceId = workspace_id.parse().map_err(|_| not_found())?;
    let existing = state
        .db
        .get_workspace(org.org_id, id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    if let Some(ref name) = req.name
        && name.trim().is_empty()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse::new("name cannot be empty")),
        ));
    }
    let row = state
        .db
        .update_workspace(
            org.org_id,
            existing.id,
            UpdateWorkspace {
                name: req.name.map(|n| n.trim().to_string()),
                description: req.description,
                status: req.status,
            },
        )
        .await
        .map_err(|e| {
            if e.to_string().contains("already exists") {
                (
                    StatusCode::CONFLICT,
                    Json(ErrorResponse::new("workspace name already exists")),
                )
            } else {
                internal_error(e)
            }
        })?
        .ok_or_else(not_found)?;
    Ok(Json(workspace_response(row)))
}

#[utoipa::path(
    description = "Archive a Workspace.",
    delete,
    path = "/v1/workspaces/{workspace_id}",
    params(("workspace_id" = String, Path, description = "Workspace ID")),
    responses(
        (status = 204, description = "Archived"),
        (status = 404, description = "Not found", body = ErrorResponse),
    ),
    tag = "workspace"
)]
pub async fn delete_workspace(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(workspace_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    enforce(
        &state,
        &org,
        &WORKSPACE_MANAGE,
        "authorize delete workspace",
    )?;
    let id: WorkspaceId = workspace_id.parse().map_err(|_| not_found())?;
    let existing = state
        .db
        .get_workspace(org.org_id, id)
        .await
        .map_err(internal_error)?
        .ok_or_else(not_found)?;
    state
        .db
        .archive_workspace(org.org_id, existing.id)
        .await
        .map_err(internal_error)?;
    Ok(StatusCode::NO_CONTENT)
}

fn not_found() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse::new("Workspace not found")),
    )
}

fn internal_error<E: std::fmt::Display>(e: E) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "workspaces: internal error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse::new("internal server error")),
    )
}
