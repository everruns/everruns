// Workspace Memory CRUD HTTP routes.

use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::domains::common::Command;
pub use crate::domains::memory::types::{
    CreateMemoryRequest, ListMemoriesQuery, MemoryResponse, UpdateMemoryRequest,
};
use crate::domains::memory::{
    CreateMemory, DeleteMemory, GetMemory, ListMemories, ListMemoryEntries, ListMemoryStores,
    MemoryEntriesView, MemoryFileHistory, MemoryFileVersion, MemoryStoreView, SyncMemoryNow,
    UpdateMemoryCmd,
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};

use super::common::{ApiResult, ErrorResponse, ListResponse};

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route(
            "/v1/platform/memories",
            post(create_memory).get(list_memories),
        )
        .route(
            "/v1/platform/memories/{memory_id}",
            get(get_memory).patch(update_memory).delete(delete_memory),
        )
        .route(
            "/v1/platform/memories/{memory_id}/sync",
            post(sync_memory_now),
        )
        .route("/v1/platform/memory/stores", get(list_memory_stores))
        .route(
            "/v1/platform/memories/{memory_id}/entries",
            get(list_memory_entries),
        )
        .route(
            "/v1/platform/memories/{memory_id}/history",
            get(memory_file_history),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "Create a new workspace memory.",
    post,
    path = "/v1/platform/memories",
    request_body = CreateMemoryRequest,
    responses(
        (status = 201, description = "Memory created", body = MemoryResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Duplicate memory name", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn create_memory(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Json(req): Json<CreateMemoryRequest>,
) -> Result<(StatusCode, Json<MemoryResponse>), (StatusCode, Json<ErrorResponse>)> {
    let memory = CreateMemory::from(req).run(&state.ctx(&org)).await?;
    Ok((StatusCode::CREATED, Json(memory)))
}

#[utoipa::path(
    description = "List workspace memories.",
    get,
    path = "/v1/platform/memories",
    params(ListMemoriesQuery),
    responses(
        (status = 200, description = "List memories", body = ListResponse<MemoryResponse>)
    ),
    tag = "memory"
)]
pub async fn list_memories(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Query(query): Query<ListMemoriesQuery>,
) -> ApiResult<ListResponse<MemoryResponse>> {
    let memories = ListMemories::from(query).run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(memories)))
}

#[utoipa::path(
    description = "Get a workspace memory by ID.",
    get,
    path = "/v1/platform/memories/{memory_id}",
    params(("memory_id" = String, Path, description = "Memory ID")),
    responses(
        (status = 200, description = "Memory found", body = MemoryResponse),
        (status = 404, description = "Memory not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn get_memory(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(memory_id): Path<String>,
) -> ApiResult<MemoryResponse> {
    Ok(Json(GetMemory { memory_id }.run(&state.ctx(&org)).await?))
}

#[utoipa::path(
    description = "Update a workspace memory. Only provided fields are modified.",
    patch,
    path = "/v1/platform/memories/{memory_id}",
    params(("memory_id" = String, Path, description = "Memory ID")),
    request_body = UpdateMemoryRequest,
    responses(
        (status = 200, description = "Memory updated", body = MemoryResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Memory not found", body = ErrorResponse),
        (status = 409, description = "Duplicate memory name", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn update_memory(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(memory_id): Path<String>,
    Json(request): Json<UpdateMemoryRequest>,
) -> ApiResult<MemoryResponse> {
    Ok(Json(
        UpdateMemoryCmd { memory_id, request }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Trigger a synchronous sync of a memory now.",
    post,
    path = "/v1/platform/memories/{memory_id}/sync",
    params(("memory_id" = String, Path, description = "Memory ID")),
    responses(
        (status = 200, description = "Memory sync queued", body = MemoryResponse),
        (status = 400, description = "Invalid sync request", body = ErrorResponse),
        (status = 404, description = "Memory not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn sync_memory_now(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(memory_id): Path<String>,
) -> ApiResult<MemoryResponse> {
    Ok(Json(
        SyncMemoryNow { memory_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "Delete a workspace memory.",
    delete,
    path = "/v1/platform/memories/{memory_id}",
    params(("memory_id" = String, Path, description = "Memory ID")),
    responses(
        (status = 204, description = "Memory archived"),
        (status = 404, description = "Memory not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn delete_memory(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(memory_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteMemory { memory_id }.run(&state.ctx(&org)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    description = "List the memory stores agents use: the organization's, named memories, team and agent memories, and your own private memory.",
    get,
    path = "/v1/platform/memory/stores",
    responses(
        (status = 200, description = "Memory stores", body = ListResponse<MemoryStoreView>)
    ),
    tag = "memory"
)]
pub async fn list_memory_stores(
    org: ResolvedOrg,
    State(state): State<ApiState>,
) -> ApiResult<ListResponse<MemoryStoreView>> {
    let stores = ListMemoryStores {}.run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(stores)))
}

#[utoipa::path(
    description = "List a memory's typed entries, newest first, and any entry files that do not parse.",
    get,
    path = "/v1/platform/memories/{memory_id}/entries",
    params(("memory_id" = String, Path, description = "Memory ID")),
    responses(
        (status = 200, description = "Entries", body = MemoryEntriesView),
        (status = 400, description = "The memory holds no typed entries", body = ErrorResponse),
        (status = 404, description = "Memory not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn list_memory_entries(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(memory_id): Path<String>,
) -> ApiResult<MemoryEntriesView> {
    Ok(Json(
        ListMemoryEntries { memory_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

/// Query for a memory's file history.
#[derive(Debug, serde::Deserialize, utoipa::IntoParams)]
pub struct MemoryHistoryQuery {
    /// One file's path (`/decisions/merges.md`); omit for every file.
    #[serde(default)]
    pub path: Option<String>,
}

#[utoipa::path(
    description = "List the kept versions of a memory's files, newest first; every write and delete is kept.",
    get,
    path = "/v1/platform/memories/{memory_id}/history",
    params(("memory_id" = String, Path, description = "Memory ID"), MemoryHistoryQuery),
    responses(
        (status = 200, description = "Versions", body = ListResponse<MemoryFileVersion>),
        (status = 404, description = "Memory not found", body = ErrorResponse)
    ),
    tag = "memory"
)]
pub async fn memory_file_history(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(memory_id): Path<String>,
    Query(query): Query<MemoryHistoryQuery>,
) -> ApiResult<ListResponse<MemoryFileVersion>> {
    let versions = MemoryFileHistory {
        memory_id,
        path: query.path,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(ListResponse::new(versions)))
}
