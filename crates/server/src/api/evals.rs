// Eval API routes
// See knowledge/evaluation/evals.md

use axum::body::{Body, Bytes};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use serde_json::{Map, Value};

use everruns_platform::eval::*;
use everruns_provider::typed_id::EvalResultId;

use crate::api::common::{ApiResult, ErrorResponse, ListResponse};
use crate::api::dispatch::{Dispatchable, impl_dispatchable};
use crate::auth::{AuthState, ResolvedOrg};
use crate::domains::common::{Command, Ctx};
use crate::domains::evals::EvalService;
use crate::domains::evals::dataset::ExportEvalRunDatasetRequest;
use crate::domains::evals::runner::EvalRunContext;
use crate::domains::evals::{
    BulkUpdateEvalRunScores, CancelEvalRun, CreateEval, CreateEvalCase, CreateEvalRun,
    CreateEvalRunShare, DeleteEval, DeleteEvalCase, EvalImportPreflightCmd, ExportEvalRunArtifacts,
    ExportEvalRunDataset, GetEval, GetEvalCase, GetEvalRun, GetEvalRunDataset, GetEvalRunShare,
    ImportAtifTrajectories, ImportEvalRun, ListEvalCases, ListEvalRuns, ListEvals,
    RevokeEvalRunShare, UpdateEval, UpdateEvalCase, UpdateEvalResultScores,
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
        .with_feature_flags(org.feature_flags.clone())
        .with_eval_service(self.service.clone())
    }
}

// ============================================
// Request/Response types
// ============================================

/// Request to create a new eval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEvalRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// Session setup target (harness+agent, app, or full session params).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
    pub tags: Option<Vec<String>>,
}

/// Request to update an eval
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct UpdateEvalRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// Session setup target (harness+agent, app, or full session params).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form tags attached to this resource.
    pub tags: Option<Vec<String>>,
}

/// Request to create an eval case
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct CreateEvalCaseRequest {
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// Optional per-case target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(default)]
    /// Free-form tags attached to this resource.
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
    /// Human-readable name. Safe to render in user-facing messages.
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
    /// Optional per-case target override.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form tags attached to this resource.
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
    /// Current lifecycle status.
    pub status: Option<ExternalScoreStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form metadata attached to this resource.
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BulkUpdateEvalResultScoresItem {
    #[schema(value_type = String)]
    pub result_id: EvalResultId,
    pub scores: Vec<Score>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Current lifecycle status.
    pub status: Option<ExternalScoreStatus>,
}

#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct BulkUpdateEvalRunScoresRequest {
    pub results: Vec<BulkUpdateEvalResultScoresItem>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Free-form metadata attached to this resource.
    pub metadata: Option<serde_json::Value>,
}

// ============================================
// Import (external eval results) — everruns as host/viewer.
// See proposals/mira-results-publishing.md.
// ============================================

/// A whole external run group: one external run, one entry per eval. Maps to
/// one everruns EvalRun per eval, all sharing `source.run_id`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportEvalRunRequest {
    pub source: ImportEvalSource,
    pub evals: Vec<ImportEvalGroup>,
}

/// Attribution for the external system that produced the run.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportEvalSource {
    /// External system name, e.g. "mira".
    pub system: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Stable external run id: cross-eval group key + idempotency key.
    pub run_id: String,
    /// Optional environment/labels (git commit, host, etc.).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

/// One eval's worth of results within the run. The eval is upserted by `name`.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportEvalGroup {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub cases: Vec<ImportEvalCaseEntry>,
}

/// One case result. The case is upserted by `name` (identity-only: everruns
/// never re-executes it).
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportEvalCaseEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Display-only input turns shown in the UI.
    #[serde(default)]
    pub input: Vec<String>,
    /// Provider/model labels this result was produced against.
    pub target: ImportEvalTarget,
    pub status: ImportCaseStatus,
    /// Named, attributed scores. Stored opaque; everruns does not re-grade.
    #[serde(default)]
    pub scores: Vec<ImportScore>,
    /// Normalized transcript (messages, tool calls, events, parts, files).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<serde_json::Value>,
    /// Open-vocab metrics bag (cost_usd, cache/reasoning tokens, ttft, ...).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

