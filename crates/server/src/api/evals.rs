// Eval API routes
// See knowledge/evaluation/evals.md

pub use crate::domains::evals::types::{
    AtifImportReport, BulkUpdateEvalResultScoresItem, BulkUpdateEvalRunScoresRequest,
    CreateEvalCaseRequest, CreateEvalRequest, CreateEvalRunRequest, EvalImportPreflight,
    EvalRunShareLink, EvalRunShareStatus, ExternalScoreStatus, ImportCaseStatus,
    ImportEvalCaseEntry, ImportEvalGroup, ImportEvalRunRequest, ImportEvalSource, ImportEvalTarget,
    ImportScore, ListEvalsQuery, PublicAttribution, PublicEvalCaseResult, PublicEvalRun,
    UpdateEvalCaseRequest, UpdateEvalRequest, UpdateEvalResultScoresRequest,
};
use axum::body::{Body, Bytes};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
#[cfg(test)]
use serde_json::{Map, Value};

use crate::records::eval::*;

use crate::api::command_http::CommandRouterExt;
use crate::api::common::{ApiResult, ErrorResponse};
use crate::api::dispatch::impl_dispatchable;
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
// Routes
// ============================================

pub fn routes(state: AppState) -> Router {
    Router::new()
        .command::<CreateEval>()
        .command::<ListEvals>()
        // Import (external eval results). Static segments take priority over
        // `{eval_id}`, so these never shadow eval-by-id routes.
        .command::<ImportEvalRun>()
        .command::<EvalImportPreflightCmd>()
        .command::<GetEval>()
        .command::<UpdateEval>()
        .command::<DeleteEval>()
        // ATIF trajectory import → eval cases (knowledge/evaluation/atif-adoption.md).
        // Hand-written: the raw body is NDJSON or JSON, not a params object.
        .route("/v1/evals/{eval_id}/atif_import", post(import_atif))
        // Cases
        .command::<CreateEvalCase>()
        .command::<ListEvalCases>()
        .command::<GetEvalCase>()
        .command::<UpdateEvalCase>()
        .command::<DeleteEvalCase>()
        // Runs
        .command::<CreateEvalRun>()
        .command::<ListEvalRuns>()
        .command::<GetEvalRun>()
        // Hand-written: NDJSON body rather than JSON.
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/artifacts",
            get(export_run_artifacts),
        )
        // Hand-written: answers 202 Accepted, which no generic mode emits.
        .route(
            "/v1/evals/{eval_id}/runs/{run_id}/dataset",
            post(export_run_dataset),
        )
        .command::<GetEvalRunDataset>()
        .command::<CancelEvalRun>()
        .command::<UpdateEvalResultScores>()
        .command::<BulkUpdateEvalRunScores>()
        // Read-only share link (mint / status / revoke).
        .command::<CreateEvalRunShare>()
        .command::<GetEvalRunShare>()
        .command::<RevokeEvalRunShare>()
        // Public, UNAUTHENTICATED read of a shared run (no auth extractor).
        .route("/v1/public/eval-runs/{token}", get(public_eval_run))
        .with_state(state)
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

/// Import ATIF trajectories as eval cases. Accepts NDJSON (one trajectory per
/// line) or JSON (array, single object, or `{ "trajectories": [...] }`) as the
/// raw body, so both content types work without a wrapper schema.
#[utoipa::path(
    post,
    path = "/v1/evals/{eval_id}/atif_import",
    summary = "Import ATIF trajectories as eval cases.",
    params(("eval_id" = String, Path, description = "Eval ID")),
    request_body(content = String, description = "ATIF trajectories as NDJSON or JSON", content_type = "application/x-ndjson"),
    responses(
        (status = 200, description = "Import report", body = AtifImportReport),
        (status = 400, description = "Invalid payload", body = ErrorResponse),
        (status = 404, description = "Eval not found", body = ErrorResponse)
    ),
    tag = "evals"
)]
pub async fn import_atif(
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

/// Export eval run artifacts as NDJSON, one line per case result.
#[utoipa::path(
    get,
    path = "/v1/evals/{eval_id}/runs/{run_id}/artifacts",
    summary = "Export eval run artifacts as NDJSON.",
    params(
        ("eval_id" = String, Path, description = "Eval ID"),
        ("run_id" = String, Path, description = "Eval run ID")
    ),
    responses(
        (status = 200, description = "NDJSON artifacts", body = String, content_type = "application/x-ndjson"),
        (status = 404, description = "Eval run not found", body = ErrorResponse)
    ),
    tag = "evals"
)]
pub async fn export_run_artifacts(
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
#[utoipa::path(
    post,
    path = "/v1/evals/{eval_id}/runs/{run_id}/dataset",
    summary = "Start an async dataset export for an eval run.",
    params(
        ("eval_id" = String, Path, description = "Eval ID"),
        ("run_id" = String, Path, description = "Eval run ID")
    ),
    request_body = ExportEvalRunDatasetRequest,
    responses(
        (status = 202, description = "Export enqueued", body = EvalRunDataset),
        (status = 404, description = "Eval run not found", body = ErrorResponse)
    ),
    tag = "evals"
)]
pub async fn export_run_dataset(
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
    use everruns_contracts::typed_id::EvalCaseId;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    #[test]
    fn run_artifact_export_maps_patch_to_model_patch() {
        let result = EvalCaseResult {
            public_id: everruns_contracts::typed_id::EvalResultId::from_uuid(Uuid::now_v7()),
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
            public_id: everruns_contracts::typed_id::EvalResultId::from_uuid(Uuid::now_v7()),
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
