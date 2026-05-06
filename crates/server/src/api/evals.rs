// Eval API routes
// See specs/evals.md

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::Deserialize;
#[cfg(test)]
use serde_json::{Map, Value};

use everruns_core::eval::*;
use everruns_core::typed_id::EvalResultId;

use crate::api::common::{ApiResult, ErrorResponse, ListResponse};
use crate::api::dispatch::{Dispatchable, impl_dispatchable};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::evals::EvalService;
use crate::domains::evals::runner::EvalRunContext;
use crate::domains::evals::{
    BulkUpdateEvalRunScores, CancelEvalRun, CreateEval, CreateEvalCase, CreateEvalRun, DeleteEval,
    DeleteEvalCase, ExportEvalRunArtifacts, GetEval, GetEvalCase, GetEvalRun, ListEvalCases,
    ListEvalRuns, ListEvals, UpdateEval, UpdateEvalCase, UpdateEvalResultScores,
};
use crate::storage::StorageBackend;
use everruns_core::Caller;
use std::sync::Arc;

use utoipa::{IntoParams, ToSchema};

// ============================================
// State
// ============================================

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<StorageBackend>,
    pub service: Arc<EvalService>,
    pub auth: AuthState,
}

crate::api::common::impl_auth_state!(AppState);
impl_dispatchable!(AppState);

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            db: db.clone(),
            service: Arc::new(EvalService::new(db)),
            auth,
        }
    }

    pub fn with_run_context(mut self, ctx: Arc<EvalRunContext>) -> Self {
        self.service = Arc::new(EvalService::new(self.db.clone()).with_run_context(ctx));
        self
    }

    fn ctx(&self, org: &ResolvedOrg) -> Ctx {
        Ctx::minimal(
            Caller::from(org),
            self.db.clone(),
            None,
            self.auth.permission_resolver.clone(),
        )
        .with_eval_service(self.service.clone())
    }
}

// ============================================
// Request/Response types
// ============================================

/// Request to create a new eval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEvalRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Session setup target (harness+agent, app, or full session params).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
}

/// Request to update an eval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateEvalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Session setup target (harness+agent, app, or full session params).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
}

/// Request to create an eval case
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEvalCaseRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional per-case target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub conversation: Vec<EvalInputMessage>,
    /// Verification messages sent after conversation completes and session idles.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post: Option<Vec<EvalInputMessage>>,
    /// Session files to capture after scoring completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<ArtifactSpec>>,
    pub scorers: Vec<Scorer>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

/// Request to update an eval case
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateEvalCaseRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Optional per-case target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Vec<EvalInputMessage>>,
    /// Verification messages sent after conversation completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post: Option<Vec<EvalInputMessage>>,
    /// Session files to capture after scoring completes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<ArtifactSpec>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scorers: Option<Vec<Scorer>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<i32>,
}

/// Request to create an eval run
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEvalRunRequest {
    /// Optional per-run target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ExternalScoreStatus {
    Passed,
    Failed,
    Errored,
}

impl std::fmt::Display for ExternalScoreStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExternalScoreStatus::Passed => write!(f, "passed"),
            ExternalScoreStatus::Failed => write!(f, "failed"),
            ExternalScoreStatus::Errored => write!(f, "errored"),
        }
    }
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateEvalResultScoresRequest {
    pub scores: Vec<Score>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ExternalScoreStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BulkUpdateEvalResultScoresItem {
    #[schema(value_type = String)]
    pub result_id: EvalResultId,
    pub scores: Vec<Score>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ExternalScoreStatus>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BulkUpdateEvalRunScoresRequest {
    pub results: Vec<BulkUpdateEvalResultScoresItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// Query parameters for listing evals
#[derive(Debug, Clone, Deserialize, IntoParams, ToSchema)]
pub struct ListEvalsQuery {
    pub search: Option<String>,
    pub include_archived: Option<bool>,
}

// ============================================
// Routes
// ============================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/evals", post(create_eval).get(list_evals))
        .route(
            "/v1/evals/{eval_id}",
            get(get_eval).patch(update_eval).delete(delete_eval),
        )
        // Cases
        .route(
            "/v1/evals/{eval_id}/cases",
            post(create_case).get(list_cases),
        )
        .route(
            "/v1/evals/{eval_id}/cases/{case_id}",
            get(get_case).patch(update_case).delete(delete_case),
        )
        // Runs
        .route("/v1/evals/{eval_id}/runs", post(create_run).get(list_runs))
        .route("/v1/evals/{eval_id}/runs/{run_id}", get(get_run))
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/artifacts",
            get(export_run_artifacts),
        )
        .route("/v1/evals/{eval_id}/runs/{run_id}/cancel", post(cancel_run))
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/results/{result_id}/scores",
            patch(update_result_scores),
        )
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/scores",
            patch(bulk_update_run_scores),
        )
        .with_state(state)
}

// ============================================
// Eval handlers
// ============================================

async fn create_eval(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<CreateEvalRequest>,
) -> Result<(StatusCode, Json<Eval>), (StatusCode, Json<ErrorResponse>)> {
    state.dispatcher(&org).run_created(CreateEval(req)).await
}

async fn list_evals(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListEvalsQuery>,
) -> ApiResult<ListResponse<Eval>> {
    let evals = ListEvals {
        search: query.search,
        include_archived: query.include_archived.unwrap_or(false),
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(ListResponse::new(evals)))
}

