// Memory store CRUD HTTP routes.
// See specs/memory.md.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
pub use crate::domains::memory_stores::types::{
    CreateMemoryStoreRequest, ListMemoriesQuery, ListMemoriesResponse, MemoryResponse,
    MemoryStoreResponse, UpdateMemoryStoreRequest,
};
use crate::domains::memory_stores::{
    CreateMemoryStore, ForgetMemoryCmd, GetMemoryStore, ListMemoriesCmd, ListMemoryStores,
    UpdateMemoryStore,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
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
        .route(
            "/v1/memory-stores",
            post(create_memory_store).get(list_memory_stores),
        )
        .route(
            "/v1/memory-stores/{store_id}",
            get(get_memory_store).patch(update_memory_store),
        )
        .route("/v1/memory-stores/{store_id}/memories", get(list_memories))
        .route(
            "/v1/memory-stores/{store_id}/memories/{memory_id}",
            delete(forget_memory),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "List memory stores.",
    get,
    path = "/v1/memory-stores",
    responses(
        (status = 200, description = "List org memory stores",
            body = ListResponse<MemoryStoreResponse>)
    ),
    tag = "memory_stores"
)]
pub async fn list_memory_stores(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<ListResponse<MemoryStoreResponse>> {
    let stores = ListMemoryStores.run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(stores)))
}

#[utoipa::path(
    description = "Create a new memory store.",
    post,
    path = "/v1/memory-stores",
    request_body = CreateMemoryStoreRequest,
    responses(
        (status = 201, description = "Memory store created", body = MemoryStoreResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Duplicate name or default conflict", body = ErrorResponse)
    ),
    tag = "memory_stores"
)]
pub async fn create_memory_store(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateMemoryStoreRequest>,
) -> Result<(StatusCode, Json<MemoryStoreResponse>), (StatusCode, Json<ErrorResponse>)> {
    let store = CreateMemoryStore::from(req).run(&state.ctx(&org)).await?;
    Ok((StatusCode::CREATED, Json(store)))
}

#[utoipa::path(
    description = "Get a memory store by ID.",
    get,
    path = "/v1/memory-stores/{store_id}",
    params(("store_id" = String, Path, description = "Memory store ID")),
    responses(
        (status = 200, description = "Memory store", body = MemoryStoreResponse),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "memory_stores"
)]
pub async fn get_memory_store(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(store_id): Path<String>,
) -> ApiResult<MemoryStoreResponse> {
    Ok(Json(
        GetMemoryStore { store_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "Update a memory store. Only provided fields are modified.",
    patch,
    path = "/v1/memory-stores/{store_id}",
    params(("store_id" = String, Path, description = "Memory store ID")),
    request_body = UpdateMemoryStoreRequest,
    responses(
        (status = 200, description = "Memory store updated", body = MemoryStoreResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Not found", body = ErrorResponse),
        (status = 409, description = "Conflict (duplicate name or concurrent default reassignment)", body = ErrorResponse)
    ),
    tag = "memory_stores"
)]
pub async fn update_memory_store(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(store_id): Path<String>,
    Json(req): Json<UpdateMemoryStoreRequest>,
) -> ApiResult<MemoryStoreResponse> {
    Ok(Json(
        UpdateMemoryStore::from_request(store_id, req)
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "List memories.",
    get,
    path = "/v1/memory-stores/{store_id}/memories",
    params(
        ("store_id" = String, Path, description = "Memory store ID"),
        ListMemoriesQuery
    ),
    responses(
        (status = 200, description = "Memories", body = ListMemoriesResponse),
        (status = 404, description = "Store not found", body = ErrorResponse)
    ),
    tag = "memory_stores"
)]
pub async fn list_memories(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(store_id): Path<String>,
    Query(query): Query<ListMemoriesQuery>,
) -> ApiResult<ListMemoriesResponse> {
    Ok(Json(
        ListMemoriesCmd::from_query(store_id, query)
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Forget a memory.",
    delete,
    path = "/v1/memory-stores/{store_id}/memories/{memory_id}",
    params(
        ("store_id" = String, Path, description = "Memory store ID"),
        ("memory_id" = String, Path, description = "Memory ID")
    ),
    responses(
        (status = 204, description = "Memory deactivated"),
        (status = 404, description = "Not found", body = ErrorResponse)
    ),
    tag = "memory_stores"
)]
pub async fn forget_memory(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((store_id, memory_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    ForgetMemoryCmd {
        store_id,
        memory_id,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(StatusCode::NO_CONTENT)
}
