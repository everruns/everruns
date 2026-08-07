// Audit log query API (TM-OBS-007, EVE-226)
//
// Policy-gated endpoint (AUDIT_LOG_VIEW). Supports domain/action filtering.
// Audit logs are append-only — no mutation endpoints exposed.
//
// Business logic lives in `crate::domains::audit_logs`; this file only
// binds HTTP params to the `ListAuditLogs` command. See knowledge/foundations/domains.md.

use crate::auth::middleware::{AuthState, ResolvedOrg};
use crate::domains::audit_logs::{AuditLogEntry, ListAuditLogs};
use crate::domains::common::{Command, Ctx};
use crate::services::CapabilityService;
use crate::storage::StorageBackend;
use axum::{Json, Router, extract::State, routing::get};
use chrono::{DateTime, Utc};
use everruns_core::Caller;
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

use super::common::{ApiResult, ListResponse, impl_auth_state};

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub capability_service: Arc<CapabilityService>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(
        db: Arc<StorageBackend>,
        capability_service: Arc<CapabilityService>,
        auth: AuthState,
    ) -> Self {
        Self {
            db,
            capability_service,
            auth,
        }
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::new(
            Caller::from(org),
            self.db.clone(),
            self.capability_service.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
    }
}

impl_auth_state!(AppState);

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
