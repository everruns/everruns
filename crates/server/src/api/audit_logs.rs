// Audit log query API (TM-OBS-007)
//
// Admin-only endpoint. Audit logs are written by auth routes via auth::audit::emit().

use crate::auth::middleware::{AuthState, OrgAdmin};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{Json, Router, extract::State, routing::get};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;
use uuid::Uuid;

use super::common::ListResponse;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self { db, auth }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Audit log entry response
#[derive(Debug, Serialize, ToSchema)]
pub struct AuditLogResponse {
    pub id: String,
    pub actor_id: Option<String>,
    pub event_type: String,
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
    /// Filter by event type prefix (e.g. "auth.login")
    pub event_type: Option<String>,
    /// Filter by actor UUID
    pub actor_id: Option<Uuid>,
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/orgs/{org}/audit-logs", get(list_audit_logs))
        .with_state(state)
}

/// GET /v1/orgs/{org}/audit-logs - List audit logs (admin only)
async fn list_audit_logs(
    State(state): State<AppState>,
    org: OrgAdmin,
    axum::extract::Query(query): axum::extract::Query<ListAuditLogsQuery>,
) -> Result<
    Json<ListResponse<AuditLogResponse>>,
    (axum::http::StatusCode, Json<super::common::ErrorResponse>),
> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);

    let rows = state
        .db
        .list_audit_logs(
            org.0.org_id,
            limit,
            query.before,
            query.event_type.as_deref(),
            query.actor_id,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to list audit logs: {}", e);
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(super::common::ErrorResponse {
                    error: "Failed to list audit logs".to_string(),
                }),
            )
        })?;

    let items: Vec<AuditLogResponse> = rows
        .into_iter()
        .map(|r| AuditLogResponse {
            id: r.id.to_string(),
            actor_id: r.actor_id.map(|a| a.to_string()),
            event_type: r.event_type,
            ip_address: r.ip_address,
            metadata: r.metadata,
            created_at: r.created_at,
        })
        .collect();

    Ok(Json(ListResponse::new(items)))
}
