// Knowledge Index CRUD HTTP routes. See knowledge/runtime-resources/knowledge-indexes.md.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
pub use crate::domains::knowledge_indexes::types::{
    CreateKnowledgeIndexRequest, KnowledgeIndexDocumentResponse, KnowledgeIndexResponse,
    ListKnowledgeIndexesQuery, UpdateKnowledgeIndexRequest,
};
use crate::domains::knowledge_indexes::{
    CreateKnowledgeIndex, DeleteKnowledgeIndex, GetKnowledgeIndex, ListKnowledgeIndexDocuments,
    ListKnowledgeIndexes, SyncKnowledgeIndex, UpdateKnowledgeIndexCmd,
};
use crate::storage::StorageBackend;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use everruns_core::Caller;
use everruns_provider::driver_registry::DriverRegistry;
use std::sync::Arc;

use super::common::{ApiResult, ErrorResponse, ListResponse, impl_auth_state};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub driver_registry: Arc<DriverRegistry>,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        auth: AuthState,
        driver_registry: Arc<DriverRegistry>,
    ) -> Self {
        Self {
            db,
            auth,
            driver_registry,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
        .with_driver_registry(self.driver_registry.clone())
    }
}

impl_auth_state!(AppState);

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route(
            "/v1/knowledge-indexes",
            post(create_index).get(list_indexes),
        )
        .route(
            "/v1/knowledge-indexes/{index_id}",
            get(get_index).patch(update_index).delete(delete_index),
        )
        .route(
            "/v1/knowledge-indexes/{index_id}/documents",
            get(list_documents),
        )
        .route("/v1/knowledge-indexes/{index_id}/sync", post(sync_index))
        .with_state(state)
}

#[utoipa::path(
    description = "Create a new knowledge index.",
    post,
    path = "/v1/knowledge-indexes",
    request_body = CreateKnowledgeIndexRequest,
    responses(
        (status = 201, description = "Knowledge index created", body = KnowledgeIndexResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Duplicate knowledge index name", body = ErrorResponse)
    ),
    tag = "knowledge_indexes"
)]
pub async fn create_index(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateKnowledgeIndexRequest>,
) -> Result<(StatusCode, Json<KnowledgeIndexResponse>), (StatusCode, Json<ErrorResponse>)> {
    let index = CreateKnowledgeIndex::from(req)
        .run(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(index)))
}

#[utoipa::path(
    description = "List knowledge indexes.",
    get,
    path = "/v1/knowledge-indexes",
    params(ListKnowledgeIndexesQuery),
    responses(
        (status = 200, description = "List knowledge indexes", body = ListResponse<KnowledgeIndexResponse>)
    ),
    tag = "knowledge_indexes"
)]
pub async fn list_indexes(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListKnowledgeIndexesQuery>,
) -> ApiResult<ListResponse<KnowledgeIndexResponse>> {
    let indexes = ListKnowledgeIndexes::from(query)
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(ListResponse::new(indexes)))
}

#[utoipa::path(
    description = "Get a knowledge index by ID.",
    get,
    path = "/v1/knowledge-indexes/{index_id}",
    params(("index_id" = String, Path, description = "Knowledge index ID")),
    responses(
        (status = 200, description = "Knowledge index found", body = KnowledgeIndexResponse),
        (status = 404, description = "Knowledge index not found", body = ErrorResponse)
    ),
    tag = "knowledge_indexes"
)]
pub async fn get_index(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(index_id): Path<String>,
) -> ApiResult<KnowledgeIndexResponse> {
    Ok(Json(
        GetKnowledgeIndex { index_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "Update a knowledge index. Only provided fields are modified.",
    patch,
    path = "/v1/knowledge-indexes/{index_id}",
    params(("index_id" = String, Path, description = "Knowledge index ID")),
    request_body = UpdateKnowledgeIndexRequest,
    responses(
        (status = 200, description = "Knowledge index updated", body = KnowledgeIndexResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Knowledge index not found", body = ErrorResponse),
        (status = 409, description = "Duplicate knowledge index name", body = ErrorResponse)
    ),
    tag = "knowledge_indexes"
)]
pub async fn update_index(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(index_id): Path<String>,
    Json(request): Json<UpdateKnowledgeIndexRequest>,
) -> ApiResult<KnowledgeIndexResponse> {
    Ok(Json(
        UpdateKnowledgeIndexCmd { index_id, request }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Delete a knowledge index.",
    delete,
    path = "/v1/knowledge-indexes/{index_id}",
    params(("index_id" = String, Path, description = "Knowledge index ID")),
    responses(
        (status = 204, description = "Knowledge index archived"),
        (status = 404, description = "Knowledge index not found", body = ErrorResponse)
    ),
    tag = "knowledge_indexes"
)]
pub async fn delete_index(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(index_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteKnowledgeIndex { index_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    description = "Enqueue a manual sync of a knowledge index.",
    post,
    path = "/v1/knowledge-indexes/{index_id}/sync",
    params(("index_id" = String, Path, description = "Knowledge index ID")),
    responses(
        (status = 200, description = "Sync enqueued", body = KnowledgeIndexResponse),
        (status = 400, description = "Knowledge index is archived", body = ErrorResponse),
        (status = 404, description = "Knowledge index not found", body = ErrorResponse)
    ),
    tag = "knowledge_indexes"
)]
pub async fn sync_index(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(index_id): Path<String>,
) -> ApiResult<KnowledgeIndexResponse> {
    Ok(Json(
        SyncKnowledgeIndex { index_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "List documents inside a knowledge index.",
    get,
    path = "/v1/knowledge-indexes/{index_id}/documents",
    params(("index_id" = String, Path, description = "Knowledge index ID")),
    responses(
        (status = 200, description = "List documents", body = ListResponse<KnowledgeIndexDocumentResponse>),
        (status = 404, description = "Knowledge index not found", body = ErrorResponse)
    ),
    tag = "knowledge_indexes"
)]
pub async fn list_documents(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(index_id): Path<String>,
) -> ApiResult<ListResponse<KnowledgeIndexDocumentResponse>> {
    let documents = ListKnowledgeIndexDocuments { index_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(ListResponse::new(documents)))
}
