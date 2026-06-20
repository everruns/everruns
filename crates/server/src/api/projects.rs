// Project CRUD HTTP routes — a thin adapter over the project domain commands
// (crate::domains::projects). The same commands surface through MCP
// discover/query/execute via the command registry, so REST and MCP share one
// implementation. Projects are scoped to the caller's active organization
// (resolved from auth via `ResolvedOrg`).

use crate::api::common::{ErrorResponse, ListResponse, impl_auth_state};
use crate::api::dispatch::{Dispatchable, impl_dispatchable};
use crate::auth::middleware::{AuthState, ResolvedOrg};
use crate::domains::common::Ctx;
use crate::domains::projects::types::ProjectResponse;
use crate::domains::projects::{
    CreateProject, DeleteProject, GetProject, ListProjects, UpdateProject,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use everruns_core::Caller;
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

/// App state for project routes.
#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);
impl_dispatchable!(AppState);

/// REST body for creating a project (no path param).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateProjectRequest {
    #[schema(example = "Support")]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

/// REST body for updating a project (id comes from the path).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateProjectRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

/// Build project routes.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/projects", get(list_projects).post(create_project))
        .route(
            "/v1/projects/{project}",
            get(get_project)
                .patch(update_project)
                .delete(delete_project),
        )
        .with_state(state)
}

/// GET /v1/projects - List projects in the active organization.
#[utoipa::path(
    get, path = "/v1/projects", tag = "projects",
    responses((status = 200, description = "Projects in the active org", body = ListResponse<ProjectResponse>))
)]
pub async fn list_projects(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> crate::api::common::ApiResult<ListResponse<ProjectResponse>> {
    state.dispatcher(&org).run(ListProjects {}).await
}

/// POST /v1/projects - Create a project.
#[utoipa::path(
    post, path = "/v1/projects", tag = "projects", request_body = CreateProjectRequest,
    responses(
        (status = 201, description = "Project created", body = ProjectResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Name taken or limit reached", body = ErrorResponse)
    )
)]
pub async fn create_project(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateProjectRequest>,
) -> Result<(StatusCode, Json<ProjectResponse>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created(CreateProject {
            name: req.name,
            description: req.description,
        })
        .await
}

/// GET /v1/projects/{project} - Get a project by public ID.
#[utoipa::path(
    get, path = "/v1/projects/{project}", tag = "projects",
    params(("project" = String, Path, description = "Project public ID")),
    responses(
        (status = 200, description = "Project", body = ProjectResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    )
)]
pub async fn get_project(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(project): Path<String>,
) -> crate::api::common::ApiResult<ProjectResponse> {
    state.dispatcher(&org).run(GetProject { project }).await
}

/// PATCH /v1/projects/{project} - Update a project's name/description.
#[utoipa::path(
    patch, path = "/v1/projects/{project}", tag = "projects",
    params(("project" = String, Path, description = "Project public ID")),
    request_body = UpdateProjectRequest,
    responses(
        (status = 200, description = "Updated", body = ProjectResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 409, description = "Name taken", body = ErrorResponse)
    )
)]
pub async fn update_project(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(project): Path<String>,
    Json(req): Json<UpdateProjectRequest>,
) -> crate::api::common::ApiResult<ProjectResponse> {
    state
        .dispatcher(&org)
        .run(UpdateProject {
            project,
            name: req.name,
            description: req.description,
        })
        .await
}

/// DELETE /v1/projects/{project} - Delete a non-default, empty project.
#[utoipa::path(
    delete, path = "/v1/projects/{project}", tag = "projects",
    params(("project" = String, Path, description = "Project public ID")),
    responses(
        (status = 204, description = "Deleted"),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 409, description = "Default or non-empty project", body = ErrorResponse)
    )
)]
pub async fn delete_project(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(project): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(DeleteProject { project })
        .await
}
