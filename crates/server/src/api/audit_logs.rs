// Audit log query API (TM-OBS-007, EVE-226)
//
// Policy-gated endpoint (AUDIT_LOG_VIEW). Supports domain/action filtering.
// Audit logs are append-only — no mutation endpoints exposed.
//
// Business logic lives in `crate::domains::audit_logs`; this file only
// mounts the `ListAuditLogs` command on the generic command handler.
// See knowledge/foundations/domains.md.

use crate::api::command_http::CommandRouterExt;
use crate::api::state::ApiState;
use crate::domains::audit_logs::ListAuditLogs;
use axum::Router;

pub fn routes(state: ApiState) -> Router {
    Router::new().command::<ListAuditLogs>().with_state(state)
}
