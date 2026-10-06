// Manager context over REST.
//
//   GET    /v1/context/{entity_ref}          read an entity's manager context
//   PUT    /v1/context/{entity_ref}          replace it
//   POST   /v1/context/{entity_ref}/append   add a paragraph
//   DELETE /v1/context/{entity_ref}          empty it
//
// Thin routes over the `*_manager_context` commands
// (`domains::change_history::context`), dispatched through the same pipeline
// as `/v1/commands`. The surface is pinned to `api` so the reason header the
// HTTP layer captured is recorded as a REST change. Mounted by
// `command_dispatch::routes`.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use super::common::ErrorResponse;
use super::mcp_endpoint::{AppState, catalog, catalog_context};
use crate::auth::ResolvedOrg;
use crate::domains::change_history::context::ManagerContext;
use crate::domains::change_history::{ChangeIntent, ChangeSurface};

type ContextResult = Result<Json<ManagerContext>, (StatusCode, Json<ErrorResponse>)>;

/// Which entity, for ids whose kind cannot be told from their prefix.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ContextKindQuery {
    /// Entity kind, needed only for ids without a prefix (`schedule`,
    /// `saved_report`, `check_rule`).
    pub kind: Option<String>,
    /// For DELETE: the revision being cleared; refused when stale.
    pub expected_revision: Option<i64>,
}

/// Replace an entity's manager context.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetManagerContextRequest {
    /// The whole document, markdown, at most 16 KiB.
    pub content: String,
    /// The revision this edit was based on (0 when none exists yet); refused
    /// with `manager_context_changed` when the stored one differs.
    pub expected_revision: Option<i64>,
}

/// Add a paragraph to an entity's manager context.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AppendManagerContextRequest {
    /// The paragraph to add, markdown.
    pub text: String,
}

async fn dispatch(
    org: &ResolvedOrg,
    state: &AppState,
    name: &str,
    params: serde_json::Value,
) -> ContextResult {
    let mut context = catalog_context(org, state);
    context.domain_ctx = context
        .domain_ctx
        .with_change_intent(ChangeIntent::on(ChangeSurface::Api));
    let output = catalog::dispatch_named(name, params, &context).await?;
    let context = serde_json::from_value(output).map_err(|error| {
        crate::domains::common::CommandError::internal(anyhow::anyhow!(
            "manager context output has the wrong shape: {error}"
        ))
    })?;
    Ok(Json(context))
}

/// GET /v1/context/{entity_ref} - Read an entity's manager context
#[utoipa::path(
    get,
    path = "/v1/context/{entity_ref}",
    params(
        ("entity_ref" = String, Path, description = "The entity's public id, e.g. agent_01933b5a..."),
        ContextKindQuery,
    ),
    responses(
        (status = 200, description = "The manager context; empty content and revision 0 when none was written", body = ManagerContext),
        (status = 400, description = "Unknown kind, or a kind without manager context", body = ErrorResponse),
        (status = 403, description = "The caller does not manage this kind, or is the entity's own session", body = ErrorResponse),
        (status = 404, description = "Entity not found", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn get_manager_context(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(entity_ref): Path<String>,
    Query(query): Query<ContextKindQuery>,
) -> ContextResult {
    let params = serde_json::json!({ "entity_ref": entity_ref, "kind": query.kind });
    dispatch(&org, &state, "get_manager_context", params).await
}

/// PUT /v1/context/{entity_ref} - Replace an entity's manager context
#[utoipa::path(
    put,
    path = "/v1/context/{entity_ref}",
    params(
        ("entity_ref" = String, Path, description = "The entity's public id"),
        ContextKindQuery,
    ),
    request_body = SetManagerContextRequest,
    responses(
        (status = 200, description = "The new manager context", body = ManagerContext),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Entity not found", body = ErrorResponse),
        (status = 409, description = "Stale expected_revision (manager_context_changed)", body = ErrorResponse),
        (status = 422, description = "Larger than 16 KiB", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn set_manager_context(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(entity_ref): Path<String>,
    Query(query): Query<ContextKindQuery>,
    Json(body): Json<SetManagerContextRequest>,
) -> ContextResult {
    let params = serde_json::json!({
        "entity_ref": entity_ref,
        "kind": query.kind,
        "content": body.content,
        "expected_revision": body.expected_revision,
    });
    dispatch(&org, &state, "set_manager_context", params).await
}

/// POST /v1/context/{entity_ref}/append - Add a paragraph to an entity's manager context
#[utoipa::path(
    post,
    path = "/v1/context/{entity_ref}/append",
    params(
        ("entity_ref" = String, Path, description = "The entity's public id"),
        ContextKindQuery,
    ),
    request_body = AppendManagerContextRequest,
    responses(
        (status = 200, description = "The new manager context", body = ManagerContext),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Entity not found", body = ErrorResponse),
        (status = 422, description = "Larger than 16 KiB", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn append_manager_context(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(entity_ref): Path<String>,
    Query(query): Query<ContextKindQuery>,
    Json(body): Json<AppendManagerContextRequest>,
) -> ContextResult {
    let params = serde_json::json!({
        "entity_ref": entity_ref,
        "kind": query.kind,
        "text": body.text,
    });
    dispatch(&org, &state, "append_manager_context", params).await
}

/// DELETE /v1/context/{entity_ref} - Empty an entity's manager context
#[utoipa::path(
    delete,
    path = "/v1/context/{entity_ref}",
    params(
        ("entity_ref" = String, Path, description = "The entity's public id"),
        ContextKindQuery,
    ),
    responses(
        (status = 200, description = "The emptied manager context, with its new revision", body = ManagerContext),
        (status = 403, description = "Forbidden", body = ErrorResponse),
        (status = 404, description = "Entity not found", body = ErrorResponse),
        (status = 409, description = "Stale expected_revision (manager_context_changed)", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn clear_manager_context(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(entity_ref): Path<String>,
    Query(query): Query<ContextKindQuery>,
) -> ContextResult {
    let params = serde_json::json!({
        "entity_ref": entity_ref,
        "kind": query.kind,
        "expected_revision": query.expected_revision,
    });
    dispatch(&org, &state, "clear_manager_context", params).await
}
