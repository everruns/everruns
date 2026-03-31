// Audit log query API (TM-OBS-007, EVE-226)
//
// Policy-gated endpoint (AUDIT_LOG_VIEW). Supports domain/action filtering.
// Audit logs are append-only — no mutation endpoints exposed.

use crate::auth::middleware::{AuthState, ResolvedOrg};
use crate::services::AuditLogService;
use crate::storage::models::AuditLogQuery;
use axum::{Json, Router, extract::State, routing::get};
use chrono::{DateTime, Utc};
use everruns_core::Caller;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::{ApiPolicyResultExt, ApiResult, ListResponse, impl_auth_state};

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<AuditLogService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(service: Arc<AuditLogService>, auth: AuthState) -> Self {
        Self { service, auth }
    }
}

impl_auth_state!(AppState);

/// Audit log entry response
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogResponse {
    pub id: String,
    pub domain: String,
    pub action: String,
    pub actor_id: Option<String>,
    pub event_type: String,
    pub target_type: Option<String>,
    pub target_id: Option<String>,
    pub ip_address: Option<String>,
    pub metadata: serde_json::Value,
    pub created_at: DateTime<Utc>,
}

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

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/orgs/{org}/audit-logs", get(list_audit_logs))
        .with_state(state)
}

/// GET /v1/orgs/{org}/audit-logs - List audit logs (policy: AUDIT_LOG_VIEW)
async fn list_audit_logs(
    State(state): State<AppState>,
    org: ResolvedOrg,
    axum::extract::Query(query): axum::extract::Query<ListAuditLogsQuery>,
) -> ApiResult<ListResponse<AuditLogResponse>> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let caller = Caller::from(&org);

    let rows = state
        .service
        .list(
            &caller,
            AuditLogQuery {
                org_id: caller.org_id,
                limit,
                before: query.before,
                event_type_prefix: query.event_type.as_deref(),
                actor_id: query.actor_id,
                domain: query.domain.as_deref(),
                action: query.action.as_deref(),
            },
        )
        .await
        .map_policy_or_internal("list audit logs")?;

    let items: Vec<AuditLogResponse> = rows
        .into_iter()
        .map(|r| AuditLogResponse {
            id: r.id.to_string(),
            domain: r.domain,
            action: r.action,
            actor_id: r.actor_id.map(|a| a.to_string()),
            event_type: r.event_type,
            target_type: r.target_type,
            target_id: r.target_id,
            ip_address: r.ip_address,
            metadata: r.metadata,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(ListResponse::new(items)))
}
