// Workspace CRUD HTTP routes and the workspace-scoped filesystem alias.
//
// /v1/workspaces/* — CRUD on the new Workspace entity.
// /v1/workspaces/{workspace_id}/fs/* — delegates to the existing session
// filesystem service. Since the migration ensures each session's
// `workspace_id` equals its `session.id` for the default 1:1 case, we can
// pass the resolved internal UUID directly to the session_files service.

use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::domains::change_history::rest::RestChange;
use crate::domains::change_history::{ChangeAction, EntityKind};
use crate::domains::workspaces::types::workspace_response;
pub use crate::domains::workspaces::types::{
    CreateWorkspaceRequest, ListWorkspacesQuery, UpdateWorkspaceRequest, WorkspaceResponse,
};
use crate::domains::workspaces::{WORKSPACE_MANAGE, WORKSPACE_VIEW};
use crate::storage::models::{CreateWorkspaceRow, UpdateWorkspace};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use everruns_contracts::typed_id::WorkspaceId;
use everruns_core::{Caller, Policy};

use super::common::{ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse};

fn enforce(
    state: &ApiState,
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

pub fn routes(state: ApiState) -> Router {
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
    State(state): State<ApiState>,
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
    State(state): State<ApiState>,
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
    let change = history(
        &state,
        &org,
        "create_workspace",
        ChangeAction::Created,
        None,
        &{
            let mut fields = vec!["name"];
            fields.extend(req.description.as_ref().map(|_| "description"));
            fields
        },
    )
    .await?;
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
    change.finish(&row.public_id).await;
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
    State(state): State<ApiState>,
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
    State(state): State<ApiState>,
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
    let fields: Vec<&str> = [
        req.name.as_ref().map(|_| "name"),
        req.description.as_ref().map(|_| "description"),
        req.status.as_ref().map(|_| "status"),
    ]
    .into_iter()
    .flatten()
    .collect();
    let change = history(
        &state,
        &org,
        "update_workspace",
        ChangeAction::Updated,
        Some(&existing.public_id),
        &fields,
    )
    .await?;
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
    change.finish(&row.public_id).await;
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
    State(state): State<ApiState>,
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
    let change = history(
        &state,
        &org,
        "delete_workspace",
        ChangeAction::Deleted,
        Some(&existing.public_id),
        &[],
    )
    .await?;
    state
        .db
        .archive_workspace(org.org_id, existing.id)
        .await
        .map_err(internal_error)?;
    change.finish(&existing.public_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// These routes write storage directly rather than through the workspace
/// commands, so they record their history entry themselves.
async fn history(
    state: &ApiState,
    org: &ResolvedOrg,
    operation: &'static str,
    action: ChangeAction,
    entity_ref: Option<&str>,
    fields: &[&str],
) -> Result<RestChange, (StatusCode, Json<ErrorResponse>)> {
    let caller = Caller::from(org);
    let kind = EntityKind::Workspace;
    Ok(RestChange::begin(
        state.db.clone(),
        caller,
        operation,
        kind,
        action,
        entity_ref,
        fields,
    )
    .await?)
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
