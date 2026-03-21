// Session Git HTTP routes
//
// API endpoints for git operations on session filesystems:
//   POST /v1/sessions/{session_id}/git/commit   - Commit current files
//   GET  /v1/sessions/{session_id}/git/log       - Commit history
//   GET  /v1/sessions/{session_id}/git/diff      - Diff between commits
//   GET  /v1/sessions/{session_id}/git/refs      - List refs/branches
//   POST /v1/sessions/{session_id}/git/branches  - Create branch
//   DELETE /v1/sessions/{session_id}/git/branches/{name} - Delete branch

use crate::auth::{AuthState, ResolvedOrg};
use crate::services::session_git::{
    CommitResult, GitCommitInfo, GitDiff, GitRefInfo, SessionGitService,
};
use crate::storage::StorageBackend;
use axum::extract::FromRef;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{delete, get, post},
};
use everruns_core::typed_id::SessionId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

use super::common::verify_session_ownership;

/// Request to create a commit
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CommitRequest {
    /// Commit message
    pub message: String,
    /// Author name (defaults to "Agent")
    #[serde(default)]
    pub author_name: Option<String>,
    /// Author email (defaults to "agent@everruns.local")
    #[serde(default)]
    pub author_email: Option<String>,
    /// Branch name (defaults to "refs/heads/main"); short names like "main" are normalized
    #[serde(default)]
    pub branch: Option<String>,
}

/// Query for log endpoint
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct LogQuery {
    /// Ref to start from (default: HEAD)
    #[serde(default, rename = "ref")]
    pub from_ref: Option<String>,
    /// Max commits to return (default: 50)
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Query for diff endpoint
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct DiffQuery {
    /// Commit OID to diff (required)
    pub oid: String,
    /// Base commit OID (optional, defaults to parent)
    #[serde(default)]
    pub base: Option<String>,
}

/// Request to create a branch
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateBranchRequest {
    /// Branch name (e.g., "feature-xyz")
    pub name: String,
    /// Commit OID (hex) to point to
    pub commit_oid: String,
}

/// Generic success response
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct SuccessResponse {
    pub ok: bool,
}

/// App state for session git routes
#[derive(Clone)]
pub struct AppState {
    pub git_service: Arc<SessionGitService>,
    pub db: Arc<StorageBackend>,
    pub auth: AuthState,
}

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            git_service: Arc::new(SessionGitService::new(db.clone())),
            db,
            auth,
        }
    }
}

impl FromRef<AppState> for AuthState {
    fn from_ref(input: &AppState) -> Self {
        input.auth.clone()
    }
}

/// Create session git routes
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/sessions/{session_id}/git/commit", post(commit))
        .route("/v1/sessions/{session_id}/git/log", get(log))
        .route("/v1/sessions/{session_id}/git/diff", get(diff))
        .route("/v1/sessions/{session_id}/git/refs", get(list_refs))
        .route(
            "/v1/sessions/{session_id}/git/branches",
            post(create_branch),
        )
        .route(
            "/v1/sessions/{session_id}/git/branches/{name}",
            delete(delete_branch),
        )
        .with_state(state)
}

/// Parse session ID string, returning 400 on failure.
fn parse_session_id(s: &str) -> Result<SessionId, (StatusCode, String)> {
    SessionId::parse(s).map_err(|_| (StatusCode::BAD_REQUEST, format!("invalid session_id: {s}")))
}

/// Classify a git error into an appropriate HTTP status code.
fn classify_git_error(e: &anyhow::Error) -> (StatusCode, String) {
    let msg = e.to_string();
    if msg.contains("not found") || msg.contains("unknown revision") {
        (StatusCode::NOT_FOUND, msg)
    } else if msg.contains("bad hex")
        || msg.contains("invalid object id")
        || msg.contains("invalid oid")
    {
        (StatusCode::BAD_REQUEST, msg)
    } else {
        (StatusCode::INTERNAL_SERVER_ERROR, msg)
    }
}

/// POST /v1/sessions/{session_id}/git/commit
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/git/commit",
    request_body = CommitRequest,
    responses(
        (status = 201, description = "Commit created", body = CommitResult),
        (status = 400, description = "Invalid request"),
        (status = 404, description = "Session not found"),
    ),
    tag = "Session Git"
)]
async fn commit(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id_str): Path<String>,
    Json(body): Json<CommitRequest>,
) -> Result<(StatusCode, Json<CommitResult>), (StatusCode, String)> {
    let session_id = parse_session_id(&session_id_str)?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;

    let author_name = body.author_name.as_deref().unwrap_or("Agent");
    let author_email = body
        .author_email
        .as_deref()
        .unwrap_or("agent@everruns.local");

    let result = state
        .git_service
        .commit(
            session_id.uuid(),
            &body.message,
            author_name,
            author_email,
            body.branch.as_deref(),
        )
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "git commit failed");
            classify_git_error(&e)
        })?;

    Ok((StatusCode::CREATED, Json(result)))
}

