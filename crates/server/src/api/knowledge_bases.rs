// Knowledge Base CRUD HTTP routes. See specs/knowledge-bases.md.

use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::knowledge_bases::okf::ImportOkfBundle;
pub use crate::domains::knowledge_bases::okf::{
    ImportOkfBundleRequest, OkfFileInput, OkfImportSummary,
};
pub use crate::domains::knowledge_bases::types::{
    CreateKnowledgeBaseRequest, CreateKnowledgeEntryRequest, KnowledgeBaseResponse,
    KnowledgeEntryResponse, ListKnowledgeBasesQuery, ListKnowledgeEntriesQuery,
    UpdateKnowledgeBaseRequest, UpdateKnowledgeEntryRequest,
};
use crate::domains::knowledge_bases::{
    CreateKnowledgeBase, CreateKnowledgeEntry, DeleteKnowledgeBase, DeleteKnowledgeEntry,
    GetKnowledgeBase, GetKnowledgeEntry, ListKnowledgeBases, ListKnowledgeEntries,
    UpdateKnowledgeBaseCmd, UpdateKnowledgeEntryCmd,
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
        .route("/v1/knowledge-bases", post(create_kb).get(list_kbs))
        .route(
            "/v1/knowledge-bases/{kb_id}",
            get(get_kb).patch(update_kb).delete(delete_kb),
        )
        .route("/v1/knowledge-bases/{kb_id}/okf_import", post(import_okf))
        .route(
            "/v1/knowledge-bases/{kb_id}/entries",
            post(create_entry).get(list_entries),
        )
        .route(
            "/v1/knowledge-bases/{kb_id}/entries/{entry_id}",
            get(get_entry).patch(update_entry).delete(delete_entry),
        )
        .with_state(state)
}

#[utoipa::path(
    description = "Create a new knowledge base.",
    post,
    path = "/v1/knowledge-bases",
    request_body = CreateKnowledgeBaseRequest,
    responses(
        (status = 201, description = "Knowledge base created", body = KnowledgeBaseResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 409, description = "Duplicate knowledge base name", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn create_kb(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateKnowledgeBaseRequest>,
) -> Result<(StatusCode, Json<KnowledgeBaseResponse>), (StatusCode, Json<ErrorResponse>)> {
    let kb = CreateKnowledgeBase::from(req).run(&state.ctx(&org)).await?;
    Ok((StatusCode::CREATED, Json(kb)))
}

#[utoipa::path(
    description = "List knowledge bases.",
    get,
    path = "/v1/knowledge-bases",
    params(ListKnowledgeBasesQuery),
    responses(
        (status = 200, description = "List knowledge bases", body = ListResponse<KnowledgeBaseResponse>)
    ),
    tag = "knowledge_bases"
)]
pub async fn list_kbs(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListKnowledgeBasesQuery>,
) -> ApiResult<ListResponse<KnowledgeBaseResponse>> {
    let kbs = ListKnowledgeBases::from(query)
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(ListResponse::new(kbs)))
}

