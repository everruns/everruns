// Eval API routes
// See specs/evals.md

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use everruns_core::eval::*;
use everruns_core::typed_id::{AgentId, EvalCaseId, EvalId, EvalRunId, HarnessId};

use crate::api::common::{
    ApiOptionExt, ApiPolicyResultExt, ApiResult, ErrorResponse, ListResponse,
};
use crate::auth::{AuthState, ResolvedOrg};
use crate::services::EvalService;
use crate::storage::StorageBackend;
use everruns_core::Caller;
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
    #[schema(value_type = String)]
    pub agent_id: AgentId,
    #[schema(value_type = String)]
    pub harness_id: HarnessId,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub agent_id: Option<AgentId>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(value_type = Option<String>)]
    pub harness_id: Option<HarnessId>,
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
    #[serde(default)]
    pub tags: Option<Vec<String>>,
    pub conversation: Vec<EvalInputMessage>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation: Option<Vec<EvalInputMessage>>,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filter_tags: Option<Vec<String>>,
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
        .route("/v1/evals/{eval_id}/runs/{run_id}/cancel", post(cancel_run))
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