/// GET /v1/sessions/{session_id}/git/log
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/git/log",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("ref" = Option<String>, Query, description = "Ref to start from (default: HEAD)"),
        ("limit" = Option<usize>, Query, description = "Max commits (default: 50)"),
    ),
    responses(
        (status = 200, description = "Commit log", body = Vec<GitCommitInfo>),
        (status = 404, description = "Session or ref not found"),
    ),
    tag = "Session Git"
)]
async fn log(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id_str): Path<String>,
    Query(query): Query<LogQuery>,
) -> Result<Json<Vec<GitCommitInfo>>, (StatusCode, String)> {
    let session_id = parse_session_id(&session_id_str)?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;

    let limit = query.limit.unwrap_or(50).min(500);

    let commits = state
        .git_service
        .log(session_id.uuid(), query.from_ref.as_deref(), Some(limit))
        .await
        .map_err(|e| {
            let msg = e.to_string();
            // Fix: return empty list for missing ref (no commits yet) instead of 500
            if msg.contains("not found") {
                tracing::debug!(error = %msg, "git log: ref not found (no commits yet)");
                (StatusCode::NOT_FOUND, format!("Ref not found: {msg}"))
            } else {
                tracing::error!(error = %msg, "git log failed");
                (StatusCode::INTERNAL_SERVER_ERROR, msg)
            }
        })?;

    Ok(Json(commits))
}

/// GET /v1/sessions/{session_id}/git/diff
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/git/diff",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("oid" = String, Query, description = "Commit OID to diff"),
        ("base" = Option<String>, Query, description = "Base commit OID (default: parent)"),
    ),
    responses(
        (status = 200, description = "Diff result", body = GitDiff),
        (status = 400, description = "Invalid commit OID"),
        (status = 404, description = "Commit not found"),
    ),
    tag = "Session Git"
)]
async fn diff(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id_str): Path<String>,
    Query(query): Query<DiffQuery>,
) -> Result<Json<GitDiff>, (StatusCode, String)> {
    let session_id = parse_session_id(&session_id_str)?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;

    let result = state
        .git_service
        .diff(session_id.uuid(), &query.oid, query.base.as_deref())
        .await
        .map_err(|e| {
            // Fix: classify git errors into proper HTTP status codes
            tracing::warn!(error = %e, "git diff failed");
            classify_git_error(&e)
        })?;

    Ok(Json(result))
}

/// GET /v1/sessions/{session_id}/git/refs
#[utoipa::path(
    get,
    path = "/v1/sessions/{session_id}/git/refs",
    params(
        ("session_id" = String, Path, description = "Session ID"),
    ),
    responses(
        (status = 200, description = "List of refs", body = Vec<GitRefInfo>),
    ),
    tag = "Session Git"
)]
async fn list_refs(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id_str): Path<String>,
) -> Result<Json<Vec<GitRefInfo>>, (StatusCode, String)> {
    let session_id = parse_session_id(&session_id_str)?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;

    let refs = state
        .git_service
        .list_refs(session_id.uuid())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "git list refs failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    Ok(Json(refs))
}

/// POST /v1/sessions/{session_id}/git/branches
#[utoipa::path(
    post,
    path = "/v1/sessions/{session_id}/git/branches",
    request_body = CreateBranchRequest,
    responses(
        (status = 201, description = "Branch created", body = SuccessResponse),
        (status = 400, description = "Invalid request"),
    ),
    tag = "Session Git"
)]
async fn create_branch(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(session_id_str): Path<String>,
    Json(body): Json<CreateBranchRequest>,
) -> Result<(StatusCode, Json<SuccessResponse>), (StatusCode, String)> {
    let session_id = parse_session_id(&session_id_str)?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;

    state
        .git_service
        .create_branch(session_id.uuid(), &body.name, &body.commit_oid)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "git create branch failed");
            classify_git_error(&e)
        })?;

    Ok((StatusCode::CREATED, Json(SuccessResponse { ok: true })))
}

/// DELETE /v1/sessions/{session_id}/git/branches/{name}
#[utoipa::path(
    delete,
    path = "/v1/sessions/{session_id}/git/branches/{name}",
    params(
        ("session_id" = String, Path, description = "Session ID"),
        ("name" = String, Path, description = "Branch name"),
    ),
    responses(
        (status = 200, description = "Branch deleted", body = SuccessResponse),
        (status = 404, description = "Branch not found"),
    ),
    tag = "Session Git"
)]
async fn delete_branch(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((session_id_str, name)): Path<(String, String)>,
) -> Result<Json<SuccessResponse>, (StatusCode, String)> {
    let session_id = parse_session_id(&session_id_str)?;
    verify_session_ownership(&state.db, org.org_id, session_id)
        .await
        .map_err(|s| (s, "Session not found".to_string()))?;

    let deleted = state
        .git_service
        .delete_branch(session_id.uuid(), &name)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "git delete branch failed");
            (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
        })?;

    if !deleted {
        return Err((StatusCode::NOT_FOUND, "Branch not found".to_string()));
    }

    Ok(Json(SuccessResponse { ok: true }))
}