async fn get_eval(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> ApiResult<Eval> {
    state.dispatcher(&org).run(GetEval { eval_id }).await
}

async fn update_eval(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
    Json(req): Json<UpdateEvalRequest>,
) -> ApiResult<Eval> {
    state
        .dispatcher(&org)
        .run(UpdateEval { eval_id, req })
        .await
}

async fn delete_eval(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(DeleteEval { eval_id })
        .await
}

// ============================================
// Case handlers
// ============================================

async fn create_case(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
    Json(req): Json<CreateEvalCaseRequest>,
) -> Result<(StatusCode, Json<EvalCase>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created(CreateEvalCase { eval_id, req })
        .await
}

async fn list_cases(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> ApiResult<ListResponse<EvalCase>> {
    let cases = ListEvalCases { eval_id }.run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(cases)))
}

async fn get_case(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, case_id)): Path<(String, String)>,
) -> ApiResult<EvalCase> {
    state
        .dispatcher(&org)
        .run(GetEvalCase { eval_id, case_id })
        .await
}

async fn update_case(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, case_id)): Path<(String, String)>,
    Json(req): Json<UpdateEvalCaseRequest>,
) -> ApiResult<EvalCase> {
    state
        .dispatcher(&org)
        .run(UpdateEvalCase {
            eval_id,
            case_id,
            req,
        })
        .await
}

async fn delete_case(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, case_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(DeleteEvalCase { eval_id, case_id })
        .await
}

// ============================================
// Run handlers
// ============================================

async fn create_run(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
    Json(req): Json<CreateEvalRunRequest>,
) -> Result<(StatusCode, Json<EvalRun>), (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_created(CreateEvalRun { eval_id, req })
        .await
}

async fn list_runs(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> ApiResult<ListResponse<EvalRun>> {
    let runs = ListEvalRuns { eval_id }.run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(runs)))
}

async fn get_run(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> ApiResult<EvalRun> {
    state
        .dispatcher(&org)
        .run(GetEvalRun { eval_id, run_id })
        .await
}

async fn export_run_artifacts(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let export = ExportEvalRunArtifacts { eval_id, run_id }
        .run(&state.ctx(&org))
        .await?;
    let body = Body::from(Bytes::from(export.body));

    Ok((
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/x-ndjson".to_string(),
        )],
        body,
    ))
}

async fn cancel_run(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> ApiResult<EvalRun> {
    state
        .dispatcher(&org)
        .run(CancelEvalRun { eval_id, run_id })
        .await
}

async fn update_result_scores(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id, result_id)): Path<(String, String, String)>,
    Json(req): Json<UpdateEvalResultScoresRequest>,
) -> ApiResult<EvalCaseResult> {
    let result = UpdateEvalResultScores {
        eval_id,
        run_id,
        result_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(result))
}

async fn bulk_update_run_scores(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
    Json(req): Json<BulkUpdateEvalRunScoresRequest>,
) -> ApiResult<ListResponse<EvalCaseResult>> {
    let results = BulkUpdateEvalRunScores {
        eval_id,
        run_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok(Json(ListResponse::new(results)))
}

#[cfg(test)]
fn run_artifact_export_value(result: &EvalCaseResult) -> Value {
    let mut export = Map::new();
    export.insert(
        "instance_id".to_string(),
        Value::String(
            result
                .case_name
                .clone()
                .unwrap_or_else(|| result.eval_case_id.to_string()),
        ),
    );

    if let Some(artifacts) = &result.artifacts {
        for (name, content) in artifacts {
            let key = if name == "patch" {
                "model_patch"
            } else {
                name.as_str()
            };
            if export.contains_key(key) {
                tracing::warn!(
                    eval_case_id = %result.eval_case_id,
                    artifact_name = %name,
                    export_key = %key,
                    "Skipping colliding eval artifact export field"
                );
                continue;
            }
            export.insert(key.to_string(), Value::String(content.clone()));
        }
    }

    Value::Object(export)
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::typed_id::EvalCaseId;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    #[test]
    fn run_artifact_export_maps_patch_to_model_patch() {
        let result = EvalCaseResult {
            public_id: everruns_core::typed_id::EvalResultId::from_uuid(Uuid::now_v7()),
            internal_id: Uuid::nil(),
            eval_case_id: EvalCaseId::from_uuid(Uuid::now_v7()),
            case_name: Some("astropy__astropy-12907".to_string()),
            session_id: None,
            target: None,
            target_snapshot: None,
            status: CaseResultStatus::Passed,
            scores: None,
            metadata: None,
            turns: None,
            latency_ms: None,
            input_tokens: None,
            output_tokens: None,
            error_message: None,
            artifacts: Some(BTreeMap::from([
                ("patch".to_string(), "diff --git a/file b/file".to_string()),
                ("log".to_string(), "done".to_string()),
            ])),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let value = run_artifact_export_value(&result);
        assert_eq!(value["instance_id"], "astropy__astropy-12907");
        assert_eq!(value["model_patch"], "diff --git a/file b/file");
        assert_eq!(value["log"], "done");
        assert!(value.get("patch").is_none());
    }

    #[test]
    fn run_artifact_export_preserves_existing_model_patch() {
        let result = EvalCaseResult {
            public_id: everruns_core::typed_id::EvalResultId::from_uuid(Uuid::now_v7()),
            internal_id: Uuid::nil(),
            eval_case_id: EvalCaseId::from_uuid(Uuid::now_v7()),
            case_name: Some("collision-case".to_string()),
            session_id: None,
            target: None,
            target_snapshot: None,
            status: CaseResultStatus::Passed,
            scores: None,
            metadata: None,
            turns: None,
            latency_ms: None,
            input_tokens: None,
            output_tokens: None,
            error_message: None,
            artifacts: Some(BTreeMap::from([
                ("model_patch".to_string(), "kept".to_string()),
                ("patch".to_string(), "ignored".to_string()),
            ])),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };

        let value = run_artifact_export_value(&result);
        assert_eq!(value["model_patch"], "kept");
        assert!(value.get("patch").is_none());
    }
}
