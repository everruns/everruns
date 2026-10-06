// Entity history over REST.
//
//   GET /v1/history                 changes across the organization
//   GET /v1/history/{entity_ref}    changes to one entity
//
// Thin routes over the `list_org_history` and `list_entity_history` commands
// (`domains::change_history::commands`), dispatched through the same pipeline
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

async fn dispatch(
    org: &ResolvedOrg,
    state: &AppState,
    name: &str,
    params: serde_json::Value,
) -> HistoryResult {
    let output = catalog::dispatch_named(name, params, &catalog_context(org, state)).await?;
    let entries = serde_json::from_value(output).map_err(|error| {
        crate::domains::common::CommandError::internal(anyhow::anyhow!(
            "history output is not a list of changes: {error}"
        ))
    })?;
    Ok(Json(entries))
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
