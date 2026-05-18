// Workspace Volume CRUD HTTP routes.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
pub use crate::domains::volumes::types::{
    CreateVolumeRequest, ListVolumesQuery, UpdateVolumeRequest, VolumeResponse,
};
use crate::domains::volumes::{
    CreateVolume, DeleteVolume, GetVolume, ListVolumes, SyncVolumeNow, UpdateVolumeCmd,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::Caller;
use std::sync::Arc;

use super::common::{ApiResult, ErrorResponse, ListResponse, impl_auth_state};

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

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/volumes", post(create_volume).get(list_volumes))
        .route(
            "/v1/volumes/{volume_id}",
            get(get_volume).patch(update_volume).delete(delete_volume),
        )
        .route("/v1/volumes/{volume_id}/sync", post(sync_volume_now))
        .with_state(state)
}

#[utoipa::path(
    description = "Create a new workspace volume.",
    post,
    path = "/v1/volumes",
    request_body = CreateVolumeRequest,
    responses(
        (status = 201, description = "Volume created", body = VolumeResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Duplicate volume name", body = ErrorResponse)
    ),
    tag = "volumes"
)]
pub async fn create_volume(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateVolumeRequest>,
) -> Result<(StatusCode, Json<VolumeResponse>), (StatusCode, Json<ErrorResponse>)> {
    let volume = CreateVolume::from(req).run(&state.ctx(&org)).await?;
    Ok((StatusCode::CREATED, Json(volume)))
}

#[utoipa::path(
    description = "List workspace volumes.",
    get,
    path = "/v1/volumes",
    params(ListVolumesQuery),
    responses(
        (status = 200, description = "List volumes", body = ListResponse<VolumeResponse>)
    ),
    tag = "volumes"
)]
pub async fn list_volumes(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListVolumesQuery>,
) -> ApiResult<ListResponse<VolumeResponse>> {
    let volumes = ListVolumes::from(query).run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(volumes)))
}

#[utoipa::path(
    description = "Get a workspace volume by ID.",
    get,
    path = "/v1/volumes/{volume_id}",
    params(("volume_id" = String, Path, description = "Volume ID")),
    responses(
        (status = 200, description = "Volume found", body = VolumeResponse),
        (status = 404, description = "Volume not found", body = ErrorResponse)
    ),
    tag = "volumes"
)]
pub async fn get_volume(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(volume_id): Path<String>,
) -> ApiResult<VolumeResponse> {
    Ok(Json(GetVolume { volume_id }.run(&state.ctx(&org)).await?))
}

#[utoipa::path(
    description = "Update a workspace volume. Only provided fields are modified.",
    patch,
    path = "/v1/volumes/{volume_id}",
    params(("volume_id" = String, Path, description = "Volume ID")),
    request_body = UpdateVolumeRequest,
    responses(
        (status = 200, description = "Volume updated", body = VolumeResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Volume not found", body = ErrorResponse),
        (status = 409, description = "Duplicate volume name", body = ErrorResponse)
    ),
    tag = "volumes"
)]
pub async fn update_volume(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(volume_id): Path<String>,
    Json(request): Json<UpdateVolumeRequest>,
) -> ApiResult<VolumeResponse> {
    Ok(Json(
        UpdateVolumeCmd { volume_id, request }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Trigger a synchronous sync of a volume now.",
    post,
    path = "/v1/volumes/{volume_id}/sync",
    params(("volume_id" = String, Path, description = "Volume ID")),
    responses(
        (status = 200, description = "Volume sync queued", body = VolumeResponse),
        (status = 400, description = "Invalid sync request", body = ErrorResponse),
        (status = 404, description = "Volume not found", body = ErrorResponse)
    ),
    tag = "volumes"
)]
pub async fn sync_volume_now(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(volume_id): Path<String>,
) -> ApiResult<VolumeResponse> {
    Ok(Json(
        SyncVolumeNow { volume_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "Delete a workspace volume.",
    delete,
    path = "/v1/volumes/{volume_id}",
    params(("volume_id" = String, Path, description = "Volume ID")),
    responses(
        (status = 204, description = "Volume archived"),
        (status = 404, description = "Volume not found", body = ErrorResponse)
    ),
    tag = "volumes"
)]
pub async fn delete_volume(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(volume_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteVolume { volume_id }.run(&state.ctx(&org)).await?;
    Ok(StatusCode::NO_CONTENT)
}