/// Provider/model labels for an externally-executed result.
#[derive(Debug, Clone, Deserialize, ToSchema)]
pub struct ImportEvalTarget {
    pub provider: String,
    pub model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<serde_json::Value>,
}

/// Verdict for an imported case (trusted as-is; not recomputed).
#[derive(Debug, Clone, Copy, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ImportCaseStatus {
    Passed,
    Failed,
    Errored,
    Timeout,
    Skipped,
}

impl ImportCaseStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ImportCaseStatus::Passed => "passed",
            ImportCaseStatus::Failed => "failed",
            ImportCaseStatus::Errored => "errored",
            ImportCaseStatus::Timeout => "timeout",
            ImportCaseStatus::Skipped => "skipped",
        }
    }
}

/// A single named score from an external scorer.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ImportScore {
    pub scorer: String,
    pub value: f64,
    pub pass: bool,
    #[serde(default)]
    pub reason: String,
    /// Scorer was not applicable (excluded from aggregate).
    #[serde(default)]
    pub na: bool,
}

/// Result of an ATIF trajectory import (knowledge/evaluation/atif-adoption.md): eval cases
/// created/updated from imported trajectories, upserted by case name.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct AtifImportReport {
    /// Number of eval cases created.
    pub created: u64,
    /// Number of existing eval cases updated (matched by name).
    pub updated: u64,
    /// Public ids of the affected cases, in import order.
    pub case_ids: Vec<String>,
}

/// Preflight capability report so optional-feature clients (e.g. Mira) can
/// check before publishing instead of failing mid-import.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalImportPreflight {
    /// Whether the `evals` feature is enabled for this org.
    pub evals_enabled: bool,
    /// Whether the caller may import (holds eval-management permission).
    pub can_import: bool,
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

// ============================================
// Share links (read-only public views).
// See knowledge/evaluation/evals.md, knowledge/execution/public-endpoints.md.
// ============================================

/// A freshly minted share link. The raw `token` is returned once and never
/// stored; build the public URL `/shared/eval-runs/<token>` from it.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalRunShareLink {
    pub token: String,
    pub token_prefix: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

/// Whether a run currently has an active share link.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct EvalRunShareStatus {
    pub active: bool,
}

/// Attribution shown on a public share (external runs). Display fields only.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PublicAttribution {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// Sanitized, anonymous view of one eval run, returned by the public share
/// endpoint. Omits org/internal ids, session ids, internal targets, and
/// attribution env labels — only the shared content remains.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PublicEvalRun {
    pub id: String,
    pub status: EvalRunStatus,
    pub source: EvalRunSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attribution: Option<PublicAttribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<RunSummary>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<chrono::DateTime<chrono::Utc>>,
    pub results: Vec<PublicEvalCaseResult>,
}

/// One case result in a public run view.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct PublicEvalCaseResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub case_name: Option<String>,
    /// Only label-only (external) targets are exposed; internal targets are dropped.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<EvalTarget>,
    pub status: CaseResultStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scores: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transcript: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turns: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/v1/evals", post(create_eval).get(list_evals))
        // Import (external eval results). Static segments take priority over
        // `{eval_id}`, so these never shadow eval-by-id routes.
        .route("/v1/evals/import", post(import_eval_run))
        .route("/v1/evals/import/preflight", get(import_preflight))
        .route(
            "/v1/evals/{eval_id}",
            get(get_eval).patch(update_eval).delete(delete_eval),
        )
        // ATIF trajectory import → eval cases (knowledge/evaluation/atif-adoption.md).
        .route("/v1/evals/{eval_id}/atif_import", post(import_atif))
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
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/dataset",
            post(export_run_dataset),
        )
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/dataset/{dataset_id}",
            get(get_run_dataset),
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
        // Read-only share link (mint / status / revoke).
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/share",
            post(create_run_share)
                .get(get_run_share)
                .delete(revoke_run_share),
        )
        // Public, UNAUTHENTICATED read of a shared run (no auth extractor).
        .route("/v1/public/eval-runs/{token}", get(public_eval_run))
        .with_state(state)
}

