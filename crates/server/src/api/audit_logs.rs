// Audit log query API (TM-OBS-007, EVE-226)
//
// Policy-gated endpoint (AUDIT_LOG_VIEW). Supports domain/action filtering.
// Audit logs are append-only — no mutation endpoints exposed.
//
// Business logic lives in `crate::domains::audit_logs`; this file only
// binds HTTP params to the `ListAuditLogs` command. See knowledge/foundations/domains.md.

use crate::api::state::ApiState;
use crate::auth::middleware::ResolvedOrg;
use crate::domains::audit_logs::{AuditLogEntry, ListAuditLogs};
use crate::domains::common::Command;
use axum::{Json, Router, extract::State, routing::get};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use uuid::Uuid;

use super::common::{ApiResult, ListResponse};

/// Query parameters for listing audit logs
#[derive(Debug, Deserialize)]
pub struct ListAuditLogsQuery {
    /// Max entries to return (default 50, max 200)
    pub limit: Option<i64>,
    /// Cursor: return entries created before this timestamp
    pub before: Option<DateTime<Utc>>,
    /// Filter by event type prefix (e.g. "auth.login") — legacy
    pub event_type: Option<String>,
    /// Filter by actor UUID
    pub actor_id: Option<Uuid>,
    /// Filter by audit domain ("management" or "agent")
    pub domain: Option<String>,
    /// Filter by action string (e.g. "management.member.invited")
    pub action: Option<String>,
}

pub fn routes(state: ApiState) -> Router {
    Router::new()
        .route("/v1/orgs/{org}/audit-logs", get(list_audit_logs))
        .with_state(state)
}

/// GET /v1/orgs/{org}/audit-logs - List audit logs (policy: AUDIT_LOG_VIEW)
async fn list_audit_logs(
    State(state): State<ApiState>,
    org: ResolvedOrg,
    axum::extract::Query(query): axum::extract::Query<ListAuditLogsQuery>,
) -> ApiResult<ListResponse<AuditLogEntry>> {
    let items = ListAuditLogs {
        limit: query.limit,
        before: query.before,
        event_type: query.event_type,
        actor_id: query.actor_id,
        domain: query.domain,
        action: query.action,
    }
    .run(&state.ctx(&org))
    .await?;

    Ok(Json(ListResponse::new(items)))
}