#[utoipa::path(
    description = "Get a knowledge base by ID.",
    get,
    path = "/v1/knowledge-bases/{kb_id}",
    params(("kb_id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 200, description = "Knowledge base found", body = KnowledgeBaseResponse),
        (status = 404, description = "Knowledge base not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn get_kb(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
) -> ApiResult<KnowledgeBaseResponse> {
    Ok(Json(
        GetKnowledgeBase { kb_id }.run(&state.ctx(&org)).await?,
    ))
}

#[utoipa::path(
    description = "Update a knowledge base. Only provided fields are modified.",
    patch,
    path = "/v1/knowledge-bases/{kb_id}",
    params(("kb_id" = String, Path, description = "Knowledge base ID")),
    request_body = UpdateKnowledgeBaseRequest,
    responses(
        (status = 200, description = "Knowledge base updated", body = KnowledgeBaseResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Knowledge base not found", body = ErrorResponse),
        (status = 409, description = "Duplicate knowledge base name", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn update_kb(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
    Json(request): Json<UpdateKnowledgeBaseRequest>,
) -> ApiResult<KnowledgeBaseResponse> {
    Ok(Json(
        UpdateKnowledgeBaseCmd { kb_id, request }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Delete a knowledge base.",
    delete,
    path = "/v1/knowledge-bases/{kb_id}",
    params(("kb_id" = String, Path, description = "Knowledge base ID")),
    responses(
        (status = 204, description = "Knowledge base archived"),
        (status = 404, description = "Knowledge base not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn delete_kb(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteKnowledgeBase { kb_id }.run(&state.ctx(&org)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    description = "Import an Open Knowledge Format (OKF) bundle into a knowledge base. \
Idempotent: re-importing converges entries without duplicates. See specs/okf-adoption.md.",
    post,
    path = "/v1/knowledge-bases/{kb_id}/okf_import",
    params(("kb_id" = String, Path, description = "Knowledge base ID")),
    request_body = ImportOkfBundleRequest,
    responses(
        (status = 200, description = "Import summary", body = OkfImportSummary),
        (status = 400, description = "Invalid bundle or input", body = ErrorResponse),
        (status = 404, description = "Knowledge base not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn import_okf(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
    Json(request): Json<ImportOkfBundleRequest>,
) -> ApiResult<OkfImportSummary> {
    Ok(Json(
        ImportOkfBundle::from_request(kb_id, request)
            .run(&state.ctx(&org))
            .await?,
    ))
}

// ------------- entries -------------

#[utoipa::path(
    description = "List knowledge base entries.",
    get,
    path = "/v1/knowledge-bases/{kb_id}/entries",
    params(
        ("kb_id" = String, Path, description = "Knowledge base ID"),
        ListKnowledgeEntriesQuery
    ),
    responses(
        (status = 200, description = "List entries", body = ListResponse<KnowledgeEntryResponse>),
        (status = 404, description = "Knowledge base not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn list_entries(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
    Query(query): Query<ListKnowledgeEntriesQuery>,
) -> ApiResult<ListResponse<KnowledgeEntryResponse>> {
    let entries = ListKnowledgeEntries::from_path_query(kb_id, query)
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(ListResponse::new(entries)))
}

#[utoipa::path(
    description = "Create a new knowledge base entry.",
    post,
    path = "/v1/knowledge-bases/{kb_id}/entries",
    params(("kb_id" = String, Path, description = "Knowledge base ID")),
    request_body = CreateKnowledgeEntryRequest,
    responses(
        (status = 201, description = "Entry created", body = KnowledgeEntryResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Knowledge base not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn create_entry(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(kb_id): Path<String>,
    Json(req): Json<CreateKnowledgeEntryRequest>,
) -> Result<(StatusCode, Json<KnowledgeEntryResponse>), (StatusCode, Json<ErrorResponse>)> {
    let entry = CreateKnowledgeEntry::from_request(kb_id, req)
        .run(&state.ctx(&org))
        .await?;
    Ok((StatusCode::CREATED, Json(entry)))
}

#[utoipa::path(
    description = "Get a knowledge base entry by ID.",
    get,
    path = "/v1/knowledge-bases/{kb_id}/entries/{entry_id}",
    params(
        ("kb_id" = String, Path, description = "Knowledge base ID"),
        ("entry_id" = String, Path, description = "Entry ID")
    ),
    responses(
        (status = 200, description = "Entry found", body = KnowledgeEntryResponse),
        (status = 404, description = "Entry not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn get_entry(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((kb_id, entry_id)): Path<(String, String)>,
) -> ApiResult<KnowledgeEntryResponse> {
    Ok(Json(
        GetKnowledgeEntry { kb_id, entry_id }
            .run(&state.ctx(&org))
            .await?,
    ))
}

#[utoipa::path(
    description = "Update a knowledge base entry. Only provided fields are modified.",
    patch,
    path = "/v1/knowledge-bases/{kb_id}/entries/{entry_id}",
    params(
        ("kb_id" = String, Path, description = "Knowledge base ID"),
        ("entry_id" = String, Path, description = "Entry ID")
    ),
    request_body = UpdateKnowledgeEntryRequest,
    responses(
        (status = 200, description = "Entry updated", body = KnowledgeEntryResponse),
        (status = 400, description = "Invalid input", body = ErrorResponse),
        (status = 404, description = "Entry not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn update_entry(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((kb_id, entry_id)): Path<(String, String)>,
    Json(request): Json<UpdateKnowledgeEntryRequest>,
) -> ApiResult<KnowledgeEntryResponse> {
    Ok(Json(
        UpdateKnowledgeEntryCmd {
            kb_id,
            entry_id,
            request,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}

#[utoipa::path(
    description = "Delete a knowledge base entry.",
    delete,
    path = "/v1/knowledge-bases/{kb_id}/entries/{entry_id}",
    params(
        ("kb_id" = String, Path, description = "Knowledge base ID"),
        ("entry_id" = String, Path, description = "Entry ID")
    ),
    responses(
        (status = 204, description = "Entry deleted"),
        (status = 404, description = "Entry not found", body = ErrorResponse)
    ),
    tag = "knowledge_bases"
)]
pub async fn delete_entry(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((kb_id, entry_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    DeleteKnowledgeEntry { kb_id, entry_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