// ============================================
// Share-link handlers
// ============================================

async fn create_run_share(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> ApiResult<EvalRunShareLink> {
    let link = CreateEvalRunShare { eval_id, run_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(link))
}

async fn get_run_share(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> ApiResult<EvalRunShareStatus> {
    let status = GetEvalRunShare { eval_id, run_id }
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(status))
}

async fn revoke_run_share(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .dispatcher(&org)
        .run_no_content(RevokeEvalRunShare { eval_id, run_id })
        .await
}

/// Public, unauthenticated read of a shared eval run. No auth extractor ⇒
/// anonymous; the token is the authorization. Unknown/revoked/expired ⇒ 404.
async fn public_eval_run(
    State(state): State<AppState>,
    Path(token): Path<String>,
) -> Result<Json<PublicEvalRun>, (StatusCode, Json<ErrorResponse>)> {
    match state.service.resolve_public_share(&token).await {
        Ok(Some(run)) => Ok(Json(run)),
        Ok(None) => Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse::new("Shared run not found")),
        )),
        Err(_) => Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse::new("internal_error")),
        )),
    }
}

// ============================================
// Import handlers
// ============================================

async fn import_eval_run(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Json(req): Json<ImportEvalRunRequest>,
) -> ApiResult<ListResponse<EvalRun>> {
    let runs = ImportEvalRun { req }.run(&state.ctx(&org)).await?;
    Ok(Json(ListResponse::new(runs)))
}

async fn import_preflight(
    org: ResolvedOrg,
    State(state): State<AppState>,
) -> ApiResult<EvalImportPreflight> {
    let report = EvalImportPreflightCmd {}.run(&state.ctx(&org)).await?;
    Ok(Json(report))
}

/// Import ATIF trajectories as eval cases. Accepts NDJSON (one trajectory per
/// line) or JSON (array, single object, or `{ "trajectories": [...] }`) as the
/// raw body, so both content types work without a wrapper schema.
async fn import_atif(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path(eval_id): Path<String>,
    body: String,
) -> ApiResult<AtifImportReport> {
    let report = ImportAtifTrajectories { eval_id, body }
        .run(&state.ctx(&org))
        .await?;
    Ok(Json(report))
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
    state
        .dispatcher(&org)
        .run_list(ListEvals {
            search: query.search,
            include_archived: query.include_archived.unwrap_or(false),
        })
        .await
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
    state
        .dispatcher(&org)
        .run_list(ListEvalCases { eval_id })
        .await
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
    state
        .dispatcher(&org)
        .run_list(ListEvalRuns { eval_id })
        .await
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

/// Enqueue an async dataset export and return the handle (202 Accepted).
///
/// The NDJSON is produced by a background job; fetch it once ready via
/// `GET .../dataset/{dataset_id}`.
async fn export_run_dataset(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id)): Path<(String, String)>,
    Json(req): Json<ExportEvalRunDatasetRequest>,
) -> Result<(StatusCode, Json<EvalRunDataset>), (StatusCode, Json<ErrorResponse>)> {
    let dataset = ExportEvalRunDataset {
        eval_id,
        run_id,
        req,
    }
    .run(&state.ctx(&org))
    .await?;
    Ok((StatusCode::ACCEPTED, Json(dataset)))
}

/// Fetch a dataset-export handle: status, and (once completed) the NDJSON body.
async fn get_run_dataset(
    org: ResolvedOrg,
    State(state): State<AppState>,
    Path((eval_id, run_id, dataset_id)): Path<(String, String, String)>,
) -> ApiResult<EvalRunDataset> {
    state
        .dispatcher(&org)
        .run(GetEvalRunDataset {
            eval_id,
            run_id,
            dataset_id,
        })
        .await
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
    use everruns_provider::typed_id::EvalCaseId;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    #[test]
    fn run_artifact_export_maps_patch_to_model_patch() {
        let result = EvalCaseResult {
            public_id: everruns_provider::typed_id::EvalResultId::from_uuid(Uuid::now_v7()),
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
            public_id: everruns_provider::typed_id::EvalResultId::from_uuid(Uuid::now_v7()),
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
