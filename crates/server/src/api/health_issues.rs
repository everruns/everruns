use super::common::{ApiResult, ErrorResponse, impl_auth_state};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::{
    common::{Command, Ctx},
    health_issues::{types::*, *},
};
use crate::storage::{EncryptionService, StorageBackend};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
    pub encryption: Option<Arc<EncryptionService>>,
}
impl AppState {
    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            everruns_core::Caller::from(org),
            self.db.clone(),
            self.encryption.clone(),
            self.auth.permission_resolver.clone(),
        )
        .with_feature_flags(org.feature_flags.clone())
    }
}
impl_auth_state!(AppState);
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/health-issues", get(list_health_issues))
        .route("/v1/health-issues/{issue_id}", get(get_health_issue))
        .route(
            "/v1/health-issues/{issue_id}/check",
            post(check_health_issue),
        )
        .route(
            "/v1/health-issues/{issue_id}/snooze",
            post(snooze_health_issue),
        )
        .with_state(state)
}
#[derive(Deserialize, utoipa::IntoParams)]
pub struct HealthIssueQuery {
    pub channel_id: Option<String>,
    pub offset: Option<i64>,
    pub limit: Option<i64>,
}

#[utoipa::path(get,path="/v1/health-issues",params(HealthIssueQuery),responses((status=200,body=HealthIssueList),(status=403,body=ErrorResponse)),tag="health-issues")]
pub async fn list_health_issues(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(q): Query<HealthIssueQuery>,
) -> ApiResult<HealthIssueList> {
    Ok(Json(
        ListHealthIssues {
            channel_id: q.channel_id,
            offset: q.offset,
            limit: q.limit,
        }
        .run(&state.ctx(&org))
        .await?,
    ))
}
#[utoipa::path(get,path="/v1/health-issues/{issue_id}",params(("issue_id"=Uuid,Path)),responses((status=200,body=HealthIssue),(status=404,body=ErrorResponse)),tag="health-issues")]
pub async fn get_health_issue(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<HealthIssue> {
    Ok(Json(
        GetHealthIssue { issue_id }.run(&state.ctx(&org)).await?,
    ))
}
#[utoipa::path(post,path="/v1/health-issues/{issue_id}/check",params(("issue_id"=Uuid,Path)),responses((status=200,body=HealthIssue),(status=429,body=ErrorResponse)),tag="health-issues")]
pub async fn check_health_issue(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<HealthIssue> {
    Ok(Json(
        CheckHealthIssue { issue_id }.run(&state.ctx(&org)).await?,
    ))
}
#[utoipa::path(post,path="/v1/health-issues/{issue_id}/snooze",params(("issue_id"=Uuid,Path)),responses((status=200,body=HealthIssue)),tag="health-issues")]
pub async fn snooze_health_issue(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<HealthIssue> {
    Ok(Json(
        SnoozeHealthIssue { issue_id }.run(&state.ctx(&org)).await?,
    ))
}
