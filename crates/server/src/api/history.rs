// Entity history over REST.
//
//   GET  /v1/history                                   changes across the organization
//   GET  /v1/history/{entity_ref}                      changes to one entity
//   GET  /v1/history/{entity_ref}/revisions/{revision} the entity at one revision
//   GET  /v1/history/{entity_ref}/diff                 two revisions compared
//   POST /v1/history/{entity_ref}/restore              bring a revision back
//
// Thin routes over the history commands (`domains::change_history::commands`
// and `::revisions`), dispatched through the same pipeline
// as `/v1/commands`, so policy and output are identical on every surface.
// Mounted by `command_dispatch::routes`, which owns the shared command state.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use utoipa::IntoParams;
use uuid::Uuid;

use super::common::ErrorResponse;
use super::mcp_endpoint::{AppState, catalog, catalog_context};
use crate::auth::ResolvedOrg;
use crate::domains::change_history::commands::EntityChange;
use crate::domains::change_history::revisions::{EntityRevision, RestoreResult};
use crate::domains::change_history::snapshot::FieldDiff;

type HistoryResult = Result<Json<Vec<EntityChange>>, (StatusCode, Json<ErrorResponse>)>;

/// Filters for one entity's history.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EntityHistoryQuery {
    /// Entity kind, needed only for kinds whose ids have no prefix
    /// (`schedule`, `saved_report`, `check_rule`).
    pub kind: Option<String>,
    /// Only this action (`created`, `updated`, `deleted`, ...).
    pub action: Option<String>,
    /// Only changes older than this timestamp (page cursor).
    pub before: Option<DateTime<Utc>>,
    /// Max entries (default 50, max 200).
    pub limit: Option<i64>,
}

/// Filters for the organization's history.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OrgHistoryQuery {
    /// Only this entity kind.
    pub kind: Option<String>,
    /// Only this action.
    pub action: Option<String>,
    /// Only changes made as this user.
    pub actor_user_id: Option<Uuid>,
    /// Only changes an agent made, by the agent's public id.
    pub via_agent_id: Option<String>,
    /// Only changes at or after this timestamp.
    pub since: Option<DateTime<Utc>>,
    /// Only changes older than this timestamp (page cursor).
    pub before: Option<DateTime<Utc>>,
    /// Max entries (default 50, max 200).
    pub limit: Option<i64>,
}

type ApiResult<T> = Result<Json<T>, (StatusCode, Json<ErrorResponse>)>;

async fn dispatch<T: serde::de::DeserializeOwned>(
    org: &ResolvedOrg,
    state: &AppState,
    name: &str,
    params: serde_json::Value,
) -> ApiResult<T> {
    let output = catalog::dispatch_named(name, params, &catalog_context(org, state)).await?;
    let output = serde_json::from_value(output).map_err(|error| {
        crate::domains::common::CommandError::internal(anyhow::anyhow!(
            "{name} output has an unexpected shape: {error}"
        ))
    })?;
    Ok(Json(output))
}

/// Which entity, for ids without a prefix.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RevisionKindQuery {
    /// Entity kind, needed only for kinds whose ids have no prefix.
    pub kind: Option<String>,
}

/// The two revisions to compare.
#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DiffQuery {
    /// Entity kind, needed only for kinds whose ids have no prefix.
    pub kind: Option<String>,
    /// The older revision.
    pub from: i64,
    /// The newer revision; the latest when omitted.
    pub to: Option<i64>,
}

/// The revision to bring back.
#[derive(Debug, Deserialize, utoipa::ToSchema)]
pub struct RestoreRequest {
    pub revision: i64,
    /// Entity kind, needed only for kinds whose ids have no prefix.
    pub kind: Option<String>,
}

