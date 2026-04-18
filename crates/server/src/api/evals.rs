// Eval API routes
// See specs/evals.md

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use futures::stream;
use serde::Deserialize;
use serde_json::{Map, Value};

use everruns_core::eval::*;
use everruns_core::typed_id::{EvalCaseId, EvalId, EvalResultId, EvalRunId};

use crate::api::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse,
};
use crate::auth::{AuthState, ResolvedOrg};
use crate::services::EvalService;
use crate::services::eval_runner::EvalRunContext;
use crate::storage::StorageBackend;
use everruns_core::Caller;
use std::io;
use std::sync::Arc;

use utoipa::{IntoParams, ToSchema};

// ============================================
// State
// ============================================

#[derive(Clone)]
pub struct AppState {
    pub service: Arc<EvalService>,
    pub auth: AuthState,
}

crate::api::common::impl_auth_state!(AppState);

impl AppState {
    pub fn new(db: Arc<StorageBackend>, auth: AuthState) -> Self {
        Self {
            service: Arc::new(EvalService::new(db)),
            auth,
        }
    }

    pub fn with_run_context(mut self, ctx: Arc<EvalRunContext>) -> Self {
        self.service = Arc::new(EvalService::new(ctx.db.clone()).with_run_context(ctx));
        self
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
#[derive(Debug, Clone, Deserialize, IntoParams)]
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
    let caller = Caller::from(&org);
    let eval = state
        .service
        .create(&caller, req)
        .await
        .map_policy_or_internal("create eval")?;
    Ok((StatusCode::CREATED, Json(eval)))
}

async fn list_evals(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Query(query): Query<ListEvalsQuery>,
) -> ApiResult<ListResponse<Eval>> {
    let caller = Caller::from(&org);
    let evals = state
        .service
        .list(
            &caller,
            query.search.as_deref(),
            query.include_archived.unwrap_or(false),
        )
        .await
        .map_policy_or_internal("list evals")?;
    Ok(Json(ListResponse::new(evals)))
}

async fn get_eval(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> ApiResult<Eval> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let eval = state
        .service
        .get_by_public_id(&caller, &eval_id.to_string())
        .await
        .map_policy_or_internal("get eval")?
        .ok_or_not_found_json("Eval")?;
    Ok(Json(eval))
}

async fn update_eval(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
    Json(req): Json<UpdateEvalRequest>,
) -> ApiResult<Eval> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let eval = state
        .service
        .update(&caller, &eval_id.to_string(), req)
        .await
        .map_policy_or_internal("update eval")?
        .ok_or_not_found_json("Eval")?;
    Ok(Json(eval))
}

async fn delete_eval(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete(&caller, &eval_id.to_string())
        .await
        .map_policy_or_internal("delete eval")?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("Eval not found").into_response(StatusCode::NOT_FOUND))
    }
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
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let case = state
        .service
        .create_case(&caller, &eval_id.to_string(), req)
        .await
        .map_policy_or_internal("create eval case")?;
    Ok((StatusCode::CREATED, Json(case)))
}

async fn list_cases(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> ApiResult<ListResponse<EvalCase>> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let cases = state
        .service
        .list_cases(&caller, &eval_id.to_string())
        .await
        .map_policy_or_internal("list eval cases")?;
    Ok(Json(ListResponse::new(cases)))
}

async fn get_case(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, case_id)): Path<(String, String)>,
) -> ApiResult<EvalCase> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let case_id: EvalCaseId = case_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid case ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let case = state
        .service
        .get_case(&caller, &eval_id.to_string(), &case_id.to_string())
        .await
        .map_policy_or_internal("get eval case")?
        .ok_or_not_found_json("EvalCase")?;
    Ok(Json(case))
}

