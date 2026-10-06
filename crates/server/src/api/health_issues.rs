use super::common::{ApiResult, ErrorResponse};
use crate::api::state::ApiState;
use crate::auth::ResolvedOrg;
use crate::domains::{
    common::Command,
    health_issues::{types::*, *},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

pub fn routes(state: ApiState) -> Router {
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
    /// Filter by public channel identifier.
    pub channel_id: Option<String>,
    /// Number of matching issues to skip; defaults to zero.
    pub offset: Option<i64>,
    /// Page size from 1 through 100; defaults to 20.
    pub limit: Option<i64>,
}

#[utoipa::path(get,path="/v1/health-issues",params(HealthIssueQuery),responses((status=200,body=HealthIssueList),(status=403,body=ErrorResponse)),tag="health-issues")]
/// List pending operational health issues visible to the caller in the current organization.
pub async fn list_health_issues(
    org: ResolvedOrg,
    State(state): State<ApiState>,
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
/// Get current issue evidence and recovery guidance in the current organization.
pub async fn get_health_issue(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<HealthIssue> {
    Ok(Json(
        GetHealthIssue { issue_id }.run(&state.ctx(&org)).await?,
    ))
}
#[utoipa::path(post,path="/v1/health-issues/{issue_id}/check",params(("issue_id"=Uuid,Path)),responses((status=200,body=HealthIssue),(status=429,body=ErrorResponse)),tag="health-issues")]
/// Verify current installation permissions without changing provider data; requires agent management access and enforces a check cooldown.
pub async fn check_health_issue(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<HealthIssue> {
    Ok(Json(
        CheckHealthIssue { issue_id }.run(&state.ctx(&org)).await?,
    ))
}
#[utoipa::path(post,path="/v1/health-issues/{issue_id}/snooze",params(("issue_id"=Uuid,Path)),responses((status=200,body=HealthIssue)),tag="health-issues")]
/// Snooze the authenticated user's reminders for one day while keeping the shared issue pending.
pub async fn snooze_health_issue(
    org: ResolvedOrg,
    State(state): State<ApiState>,
    Path(issue_id): Path<Uuid>,
) -> ApiResult<HealthIssue> {
    Ok(Json(
        SnoozeHealthIssue { issue_id }.run(&state.ctx(&org)).await?,
    ))
}