/// GET /v1/history/{entity_ref}/revisions/{revision} - The entity at one revision
#[utoipa::path(
    get,
    path = "/v1/history/{entity_ref}/revisions/{revision}",
    params(
        ("entity_ref" = String, Path, description = "The entity's public id"),
        ("revision" = i64, Path, description = "Revision number"),
        RevisionKindQuery,
    ),
    responses(
        (status = 200, description = "The entity as it stood after that change", body = EntityRevision),
        (status = 404, description = "No such revision", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn show_entity_revision(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((entity_ref, revision)): Path<(String, i64)>,
    Query(query): Query<RevisionKindQuery>,
) -> ApiResult<EntityRevision> {
    let params =
        serde_json::json!({ "entity_ref": entity_ref, "kind": query.kind, "revision": revision });
    dispatch(&org, &state, "show_entity_revision", params).await
}

/// GET /v1/history/{entity_ref}/diff - Two revisions compared field by field
#[utoipa::path(
    get,
    path = "/v1/history/{entity_ref}/diff",
    params(("entity_ref" = String, Path, description = "The entity's public id"), DiffQuery),
    responses(
        (status = 200, description = "Fields that differ", body = Vec<FieldDiff>),
        (status = 404, description = "No such revision", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn diff_entity_revisions(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(entity_ref): Path<String>,
    Query(query): Query<DiffQuery>,
) -> ApiResult<Vec<FieldDiff>> {
    let params = serde_json::json!({
        "entity_ref": entity_ref, "kind": query.kind, "from": query.from, "to": query.to,
    });
    dispatch(&org, &state, "diff_entity_revisions", params).await
}

/// POST /v1/history/{entity_ref}/restore - Bring a revision back as a new change
#[utoipa::path(
    post,
    path = "/v1/history/{entity_ref}/restore",
    params(("entity_ref" = String, Path, description = "The entity's public id")),
    request_body = RestoreRequest,
    responses(
        (status = 200, description = "Restored", body = RestoreResult),
        (status = 404, description = "No such revision, or the entity was deleted", body = ErrorResponse),
        (status = 409, description = "The entity's manager context changed since it was read", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn restore_entity_revision(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(entity_ref): Path<String>,
    Json(request): Json<RestoreRequest>,
) -> ApiResult<RestoreResult> {
    let params = serde_json::json!({
        "entity_ref": entity_ref, "kind": request.kind, "revision": request.revision,
    });
    dispatch(&org, &state, "restore_entity_revision", params).await
}

/// GET /v1/history/{entity_ref} - Changes to one entity
#[utoipa::path(
    get,
    path = "/v1/history/{entity_ref}",
    params(
        ("entity_ref" = String, Path, description = "The entity's public id, e.g. agent_01933b5a..."),
        EntityHistoryQuery,
    ),
    responses(
        (status = 200, description = "Recorded changes, newest first", body = Vec<EntityChange>),
        (status = 400, description = "Unknown kind or action, or a ref whose kind cannot be told", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn list_entity_history(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(entity_ref): Path<String>,
    Query(query): Query<EntityHistoryQuery>,
) -> HistoryResult {
    let params = serde_json::json!({
        "entity_ref": entity_ref,
        "kind": query.kind,
        "action": query.action,
        "before": query.before,
        "limit": query.limit,
    });
    dispatch(&org, &state, "list_entity_history", params).await
}

/// GET /v1/history - Changes across the organization
#[utoipa::path(
    get,
    path = "/v1/history",
    params(OrgHistoryQuery),
    responses(
        (status = 200, description = "Recorded changes, newest first", body = Vec<EntityChange>),
        (status = 400, description = "Unknown kind or action", body = ErrorResponse),
        (status = 403, description = "Forbidden", body = ErrorResponse),
    ),
    tag = "history"
)]
pub async fn list_org_history(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<OrgHistoryQuery>,
) -> HistoryResult {
    let params = serde_json::json!({
        "kind": query.kind,
        "action": query.action,
        "actor_user_id": query.actor_user_id,
        "via_agent_id": query.via_agent_id,
        "since": query.since,
        "before": query.before,
        "limit": query.limit,
    });
    dispatch(&org, &state, "list_org_history", params).await
}