async fn update_case(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, case_id)): Path<(String, String)>,
    Json(req): Json<UpdateEvalCaseRequest>,
) -> ApiResult<EvalCase> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let case_id: EvalCaseId = case_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid case ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let case = state
        .service
        .update_case(&caller, &eval_id.to_string(), &case_id.to_string(), req)
        .await
        .map_policy_or_internal("update eval case")?
        .ok_or_not_found_json("EvalCase")?;
    Ok(Json(case))
}

async fn delete_case(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, case_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let case_id: EvalCaseId = case_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid case ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let deleted = state
        .service
        .delete_case(&caller, &eval_id.to_string(), &case_id.to_string())
        .await
        .map_policy_or_internal("delete eval case")?;
    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ErrorResponse::new("EvalCase not found").into_response(StatusCode::NOT_FOUND))
    }
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
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let run = state
        .service
        .create_run(&caller, &eval_id.to_string(), req)
        .await
        .map_policy_or_internal("create eval run")?;
    Ok((StatusCode::CREATED, Json(run)))
}

async fn list_runs(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
) -> ApiResult<ListResponse<EvalRun>> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let runs = state
        .service
        .list_runs(&caller, &eval_id.to_string())
        .await
        .map_policy_or_internal("list eval runs")?;
    Ok(Json(ListResponse::new(runs)))
}

async fn get_run(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> ApiResult<EvalRun> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let run_id: EvalRunId = run_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid run ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let run = state
        .service
        .get_run(&caller, &eval_id.to_string(), &run_id.to_string())
        .await
        .map_policy_or_internal("get eval run")?
        .ok_or_not_found_json("EvalRun")?;
    Ok(Json(run))
}

async fn export_run_artifacts(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, (StatusCode, Json<ErrorResponse>)> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let run_id: EvalRunId = run_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid run ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let run = state
        .service
        .get_run(&caller, &eval_id.to_string(), &run_id.to_string())
        .await
        .map_policy_or_internal("export eval run artifacts")?
        .ok_or_not_found_json("EvalRun")?;
    let body = Body::from_stream(stream::iter(run.results.into_iter().map(|result| {
        serde_json::to_vec(&run_artifact_export_value(&result))
            .map(|mut line| {
                line.push(b'\n');
                Bytes::from(line)
            })
            .map_err(|error| {
                tracing::error!("Failed to serialize eval artifact export: {}", error);
                io::Error::other(error)
            })
    })));

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
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let run_id: EvalRunId = run_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid run ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let run = state
        .service
        .cancel_run(&caller, &eval_id.to_string(), &run_id.to_string())
        .await
        .map_policy_or_internal("cancel eval run")?
        .ok_or_not_found_json("EvalRun")?;
    Ok(Json(run))
}

async fn update_result_scores(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id, result_id)): Path<(String, String, String)>,
    Json(req): Json<UpdateEvalResultScoresRequest>,
) -> ApiResult<EvalCaseResult> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let run_id: EvalRunId = run_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid run ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let result_id: EvalResultId = result_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid result ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let result = state
        .service
        .update_result_scores(
            &caller,
            &eval_id.to_string(),
            &run_id.to_string(),
            &result_id.to_string(),
            req,
        )
        .await
        .map_policy_or_internal("update eval result scores")?
        .ok_or_not_found_json("EvalCaseResult")?;
    Ok(Json(result))
}

async fn bulk_update_run_scores(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
    Json(req): Json<BulkUpdateEvalRunScoresRequest>,
) -> ApiResult<ListResponse<EvalCaseResult>> {
    let eval_id: EvalId = eval_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid eval ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let run_id: EvalRunId = run_id.parse().map_err(|e| {
        ErrorResponse::new(format!("Invalid run ID: {e}")).into_response(StatusCode::BAD_REQUEST)
    })?;
    let caller = Caller::from(&org);
    let results = state
        .service
        .bulk_update_run_scores(&caller, &eval_id.to_string(), &run_id.to_string(), req)
        .await
        .map_policy_or_internal("bulk update eval result scores")?;
    Ok(Json(ListResponse::new(results)))
}

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
