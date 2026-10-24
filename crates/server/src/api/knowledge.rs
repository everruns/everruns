//! HTTP adapter for unified Knowledge. Commands retain policy and CLI parity.
use crate::{
    api::{
        common::{ApiResult, ErrorResponse},
        state::ApiState,
    },
    auth::ResolvedOrg,
    domains::{
        common::{Command, CommandError, Ctx},
        knowledge::{KnowledgeService, commands::*, types::*},
    },
};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{StatusCode, header},
    response::Response,
    routing::{get, post, put},
};
use serde::Deserialize;
use serde_json::Value;
fn ctx(state: &ApiState, org: &ResolvedOrg) -> Result<Ctx, CommandError> {
    if !org.feature_flags.knowledge {
        return Err(CommandError::feature_not_enabled("knowledge"));
    }
    Ok(state.ctx(org))
}
pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/knowledge", get(list_knowledge).post(create_knowledge))
        .route("/v1/knowledge/search", post(search_knowledge_collections))
        .route(
            "/v1/knowledge-indexers",
            get(list_knowledge_indexers).post(create_knowledge_indexer),
        )
        .route(
            "/v1/knowledge/{collection_id}",
            get(get_knowledge)
                .patch(update_knowledge)
                .delete(archive_knowledge),
        )
        .route(
            "/v1/knowledge/{collection_id}/files",
            get(list_knowledge_files)
                .put(write_knowledge_file)
                .patch(update_knowledge_file_metadata)
                .delete(delete_knowledge_file),
        )
        .route(
            "/v1/knowledge/{collection_id}/files/content",
            get(get_knowledge_file_content),
        )
        .route(
            "/v1/knowledge/{collection_id}/upload",
            put(upload_knowledge_file).layer(DefaultBodyLimit::disable()),
        )
        .route(
            "/v1/knowledge/{collection_id}/sources",
            post(add_knowledge_source),
        )
        .route("/v1/knowledge/{collection_id}/sync", post(sync_knowledge))
        .route(
            "/v1/knowledge/{collection_id}/okf",
            get(validate_knowledge_okf),
        )
        .with_state(state)
}
#[utoipa::path(operation_id="list_knowledge",get,path="/v1/knowledge",params(PageQuery),responses((status=200,description="Knowledge collections",body=Value)),tag="knowledge")]
pub async fn list_knowledge(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Value> {
    Ok(Json(
        ListKnowledge {
            after: q.after,
            limit: q.limit,
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}
#[utoipa::path(operation_id="create_knowledge",post,path="/v1/knowledge",request_body=CollectionInput,responses((status=200,description="Created collection",body=Value)),tag="knowledge")]
pub async fn create_knowledge(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Json(input): Json<CollectionInput>,
) -> ApiResult<Value> {
    Ok(Json(CreateKnowledge { input }.run(&ctx(&s, &org)?).await?))
}
#[utoipa::path(operation_id="get_knowledge",get,path="/v1/knowledge/{collection_id}",params(("collection_id"=String,Path,description="Knowledge collection")),responses((status=200,description="Collection and processing status",body=Value)),tag="knowledge")]
pub async fn get_knowledge(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
) -> ApiResult<Value> {
    Ok(Json(
        GetKnowledge { collection_id }.run(&ctx(&s, &org)?).await?,
    ))
}
#[utoipa::path(operation_id="update_knowledge",patch,path="/v1/knowledge/{collection_id}",params(("collection_id"=String,Path,description="Knowledge collection")),request_body=CollectionInput,responses((status=200,description="Updated collection",body=Value)),tag="knowledge")]
pub async fn update_knowledge(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
    Json(input): Json<CollectionInput>,
) -> ApiResult<Value> {
    Ok(Json(
        UpdateKnowledge {
            collection_id,
            input,
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}
#[utoipa::path(operation_id="archive_knowledge",delete,path="/v1/knowledge/{collection_id}",params(("collection_id"=String,Path,description="Knowledge collection")),responses((status=200,description="Archived collection",body=Value)),tag="knowledge")]
pub async fn archive_knowledge(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
) -> ApiResult<Value> {
    Ok(Json(
        ArchiveKnowledge { collection_id }
            .run(&ctx(&s, &org)?)
            .await?,
    ))
}
#[utoipa::path(operation_id="list_knowledge_files",get,path="/v1/knowledge/{collection_id}/files",params(("collection_id"=String,Path,description="Knowledge collection"),PageQuery),responses((status=200,description="File catalog page",body=Value)),tag="knowledge")]
pub async fn list_knowledge_files(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Value> {
    Ok(Json(
        ListKnowledgeFiles {
            collection_id,
            query,
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}
#[utoipa::path(operation_id="write_knowledge_file",put,path="/v1/knowledge/{collection_id}/files",params(("collection_id"=String,Path,description="Knowledge collection")),request_body=FileInput,responses((status=200,description="Stored immutable file version",body=Value)),tag="knowledge")]
pub async fn write_knowledge_file(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
    Json(input): Json<FileInput>,
) -> ApiResult<Value> {
    Ok(Json(
        WriteKnowledgeFile {
            collection_id,
            input,
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}
#[utoipa::path(operation_id="delete_knowledge_file",delete,path="/v1/knowledge/{collection_id}/files",params(("collection_id"=String,Path,description="Knowledge collection"),("path"=String,Query,description="File path")),responses((status=200,description="Deleted file",body=Value)),tag="knowledge")]
pub async fn delete_knowledge_file(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
    Query(q): Query<PageQuery>,
) -> ApiResult<Value> {
    Ok(Json(
        DeleteKnowledgeFile {
            collection_id,
            path: q.path.unwrap_or_default(),
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}
#[utoipa::path(operation_id="add_knowledge_source",post,path="/v1/knowledge/{collection_id}/sources",params(("collection_id"=String,Path,description="Knowledge collection")),request_body=SourceInput,responses((status=200,description="Connected source",body=Value)),tag="knowledge")]
pub async fn add_knowledge_source(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
    Json(input): Json<SourceInput>,
) -> ApiResult<Value> {
    Ok(Json(
        AddKnowledgeSource {
            collection_id,
            input,
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}
#[utoipa::path(operation_id="sync_knowledge",post,path="/v1/knowledge/{collection_id}/sync",params(("collection_id"=String,Path,description="Knowledge collection")),responses((status=200,description="Refresh queued",body=Value)),tag="knowledge")]
pub async fn sync_knowledge(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
) -> ApiResult<Value> {
    Ok(Json(
        SyncKnowledge { collection_id }.run(&ctx(&s, &org)?).await?,
    ))
}
#[utoipa::path(operation_id="search_knowledge_collections",post,path="/v1/knowledge/search",request_body=everruns_contracts::knowledge::KnowledgeQuery,responses((status=200,description="Versioned search results",body=Value)),tag="knowledge")]
pub async fn search_knowledge_collections(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Json(query): Json<everruns_contracts::knowledge::KnowledgeQuery>,
) -> ApiResult<Value> {
    Ok(Json(
        SearchKnowledgeCollections { query }
            .run(&ctx(&s, &org)?)
            .await?,
    ))
}
#[utoipa::path(operation_id="list_knowledge_indexers",get,path="/v1/knowledge-indexers",responses((status=200,description="Organization search accounts",body=Value)),tag="knowledge")]
pub async fn list_knowledge_indexers(
    org: ResolvedOrg,
    State(s): State<ApiState>,
) -> ApiResult<Value> {
    Ok(Json(ListKnowledgeIndexers {}.run(&ctx(&s, &org)?).await?))
}
#[utoipa::path(operation_id="create_knowledge_indexer",post,path="/v1/knowledge-indexers",request_body=IndexerInput,responses((status=200,description="Configured search account",body=Value)),tag="knowledge")]
pub async fn create_knowledge_indexer(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Json(input): Json<IndexerInput>,
) -> ApiResult<Value> {
    Ok(Json(
        CreateKnowledgeIndexer { input }
            .run(&ctx(&s, &org)?)
            .await?,
    ))
}
#[utoipa::path(operation_id="get_knowledge_file_content",get,path="/v1/knowledge/{collection_id}/files/content",params(("collection_id"=String,Path,description="Knowledge collection"),PageQuery),responses((status=200,description="Bounded original or extracted bytes",body=String)),tag="knowledge")]
pub async fn get_knowledge_file_content(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(key): Path<String>,
    Query(q): Query<PageQuery>,
) -> Result<Response, (StatusCode, Json<ErrorResponse>)> {
    let c = ctx(&s, &org)?;
    crate::domains::knowledge::VIEW
        .evaluate_with(c.permission_resolver.as_ref(), &c.caller)
        .map_err(|_| CommandError::forbidden("Knowledge permission required"))?;
    let service = KnowledgeService::from_ctx(&c);
    let collection = service.collection(c.org_id(), &key).await?;
    let (v, bytes) = service
        .read_management_bytes(&collection, q.path.as_deref().unwrap_or("/"), &q)
        .await?;
    let offset = q.offset.unwrap_or(0);
    let size = if q.extracted.unwrap_or(false) {
        v.text_bytes as u64
    } else {
        v.size_bytes as u64
    };
    if q.length.is_none() && q.offset.is_none() {
        let version_id = v.id;
        let extracted = q.extracted.unwrap_or(false);
        let path = q.path.unwrap_or_default();
        let state = (service, collection, path, version_id, extracted, 0u64, size);
        let stream = futures::stream::try_unfold(
            state,
            |(service, c, path, version, extracted, offset, size)| async move {
                if offset >= size {
                    return Ok::<_, std::io::Error>(None);
                }
                let live = service
                    .collection(c.org_id, &c.public_id())
                    .await
                    .map_err(|_| std::io::Error::other("Knowledge access revoked"))?;
                let (_, part) = service
                    .read_management_bytes(
                        &live,
                        &path,
                        &PageQuery {
                            version: Some(version),
                            extracted: Some(extracted),
                            offset: Some(offset),
                            length: Some(crate::domains::knowledge::content::READ_LIMIT as u32),
                            ..Default::default()
                        },
                    )
                    .await
                    .map_err(|_| std::io::Error::other("Knowledge download interrupted"))?;
                if part.is_empty() {
                    return Err(std::io::Error::other("Knowledge content incomplete"));
                }
                let next = offset + part.len() as u64;
                Ok(Some((
                    axum::body::Bytes::from(part),
                    (service, c, path, version, extracted, next, size),
                )))
            },
        );
        return Response::builder()
            .header(
                header::CONTENT_TYPE,
                if extracted {
                    "text/plain; charset=utf-8"
                } else {
                    &v.mime_type
                },
            )
            .header(header::CONTENT_LENGTH, size.to_string())
            .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
            .header(header::CONTENT_DISPOSITION, "attachment")
            .header(header::CACHE_CONTROL, "private, no-store")
            .header("Content-Security-Policy", "sandbox; default-src 'none'")
            .header("X-Knowledge-Version", v.id.to_string())
            .body(Body::from_stream(stream))
            .map_err(|e| CommandError::internal(e.into()).into());
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            if q.extracted.unwrap_or(false) {
                "text/plain; charset=utf-8"
            } else {
                &v.mime_type
            },
        )
        .header("X-Knowledge-Version", v.id.to_string())
        .header("X-Knowledge-Size", size.to_string())
        .header("X-Next-Offset", (offset + bytes.len() as u64).to_string())
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CONTENT_DISPOSITION, "attachment")
        .header(header::CACHE_CONTROL, "private, no-store")
        .header("Content-Security-Policy", "sandbox; default-src 'none'")
        .body(Body::from(bytes))
        .map_err(|e| CommandError::internal(e.into()).into())
}
#[derive(Deserialize, utoipa::IntoParams)]
pub struct UploadQuery {
    pub path: String,
    pub mime_type: Option<String>,
}
#[utoipa::path(operation_id="upload_knowledge_file",put,path="/v1/knowledge/{collection_id}/upload",params(("collection_id"=String,Path,description="Knowledge collection"),UploadQuery),request_body(content=String,content_type="application/octet-stream"),responses((status=200,description="Stored streamed original",body=Value)),tag="knowledge")]
pub async fn upload_knowledge_file(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(key): Path<String>,
    Query(q): Query<UploadQuery>,
    body: Body,
) -> ApiResult<Value> {
    let c = ctx(&s, &org)?;
    crate::domains::knowledge::MANAGE
        .evaluate_with(c.permission_resolver.as_ref(), &c.caller)
        .map_err(|_| CommandError::forbidden("Knowledge management permission required"))?;
    let service = KnowledgeService::from_ctx(&c);
    let collection = service.collection(c.org_id(), &key).await?;
    let source = crate::domains::knowledge::commands::managed(&service, &collection).await?;
    let uri = format!(
        "knowledge://{}/{}",
        collection.public_id(),
        q.path.trim_start_matches('/')
    );
    Ok(Json(
        service
            .ingest(
                &collection,
                &source,
                FileInput {
                    path: q.path,
                    mime_type: q.mime_type,
                    ..Default::default()
                },
                body.into_data_stream(),
                uri,
                None,
                None,
                None,
            )
            .await?,
    ))
}

#[utoipa::path(operation_id="update_knowledge_file_metadata",patch,path="/v1/knowledge/{collection_id}/files",params(("collection_id"=String,Path,description="Knowledge collection")),request_body=FileMetadataInput,responses((status=200,description="Updated file metadata and policy",body=Value)),tag="knowledge")]
pub async fn update_knowledge_file_metadata(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
    Json(input): Json<FileMetadataInput>,
) -> ApiResult<Value> {
    Ok(Json(
        UpdateKnowledgeFileMetadata {
            collection_id,
            input,
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}

#[utoipa::path(operation_id="validate_knowledge_okf",get,path="/v1/knowledge/{collection_id}/okf",params(("collection_id"=String,Path,description="Knowledge collection"),PageQuery),responses((status=200,description="OKF metadata validation page",body=Value)),tag="knowledge")]
pub async fn validate_knowledge_okf(
    org: ResolvedOrg,
    State(s): State<ApiState>,
    Path(collection_id): Path<String>,
    Query(query): Query<PageQuery>,
) -> ApiResult<Value> {
    Ok(Json(
        ValidateKnowledgeOkf {
            collection_id,
            query,
        }
        .run(&ctx(&s, &org)?)
        .await?,
    ))
}
