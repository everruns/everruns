// Eval service for business logic
// Decision: Evals reuse the same permission policies as agents (OrgAgentsManage)
// Decision: Each eval case creates a real session — no mock execution
// Decision: EvalTarget replaces harness_id + agent_id. Resolution: run → case → eval → org default.

use crate::api::evals::{
    BulkUpdateEvalRunScoresRequest, CreateEvalCaseRequest, CreateEvalRequest, CreateEvalRunRequest,
    EvalRunShareLink, ExternalScoreStatus, ImportEvalCaseEntry, ImportEvalRunRequest,
    PublicAttribution, PublicEvalCaseResult, PublicEvalRun, UpdateEvalCaseRequest,
    UpdateEvalRequest, UpdateEvalResultScoresRequest,
};
use crate::auth::share_token::{SHARE_PREFIX, generate_share_token, hash_share_token};
use crate::domains::evals::limits::EvalLimits;
use crate::domains::evals::runner::{EvalRunContext, spawn_eval_run};
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateEvalCaseRow, CreateEvalRow, CreateEvalRunError, CreateEvalRunRow,
    CreateEvalRunShareTokenRow, ImportEvalCaseInput, ImportEvalRunInput, UpdateEvalCaseResultRow,
    UpdateEvalCaseRow, UpdateEvalRow,
};
use anyhow::Result;
use everruns_core::{Caller, Permission, Policy, Rule};
use everruns_platform::eval::*;
use everruns_provider::typed_id::{
    EvalCaseId, EvalDatasetId, EvalId, EvalResultId, EvalRunId, SessionId,
};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
use url::Url;
use uuid::Uuid;

/// Policy: View evals (read-only).
pub const EVAL_VIEW: Policy = Policy {
    id: "eval.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Manage evals (create, update, metadata operations).
pub const EVAL_MANAGE: Policy = Policy {
    id: "eval.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Start eval runs.
///
/// Runs create real sessions in the background eval runner, so require both
/// eval-management and session-management permissions.
pub const EVAL_RUN: Policy = Policy {
    id: "eval.run",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgSessionsManage),
    ],
};

/// Policy: Export reward-labeled trajectory datasets from eval runs.
///
/// Distinct from `EVAL_VIEW`/`REPORT_VIEW` because this surface exports raw
/// model-view message content (more sensitive than aggregate reporting). Gated
/// more tightly than read-only eval viewing by requiring both agent- and
/// session-management permissions, matching the privilege of starting runs.
pub const DATASET_EXPORT: Policy = Policy {
    id: "dataset.export",
    rules: &[
        Rule::UserHasPermission(Permission::OrgAgentsManage),
        Rule::UserHasPermission(Permission::OrgSessionsManage),
    ],
};

/// Policy: Import externally-executed eval runs (everruns as host/viewer).
///
/// Unlike `EVAL_RUN`, importing creates no sessions — the run is ingested
/// already-complete — so it requires only eval-management, not session
/// management. See knowledge/evaluation/evals.md.
pub const EVAL_IMPORT: Policy = Policy {
    id: "eval.import",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

pub struct EvalService {
    db: Arc<StorageBackend>,
    run_context: Option<Arc<EvalRunContext>>,
    limits: EvalLimits,
}

/// Validates an EvalTarget if present. Returns error on invalid combinations.
fn validate_target(target: &Option<EvalTarget>) -> Result<()> {
    if let Some(EvalTarget::Session {
        harness_id,
        harness_name,
        ..
    }) = target
        && harness_id.is_some()
        && harness_name.is_some()
    {
        anyhow::bail!("harness_id and harness_name are mutually exclusive in eval target");
    }
    Ok(())
}

impl EvalService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self {
            db,
            run_context: None,
            limits: EvalLimits::from_env(),
        }
    }

    pub fn with_limits(mut self, limits: EvalLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_run_context(mut self, ctx: Arc<EvalRunContext>) -> Self {
        self.run_context = Some(ctx);
        self
    }

    // ============================================
    // Eval CRUD
    // ============================================

    pub async fn create(&self, caller: &Caller, req: CreateEvalRequest) -> Result<Eval> {
        validate_target(&req.target)?;

        let internal_uuid = Uuid::now_v7();
        let public_id = EvalId::from_uuid(internal_uuid);

        let target_json = req.target.as_ref().map(serde_json::to_value).transpose()?;

        let input = CreateEvalRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            target: target_json,
            model_override: req.model_override,
            tags: req.tags.unwrap_or_default(),
        };

        let row = self.db.create_eval(caller.org_id, input).await?;
        self.row_to_eval(row).await
    }

    pub async fn get_by_public_id(&self, caller: &Caller, public_id: &str) -> Result<Option<Eval>> {
        let row = self
            .db
            .get_eval_by_public_id(caller.org_id, public_id)
            .await?;
        match row {
            Some(row) if row.status != "deleted" => Ok(Some(self.row_to_eval(row).await?)),
            _ => Ok(None),
        }
    }

    pub async fn list(
        &self,
        caller: &Caller,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<Eval>> {
        let rows = self
            .db
            .list_evals(caller.org_id, search, include_archived)
            .await?;
        let mut evals = Vec::with_capacity(rows.len());
        for row in rows {
            evals.push(self.row_to_eval(row).await?);
        }
        Ok(evals)
    }

    pub async fn update(
        &self,
        caller: &Caller,
        public_id: &str,
        req: UpdateEvalRequest,
    ) -> Result<Option<Eval>> {
        validate_target(&req.target)?;

        let existing = self
            .db
            .get_eval_by_public_id(caller.org_id, public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let target_json = req.target.as_ref().map(serde_json::to_value).transpose()?;

        let input = UpdateEvalRow {
            name: req.name,
            description: req.description,
            target: target_json,
            model_override: req.model_override,
            tags: req.tags,
            status: None,
        };

        let row = self
            .db
            .update_eval(caller.org_id, existing.id, input)
            .await?;
        match row {
            Some(row) => Ok(Some(self.row_to_eval(row).await?)),
            None => Ok(None),
        }
    }

    pub async fn delete(&self, caller: &Caller, public_id: &str) -> Result<bool> {
        let existing = self
            .db
            .get_eval_by_public_id(caller.org_id, public_id)
            .await?;
        let Some(existing) = existing else {
            return Ok(false);
        };
        self.db.delete_eval(caller.org_id, existing.id).await
    }

    // ============================================
    // Eval Case CRUD
    // ============================================

    pub async fn create_case(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        req: CreateEvalCaseRequest,
    ) -> Result<EvalCase> {
        validate_target(&req.target)?;

        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let case_uuid = Uuid::now_v7();
        let case_public_id = EvalCaseId::from_uuid(case_uuid);

        let case_count = self.db.count_eval_cases(eval.id).await?;

        let target_json = req.target.as_ref().map(serde_json::to_value).transpose()?;

        let post_json = req.post.as_ref().map(serde_json::to_value).transpose()?;
        let artifacts_json = req
            .artifacts
            .as_ref()
            .map(serde_json::to_value)
            .transpose()?;

        let input = CreateEvalCaseRow {
            public_id: case_public_id.to_string(),
            name: req.name,
            description: req.description,
            target: target_json,
            tags: req.tags.unwrap_or_default(),
            conversation: serde_json::to_value(&req.conversation)?,
            post: post_json,
            artifacts: artifacts_json,
            scorers: serde_json::to_value(&req.scorers)?,
            max_turns: req.max_turns.map(|v| v as i32),
            timeout_seconds: req.timeout_seconds.map(|v| v as i32),
            position: req.position.unwrap_or(case_count as i32),
        };

        let row = self.db.create_eval_case(eval.id, input).await?;
        Ok(case_row_to_case(row))
    }

    pub async fn list_cases(&self, caller: &Caller, eval_public_id: &str) -> Result<Vec<EvalCase>> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let rows = self.db.list_eval_cases(eval.id).await?;
        Ok(rows.into_iter().map(case_row_to_case).collect())
    }

    pub async fn get_case(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        case_public_id: &str,
    ) -> Result<Option<EvalCase>> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let row = self
            .db
            .get_eval_case_by_public_id(eval.id, case_public_id)
            .await?;
        Ok(row.map(case_row_to_case))
    }

    pub async fn update_case(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        case_public_id: &str,
        req: UpdateEvalCaseRequest,
    ) -> Result<Option<EvalCase>> {
        validate_target(&req.target)?;

        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let case = self
            .db
            .get_eval_case_by_public_id(eval.id, case_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("EvalCase"))?;

        let target_json = req.target.as_ref().map(serde_json::to_value).transpose()?;

        let post_json = req.post.map(serde_json::to_value).transpose()?;
        let artifacts_json = req.artifacts.map(serde_json::to_value).transpose()?;

        let input = UpdateEvalCaseRow {
            name: req.name,
            description: req.description,
            target: target_json,
            tags: req.tags,
            conversation: req.conversation.map(serde_json::to_value).transpose()?,
            post: post_json,
            artifacts: artifacts_json,
            scorers: req.scorers.map(serde_json::to_value).transpose()?,
            max_turns: req.max_turns.map(|v| v as i32),
            timeout_seconds: req.timeout_seconds.map(|v| v as i32),
            position: req.position,
        };

        let row = self.db.update_eval_case(case.id, input).await?;
        Ok(row.map(case_row_to_case))
    }

    pub async fn delete_case(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        case_public_id: &str,
    ) -> Result<bool> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let case = self
            .db
            .get_eval_case_by_public_id(eval.id, case_public_id)
            .await?;
        let Some(case) = case else {
            return Ok(false);
        };
        self.db.delete_eval_case(case.id).await
    }

    // ============================================
    // Eval Run
    // ============================================

    pub async fn create_run(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        req: CreateEvalRunRequest,
    ) -> Result<EvalRun> {
        validate_target(&req.target)?;

        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let run_uuid = Uuid::now_v7();
        let run_public_id = EvalRunId::from_uuid(run_uuid);

        let target_json = req.target.as_ref().map(serde_json::to_value).transpose()?;

        let input = CreateEvalRunRow {
            public_id: run_public_id.to_string(),
            eval_id: eval.id,
            target: target_json,
            model_override: req.model_override,
            filter_tags: None, // Phase 2: tag-based partial runs
            triggered_by: "user".to_string(),
        };

        // Load eval-level target for resolution
        let eval_target: Option<EvalTarget> = eval
            .target
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        // Run quota checks, case snapshot selection, run insertion, and result
        // insertion must share one storage critical section. Splitting these
        // steps lets concurrent POST /runs calls all pass the active-run count
        // before any pending run is inserted.
        let run_row = self
            .db
            .create_eval_run_with_case_results(
                caller.org_id,
                input,
                eval_target.map(serde_json::to_value).transpose()?,
                self.limits.max_concurrent_runs_per_org,
                self.limits.max_cases_per_run,
            )
            .await
            .map_err(|err| {
                // Map the storage layer's typed quota/validation failures to a
                // 400 by downcasting the concrete error rather than matching on
                // message text (which would silently misclassify on rewording).
                if err.downcast_ref::<CreateEvalRunError>().is_some() {
                    BadRequestError::new(err.to_string()).into()
                } else {
                    err
                }
            })?;

        // Dispatch background execution if run context is available
        if let Some(run_ctx) = &self.run_context {
            spawn_eval_run(run_ctx.clone(), caller.org_id, run_row.id);
        }

        Ok(run_row_to_run(run_row, vec![]))
    }

    pub async fn list_runs(&self, caller: &Caller, eval_public_id: &str) -> Result<Vec<EvalRun>> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let rows = self.db.list_eval_runs(eval.id).await?;
        Ok(rows
            .into_iter()
            .map(|r| run_row_to_run(r, vec![]))
            .collect())
    }

    pub async fn get_run(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<Option<EvalRun>> {
        // Verify eval ownership and run belongs to eval
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let run_row = self
            .db
            .get_eval_run_by_public_id(caller.org_id, run_public_id)
            .await?;
        let Some(run_row) = run_row else {
            return Ok(None);
        };
        if run_row.eval_id != eval.id {
            return Ok(None);
        }

        // Load case results with case names
        let result_rows = self.db.list_eval_case_results(run_row.id).await?;
        let cases = self.db.list_eval_cases(run_row.eval_id).await?;

        let results: Vec<EvalCaseResult> = result_rows
            .into_iter()
            .map(|r| {
                let case_name = cases
                    .iter()
                    .find(|c| c.id == r.eval_case_id)
                    .map(|c| c.name.clone());
                result_row_to_result(r, case_name)
            })
            .collect();

        Ok(Some(run_row_to_run(run_row, results)))
    }

    /// Enqueue an async dataset export for a completed run. Persists a `pending`
    /// dataset handle, spawns the background export, and returns the handle.
    /// Org scope is enforced by `get_run` (resolves only the caller's runs).
    pub async fn create_dataset_export(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
        req: crate::domains::evals::dataset::ExportEvalRunDatasetRequest,
    ) -> Result<Option<EvalRunDataset>> {
        let Some(run) = self.get_run(caller, eval_public_id, run_public_id).await? else {
            return Ok(None);
        };

        if run.status != EvalRunStatus::Completed {
            return Err(
                BadRequestError::new("Dataset export requires a completed eval run").into(),
            );
        }

        let request_json = serde_json::to_value(&req)?;
        if let Some(row) = self
            .db
            .find_eval_run_dataset_by_request(caller.org_id, run.internal_id, &request_json)
            .await?
        {
            return Ok(Some(dataset_row_to_dataset(row, false)));
        }

        // THREAT[TM-DOS-033]: reject excess new work before creating durable
        // rows; the permit lives for the complete background export.
        let Some(permit) = crate::domains::evals::dataset_export::try_acquire_export_permit()
        else {
            return Err(BadRequestError::new(
                "Too many dataset exports are already running; retry later",
            )
            .into());
        };

        let public_id = everruns_provider::typed_id::EvalDatasetId::from_uuid(Uuid::now_v7());
        let (row, created) = self
            .db
            .create_eval_run_dataset(
                caller.org_id,
                crate::storage::models::CreateEvalRunDatasetRow {
                    public_id: public_id.to_string(),
                    eval_run_id: run.internal_id,
                    request: request_json,
                },
            )
            .await?;

        if created {
            crate::domains::evals::dataset_export::spawn_dataset_export(
                self.db.clone(),
                row.id,
                run,
                req,
                permit,
            );
        }

        Ok(Some(dataset_row_to_dataset(row, false)))
    }

    /// Fetch a dataset export handle by id (status + NDJSON body when complete).
    /// Org-scoped: only datasets owned by the caller's org resolve.
    pub async fn get_dataset(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
        dataset_public_id: &str,
    ) -> Result<Option<EvalRunDataset>> {
        // Resolve the run first so a dataset from another run/org is never
        // reachable even if its id is guessed.
        let Some(run) = self.get_run(caller, eval_public_id, run_public_id).await? else {
            return Ok(None);
        };

        let Some(row) = self
            .db
            .get_eval_run_dataset(caller.org_id, dataset_public_id)
            .await?
        else {
            return Ok(None);
        };
        if row.eval_run_id != Some(run.internal_id) {
            return Ok(None);
        }

        Ok(Some(dataset_row_to_dataset(row, true)))
    }

    pub async fn cancel_run(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<Option<EvalRun>> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let run_row = self
            .db
            .get_eval_run_by_public_id(caller.org_id, run_public_id)
            .await?;
        let Some(run_row) = run_row else {
            return Ok(None);
        };
        if run_row.eval_id != eval.id {
            return Ok(None);
        }

        if !matches!(run_row.status.as_str(), "pending" | "running") {
            return Err(
                BadRequestError::new("can only cancel pending or running eval runs").into(),
            );
        }

        let updated = self
            .db
            .update_eval_run_status(run_row.id, "cancelled", None)
            .await?;
        Ok(updated.map(|r| run_row_to_run(r, vec![])))
    }

    pub async fn update_result_scores(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
        result_public_id: &str,
        req: UpdateEvalResultScoresRequest,
    ) -> Result<Option<EvalCaseResult>> {
        validate_scores_payload(&req.scores)?;
        validate_metadata_payload(req.metadata.as_ref())?;

        let (run_row, case_rows, mut result_rows) = self
            .load_mutable_run_context(caller, eval_public_id, run_public_id)
            .await?;
        let case_rows_by_id: HashMap<Uuid, &crate::storage::models::EvalCaseRow> =
            case_rows.iter().map(|case| (case.id, case)).collect();

        let Some(existing_index) = result_rows
            .iter()
            .position(|result| result.public_id == result_public_id)
        else {
            return Ok(None);
        };
        let existing = result_rows[existing_index].clone();
        validate_score_count(
            case_rows_by_id.get(&existing.eval_case_id).copied(),
            &req.scores,
        )?;

        let updated = self
            .db
            .update_eval_case_result(
                existing.id,
                UpdateEvalCaseResultRow {
                    status: Some(
                        resolve_external_score_status(&req.scores, req.status).to_string(),
                    ),
                    scores: Some(serde_json::to_value(&req.scores)?),
                    metadata: req.metadata.clone(),
                    ..Default::default()
                },
            )
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("EvalCaseResult"))?;

        result_rows[existing_index] = updated.clone();
        self.persist_run_summary(run_row.id, &case_rows, &result_rows)
            .await?;

        let case_name = case_rows_by_id
            .get(&updated.eval_case_id)
            .map(|case| case.name.clone());
        Ok(Some(result_row_to_result(updated, case_name)))
    }

    pub async fn bulk_update_run_scores(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
        req: BulkUpdateEvalRunScoresRequest,
    ) -> Result<Vec<EvalCaseResult>> {
        if req.results.is_empty() {
            return Err(BadRequestError::new("must provide at least one result update").into());
        }
        validate_metadata_payload(req.metadata.as_ref())?;

        let (run_row, case_rows, mut result_rows) = self
            .load_mutable_run_context(caller, eval_public_id, run_public_id)
            .await?;
        let shared_metadata = req.metadata.clone();
        let case_rows_by_id: HashMap<Uuid, &crate::storage::models::EvalCaseRow> =
            case_rows.iter().map(|case| (case.id, case)).collect();
        let result_positions: HashMap<String, usize> = result_rows
            .iter()
            .enumerate()
            .map(|(index, result)| (result.public_id.clone(), index))
            .collect();
        let mut seen_result_ids = HashSet::new();

        for update in &req.results {
            validate_scores_payload(&update.scores)?;
            let result_public_id = update.result_id.to_string();
            if !seen_result_ids.insert(result_public_id.clone()) {
                return Err(BadRequestError::new(format!(
                    "duplicate result_id in bulk request: {result_public_id}"
                ))
                .into());
            }
            let Some(existing_index) = result_positions.get(&result_public_id).copied() else {
                return Err(ResourceNotFoundError::new("EvalCaseResult").into());
            };
            validate_score_count(
                case_rows_by_id
                    .get(&result_rows[existing_index].eval_case_id)
                    .copied(),
                &update.scores,
            )?;
        }

        let mut updated_results = Vec::with_capacity(req.results.len());
        for update in req.results {
            let result_public_id = update.result_id.to_string();
            let existing_index = result_positions
                .get(&result_public_id)
                .copied()
                .ok_or_else(|| ResourceNotFoundError::new("EvalCaseResult"))?;
            let existing = result_rows[existing_index].clone();
            let updated = self
                .db
                .update_eval_case_result(
                    existing.id,
                    UpdateEvalCaseResultRow {
                        status: Some(
                            resolve_external_score_status(&update.scores, update.status)
                                .to_string(),
                        ),
                        scores: Some(serde_json::to_value(&update.scores)?),
                        metadata: shared_metadata.clone(),
                        ..Default::default()
                    },
                )
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("EvalCaseResult"))?;

            result_rows[existing_index] = updated.clone();
            updated_results.push(updated);
        }

        self.persist_run_summary(run_row.id, &case_rows, &result_rows)
            .await?;

        Ok(updated_results
            .into_iter()
            .map(|result| {
                let case_name = case_rows_by_id
                    .get(&result.eval_case_id)
                    .map(|case| case.name.clone());
                result_row_to_result(result, case_name)
            })
            .collect())
    }

    // ============================================
    // Import (external eval results)
    // ============================================

    /// Ingest a full external run group (one external run → one EvalRun per
    /// eval, sharing `source.run_id`). Upserts evals/cases by name and stores
    /// fully-scored, completed results. Everruns trusts the external verdicts;
    /// it never re-grades. See knowledge/evaluation/evals.md.
    pub async fn import_run(
        &self,
        caller: &Caller,
        req: ImportEvalRunRequest,
    ) -> Result<Vec<EvalRun>> {
        if req.evals.is_empty() {
            return Err(BadRequestError::new("import must include at least one eval").into());
        }
        // THREAT[TM-DOS-025]: Reject import fan-out before allocating result vectors or
        // opening per-eval storage transactions.
        if req.evals.len() > self.limits.max_evals_per_import {
            return Err(BadRequestError::new(format!(
                "import exceeds the limit of {} evals",
                self.limits.max_evals_per_import
            ))
            .into());
        }
        if let Some(group) = req
            .evals
            .iter()
            .find(|group| group.cases.len() > self.limits.max_cases_per_run)
        {
            return Err(BadRequestError::new(format!(
                "eval '{}' exceeds the per-run limit of {} cases",
                group.name, self.limits.max_cases_per_run
            ))
            .into());
        }
        if req.source.system.trim().is_empty() {
            return Err(BadRequestError::new("source.system is required").into());
        }
        if req.source.run_id.trim().is_empty() {
            return Err(BadRequestError::new("source.run_id is required").into());
        }
        let source_url = validate_import_source_url(req.source.url)?;

        let attribution = serde_json::json!({
            "system": req.source.system,
            "version": req.source.version,
            "url": source_url,
            "run_id": req.source.run_id,
            "metadata": req.source.metadata,
        });
        let triggered_by = format!("import:{}", req.source.system);

        let mut runs = Vec::with_capacity(req.evals.len());
        for group in req.evals {
            if group.name.trim().is_empty() {
                return Err(BadRequestError::new("eval name is required").into());
            }

            let mut import_cases = Vec::with_capacity(group.cases.len());
            let mut acc = ImportSummaryAcc::default();
            for case in group.cases {
                if case.name.trim().is_empty() {
                    return Err(BadRequestError::new("case name is required").into());
                }
                for score in &case.scores {
                    if !score.value.is_finite() || !(0.0..=1.0).contains(&score.value) {
                        return Err(BadRequestError::new(format!(
                            "score value for scorer '{}' must be between 0.0 and 1.0",
                            score.scorer
                        ))
                        .into());
                    }
                }

                let status = case.status.as_str();
                acc.add(status, &case);

                let target = EvalTarget::External {
                    provider: case.target.provider.clone(),
                    model: case.target.model.clone(),
                    params: case.target.params.clone(),
                };
                let conversation = serde_json::to_value(
                    case.input
                        .iter()
                        .map(|content| EvalInputMessage {
                            content: content.clone(),
                        })
                        .collect::<Vec<_>>(),
                )?;
                // Per-result transcript + open-vocab metrics ride in the
                // metadata envelope rather than dedicated columns.
                let mut envelope = serde_json::Map::new();
                if let Some(t) = &case.transcript {
                    envelope.insert("transcript".to_string(), t.clone());
                }
                if let Some(m) = &case.metrics {
                    envelope.insert("metrics".to_string(), m.clone());
                }
                let metadata =
                    (!envelope.is_empty()).then_some(serde_json::Value::Object(envelope));
                let scores = (!case.scores.is_empty())
                    .then(|| serde_json::to_value(&case.scores))
                    .transpose()?;

                import_cases.push(ImportEvalCaseInput {
                    case_name: case.name,
                    case_description: case.description,
                    conversation,
                    target_snapshot: Some(serde_json::to_value(&target)?),
                    status: status.to_string(),
                    scores,
                    metadata,
                    turns: case.turns.map(|v| v as i32),
                    latency_ms: case.latency_ms.map(|v| v as i64),
                    input_tokens: case.input_tokens.map(|v| v as i64),
                    output_tokens: case.output_tokens.map(|v| v as i64),
                    error_message: case.error_message,
                    artifacts: None,
                });
            }

            let input = ImportEvalRunInput {
                eval_name: group.name,
                eval_description: group.description,
                eval_tags: group.tags,
                run_public_id: EvalRunId::from_uuid(Uuid::now_v7()).to_string(),
                source: "external".to_string(),
                source_run_id: req.source.run_id.clone(),
                attribution: Some(attribution.clone()),
                triggered_by: triggered_by.clone(),
                summary: Some(serde_json::to_value(acc.finish())?),
                cases: import_cases,
            };

            let run_row = self.db.import_eval_run(caller.org_id, input).await?;
            runs.push(run_row_to_run(run_row, vec![]));
        }

        Ok(runs)
    }

    // ============================================
    // Import (ATIF trajectories → eval cases)
    // ============================================

    /// Create/update eval cases from ATIF trajectories (knowledge/evaluation/atif-adoption.md).
    ///
    /// Idempotent: the case `name` (derived from the trajectory's
    /// `extra.case_name`/`source_key`/ids) is the natural key — re-importing
    /// the same trajectories converges instead of duplicating. Org-scoped via
    /// the eval lookup inside `create_case`/`update_case`.
    pub async fn import_atif_cases(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        drafts: Vec<crate::atif::AtifCaseDraft>,
    ) -> Result<crate::api::evals::AtifImportReport> {
        // Existing case names within the target eval (also 404s cross-org ids).
        let mut by_name: HashMap<String, String> = self
            .list_cases(caller, eval_public_id)
            .await?
            .into_iter()
            .map(|c| (c.name.clone(), c.public_id.to_string()))
            .collect();

        let mut created = 0u64;
        let mut updated = 0u64;
        let mut case_ids = Vec::with_capacity(drafts.len());
        for draft in drafts {
            if let Some(case_id) = by_name.get(&draft.name).cloned() {
                let req = UpdateEvalCaseRequest {
                    name: None,
                    description: Some(draft.description),
                    target: None,
                    tags: None,
                    conversation: Some(draft.conversation),
                    post: None,
                    artifacts: None,
                    scorers: None,
                    max_turns: None,
                    timeout_seconds: None,
                    position: None,
                };
                let case = self
                    .update_case(caller, eval_public_id, &case_id, req)
                    .await?
                    .ok_or_else(|| ResourceNotFoundError::new("EvalCase"))?;
                updated += 1;
                case_ids.push(case.public_id.to_string());
            } else {
                let name = draft.name.clone();
                let req = CreateEvalCaseRequest {
                    name: draft.name,
                    description: Some(draft.description),
                    target: None,
                    tags: Some(vec!["atif-import".to_string()]),
                    conversation: draft.conversation,
                    post: None,
                    artifacts: None,
                    // ATIF carries no assertion semantics; imported cases start
                    // unscored and users attach scorers afterwards.
                    scorers: vec![],
                    max_turns: None,
                    timeout_seconds: None,
                    position: None,
                };
                let case = self.create_case(caller, eval_public_id, req).await?;
                created += 1;
                let case_id = case.public_id.to_string();
                // Track the new name so duplicate names within one import
                // batch update instead of creating twins.
                by_name.insert(name, case_id.clone());
                case_ids.push(case_id);
            }
        }

        Ok(crate::api::evals::AtifImportReport {
            created,
            updated,
            case_ids,
        })
    }

    // ============================================
    // Share links (read-only public views)
    // ============================================

    /// Mint a read-only share link for a run. Revokes any prior active link so a
    /// run has at most one live share. The raw token is returned once and stored
    /// only hashed. See knowledge/evaluation/evals.md, knowledge/execution/public-endpoints.md.
    pub async fn create_run_share(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<EvalRunShareLink> {
        let run_row = self
            .load_run_owned(caller, eval_public_id, run_public_id)
            .await?;

        // One active link per run: revoke the old before minting the new.
        self.db
            .revoke_eval_run_share_tokens(caller.org_id, run_row.id)
            .await?;

        let generated = generate_share_token();
        let public_id = format!("evalshare_{:032x}", Uuid::now_v7().as_u128());
        let row = self
            .db
            .create_eval_run_share_token(
                caller.org_id,
                CreateEvalRunShareTokenRow {
                    public_id,
                    org_id: caller.org_id,
                    eval_run_id: run_row.id,
                    token_hash: generated.token_hash,
                    token_prefix: generated.token_prefix,
                    created_by: None,
                    expires_at: None,
                },
            )
            .await?;

        Ok(EvalRunShareLink {
            token: generated.token,
            token_prefix: row.token_prefix,
            created_at: row.created_at,
        })
    }

    /// Revoke every active share link for a run. Returns whether any were live.
    pub async fn revoke_run_share(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<bool> {
        let run_row = self
            .load_run_owned(caller, eval_public_id, run_public_id)
            .await?;
        let revoked = self
            .db
            .revoke_eval_run_share_tokens(caller.org_id, run_row.id)
            .await?;
        Ok(revoked > 0)
    }

    /// Whether a run currently has an active share link (for the UI).
    pub async fn run_has_active_share(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<bool> {
        let run_row = self
            .load_run_owned(caller, eval_public_id, run_public_id)
            .await?;
        self.db
            .eval_run_has_active_share(caller.org_id, run_row.id)
            .await
    }

    /// Resolve a share token to a sanitized, anonymous view of one run. The token
    /// IS the authorization — no caller/org. Returns `None` for unknown, revoked,
    /// or expired tokens so the handler answers a uniform 404 (no oracle).
    pub async fn resolve_public_share(&self, token: &str) -> Result<Option<PublicEvalRun>> {
        if !token.starts_with(SHARE_PREFIX) {
            return Ok(None);
        }
        let hash = hash_share_token(token);
        let Some(share) = self.db.get_eval_run_share_token_by_hash(&hash).await? else {
            return Ok(None);
        };
        if share.revoked_at.is_some() {
            return Ok(None);
        }
        if let Some(exp) = share.expires_at
            && exp <= chrono::Utc::now()
        {
            return Ok(None);
        }
        let Some(run_row) = self.db.get_eval_run_by_id(share.eval_run_id).await? else {
            return Ok(None);
        };
        let result_rows = self.db.list_eval_case_results(run_row.id).await?;
        let cases = self.db.list_eval_cases(run_row.eval_id).await?;
        Ok(Some(build_public_run(run_row, result_rows, &cases)))
    }

    /// Load a run verifying it belongs to the caller's org and the given eval.
    async fn load_run_owned(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<crate::storage::models::EvalRunRow> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;
        let run_row = self
            .db
            .get_eval_run_by_public_id(caller.org_id, run_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("EvalRun"))?;
        if run_row.eval_id != eval.id {
            return Err(ResourceNotFoundError::new("EvalRun").into());
        }
        Ok(run_row)
    }

    // ============================================
    // Helpers
    // ============================================

    async fn load_mutable_run_context(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<(
        crate::storage::models::EvalRunRow,
        Vec<crate::storage::models::EvalCaseRow>,
        Vec<crate::storage::models::EvalCaseResultRow>,
    )> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let run_row = self
            .db
            .get_eval_run_by_public_id(caller.org_id, run_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("EvalRun"))?;
        if run_row.eval_id != eval.id {
            return Err(ResourceNotFoundError::new("EvalRun").into());
        }
        if run_row.status != "completed" {
            return Err(
                BadRequestError::new("can only write scores for completed eval runs").into(),
            );
        }

        let case_rows = self.db.list_eval_cases(run_row.eval_id).await?;
        let result_rows = self.db.list_eval_case_results(run_row.id).await?;
        Ok((run_row, case_rows, result_rows))
    }

    async fn persist_run_summary(
        &self,
        run_id: Uuid,
        case_rows: &[crate::storage::models::EvalCaseRow],
        result_rows: &[crate::storage::models::EvalCaseResultRow],
    ) -> Result<()> {
        let summary = build_run_summary(case_rows, result_rows);
        self.db
            .update_eval_run_status(run_id, "completed", Some(serde_json::to_value(summary)?))
            .await?;
        Ok(())
    }

    async fn row_to_eval(&self, row: crate::storage::models::EvalRow) -> Result<Eval> {
        let case_count = self.db.count_eval_cases(row.id).await?;
        let last_run = self.db.get_latest_eval_run(row.id).await?;

        let last_run_view = last_run.map(|r| EvalRunSummaryView {
            public_id: r
                .public_id
                .parse()
                .unwrap_or_else(|_| EvalRunId::from_uuid(r.id)),
            status: EvalRunStatus::from(r.status.as_str()),
            summary: r.summary.and_then(|s| serde_json::from_value(s).ok()),
            created_at: r.created_at,
        });

        let target: Option<EvalTarget> = row
            .target
            .as_ref()
            .and_then(|v| serde_json::from_value(v.clone()).ok());

        let public_id: EvalId = row
            .public_id
            .parse()
            .unwrap_or_else(|_| EvalId::from_uuid(row.id));

        Ok(Eval {
            public_id,
            internal_id: row.id,
            org_id: row.org_id,
            name: row.name,
            description: row.description,
            target,
            model_override: row.model_override,
            tags: row.tags,
            status: EvalStatus::from(row.status.as_str()),
            case_count,
            last_run: last_run_view,
            created_at: row.created_at,
            updated_at: row.updated_at,
            archived_at: row.archived_at,
            deleted_at: row.deleted_at,
        })
    }
}

fn case_row_to_case(row: crate::storage::models::EvalCaseRow) -> EvalCase {
    let target: Option<EvalTarget> = row
        .target
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    EvalCase {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| EvalCaseId::from_uuid(row.id)),
        internal_id: row.id,
        name: row.name,
        description: row.description,
        target,
        tags: row.tags,
        conversation: serde_json::from_value(row.conversation).unwrap_or_default(),
        post: row.post.and_then(|v| serde_json::from_value(v).ok()),
        artifacts: row.artifacts.and_then(|v| serde_json::from_value(v).ok()),
        scorers: serde_json::from_value(row.scorers).unwrap_or_default(),
        max_turns: row.max_turns.map(|v| v as u32),
        timeout_seconds: row.timeout_seconds.map(|v| v as u32),
        position: row.position,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn run_row_to_run(
    row: crate::storage::models::EvalRunRow,
    results: Vec<EvalCaseResult>,
) -> EvalRun {
    let target: Option<EvalTarget> = row
        .target
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    EvalRun {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| EvalRunId::from_uuid(row.id)),
        internal_id: row.id,
        org_id: row.org_id,
        target,
        model_override: row.model_override,
        filter_tags: row.filter_tags,
        status: EvalRunStatus::from(row.status.as_str()),
        source: EvalRunSource::from(row.source.as_str()),
        attribution: row.attribution,
        triggered_by: row.triggered_by,
        started_at: row.started_at,
        completed_at: row.completed_at,
        summary: row.summary.and_then(|s| serde_json::from_value(s).ok()),
        results,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// Map a dataset row to the API type. `include_body` controls whether the
/// (potentially large) NDJSON is attached — list/enqueue views omit it, the
/// detail view includes it.
fn dataset_row_to_dataset(
    row: crate::storage::models::EvalRunDatasetRow,
    include_body: bool,
) -> EvalRunDataset {
    EvalRunDataset {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| EvalDatasetId::from_uuid(row.id)),
        // A dataset always references a run at creation; fall back to the row id
        // only if the FK was cleared by run deletion (ON DELETE SET NULL).
        eval_run_id: row
            .eval_run_id
            .map(EvalRunId::from_uuid)
            .unwrap_or_else(|| EvalRunId::from_uuid(row.id)),
        status: EvalDatasetStatus::from(row.status.as_str()),
        record_count: row.record_count.map(|c| c.max(0) as u64),
        error_message: row.error_message,
        body: if include_body { row.body } else { None },
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

// ============================================
// Public (share-link) sanitization
// ============================================

/// Build the sanitized public view of a run. Strips everything an anonymous
/// viewer must not see: org/internal ids, session ids, internal (session/app)
/// targets, and attribution env/labels. Keeps the shared content: statuses,
/// scores, external target labels, and the normalized transcript/metrics.
fn build_public_run(
    run: crate::storage::models::EvalRunRow,
    result_rows: Vec<crate::storage::models::EvalCaseResultRow>,
    cases: &[crate::storage::models::EvalCaseRow],
) -> PublicEvalRun {
    let results = result_rows
        .into_iter()
        .map(|r| {
            let case_name = cases
                .iter()
                .find(|c| c.id == r.eval_case_id)
                .map(|c| c.name.clone());
            public_result_row(r, case_name)
        })
        .collect();

    PublicEvalRun {
        id: run.public_id,
        status: EvalRunStatus::from(run.status.as_str()),
        source: EvalRunSource::from(run.source.as_str()),
        attribution: run.attribution.and_then(sanitize_attribution),
        summary: run.summary.and_then(|s| serde_json::from_value(s).ok()),
        created_at: run.created_at,
        completed_at: run.completed_at,
        results,
    }
}

fn public_result_row(
    row: crate::storage::models::EvalCaseResultRow,
    case_name: Option<String>,
) -> PublicEvalCaseResult {
    // Only expose label-only (External) targets. Session/App targets carry
    // internal harness/agent/app ids and are dropped for the public view.
    let target = row
        .target_snapshot
        .as_ref()
        .and_then(|v| serde_json::from_value::<EvalTarget>(v.clone()).ok())
        .filter(|t| matches!(t, EvalTarget::External { .. }));
    // Transcript + metrics ride in the metadata envelope; expose only those two
    // keys, never the raw envelope (which may carry other provenance).
    let (transcript, metrics) = match &row.metadata {
        Some(serde_json::Value::Object(m)) => {
            (m.get("transcript").cloned(), m.get("metrics").cloned())
        }
        _ => (None, None),
    };

    PublicEvalCaseResult {
        case_name,
        target,
        status: CaseResultStatus::from(row.status.as_str()),
        scores: row.scores,
        transcript,
        metrics,
        turns: row.turns.map(|v| v as u32),
        latency_ms: row.latency_ms.map(|v| v as u64),
        input_tokens: row.input_tokens.map(|v| v as u64),
        output_tokens: row.output_tokens.map(|v| v as u64),
        error_message: row.error_message,
    }
}

/// Keep only the display fields of an external run's attribution (system,
/// version, url). Drops `run_id` and the `metadata` bag (git/host/env labels).
fn sanitize_attribution(v: serde_json::Value) -> Option<PublicAttribution> {
    let obj = v.as_object()?;
    let str_field = |k: &str| obj.get(k).and_then(|s| s.as_str()).map(String::from);
    let att = PublicAttribution {
        system: str_field("system"),
        version: str_field("version"),
        url: str_field("url"),
    };
    if att.system.is_none() && att.version.is_none() && att.url.is_none() {
        None
    } else {
        Some(att)
    }
}

fn validate_import_source_url(url: Option<String>) -> Result<Option<String>> {
    let Some(url) = url else {
        return Ok(None);
    };
    let parsed = Url::parse(&url)
        .map_err(|_| BadRequestError::new("source.url must be an absolute http(s) URL"))?;
    match parsed.scheme() {
        "http" | "https" => Ok(Some(url)),
        _ => Err(BadRequestError::new("source.url must use http or https").into()),
    }
}

fn result_row_to_result(
    row: crate::storage::models::EvalCaseResultRow,
    case_name: Option<String>,
) -> EvalCaseResult {
    let target: Option<EvalTarget> = row
        .target
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let target_snapshot: Option<EvalTarget> = row
        .target_snapshot
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());
    let artifacts: Option<BTreeMap<String, String>> = row
        .artifacts
        .as_ref()
        .and_then(|v| serde_json::from_value(v.clone()).ok());

    EvalCaseResult {
        public_id: row
            .public_id
            .parse()
            .unwrap_or_else(|_| EvalResultId::from_uuid(row.id)),
        internal_id: row.id,
        eval_case_id: EvalCaseId::from_uuid(row.eval_case_id),
        case_name,
        session_id: row.session_id.map(SessionId::from_uuid),
        target,
        target_snapshot,
        status: CaseResultStatus::from(row.status.as_str()),
        scores: row.scores,
        metadata: row.metadata,
        turns: row.turns.map(|v| v as u32),
        latency_ms: row.latency_ms.map(|v| v as u64),
        input_tokens: row.input_tokens.map(|v| v as u64),
        output_tokens: row.output_tokens.map(|v| v as u64),
        error_message: row.error_message,
        artifacts,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn validate_scores_payload(scores: &[Score]) -> Result<()> {
    if scores.is_empty() {
        return Err(BadRequestError::new("must provide at least one score").into());
    }
    for (index, score) in scores.iter().enumerate() {
        if !score.value.is_finite() {
            return Err(BadRequestError::new(format!(
                "score at index {index} must have a finite value"
            ))
            .into());
        }
        if !(0.0..=1.0).contains(&score.value) {
            return Err(BadRequestError::new(format!(
                "score at index {index} must have a value between 0.0 and 1.0"
            ))
            .into());
        }
    }
    Ok(())
}

fn validate_metadata_payload(metadata: Option<&serde_json::Value>) -> Result<()> {
    if let Some(metadata) = metadata
        && !metadata.is_object()
    {
        return Err(BadRequestError::new("metadata must be a JSON object").into());
    }
    Ok(())
}

fn validate_score_count(
    case_row: Option<&crate::storage::models::EvalCaseRow>,
    scores: &[Score],
) -> Result<()> {
    let Some(case_row) = case_row else {
        return Ok(());
    };
    let scorers = serde_json::from_value::<Vec<Scorer>>(case_row.scorers.clone())
        .map_err(|e| anyhow::anyhow!("failed to parse eval case scorers: {e}"))?;
    if scorers.len() != scores.len() {
        return Err(BadRequestError::new(format!(
            "score count ({}) must match configured scorer count ({})",
            scores.len(),
            scorers.len()
        ))
        .into());
    }
    Ok(())
}

fn resolve_external_score_status(
    scores: &[Score],
    status: Option<ExternalScoreStatus>,
) -> CaseResultStatus {
    match status {
        Some(ExternalScoreStatus::Passed) => CaseResultStatus::Passed,
        Some(ExternalScoreStatus::Failed) => CaseResultStatus::Failed,
        Some(ExternalScoreStatus::Errored) => CaseResultStatus::Errored,
        None if scores.iter().all(|score| score.pass) => CaseResultStatus::Passed,
        None => CaseResultStatus::Failed,
    }
}

fn build_run_summary(
    case_rows: &[crate::storage::models::EvalCaseRow],
    result_rows: &[crate::storage::models::EvalCaseResultRow],
) -> RunSummary {
    let total = result_rows.len() as u32;
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut errored = 0u32;
    let mut total_score = 0.0f64;
    let mut total_turns = 0.0f64;
    let mut total_latency = 0u64;
    let mut total_input_tokens = 0u64;
    let mut total_output_tokens = 0u64;

    for result in result_rows {
        let status = CaseResultStatus::from(result.status.as_str());
        match status {
            CaseResultStatus::Passed => passed += 1,
            CaseResultStatus::Failed => failed += 1,
            CaseResultStatus::Errored | CaseResultStatus::Timeout => errored += 1,
            CaseResultStatus::Pending | CaseResultStatus::Running | CaseResultStatus::Skipped => {}
        }

        if matches!(status, CaseResultStatus::Passed | CaseResultStatus::Failed) {
            total_score += case_result_avg_score(
                result,
                case_rows.iter().find(|case| case.id == result.eval_case_id),
            );
            total_turns += result.turns.unwrap_or_default() as f64;
            total_latency += result.latency_ms.unwrap_or_default() as u64;
            total_input_tokens += result.input_tokens.unwrap_or_default() as u64;
            total_output_tokens += result.output_tokens.unwrap_or_default() as u64;
        }
    }

    RunSummary {
        total,
        passed,
        failed,
        errored,
        pass_rate: if total > 0 {
            passed as f64 / total as f64
        } else {
            0.0
        },
        avg_score: if total > 0 {
            total_score / total as f64
        } else {
            0.0
        },
        avg_turns: if total > 0 {
            total_turns / total as f64
        } else {
            0.0
        },
        avg_latency_ms: if total > 0 {
            total_latency / total as u64
        } else {
            0
        },
        total_input_tokens,
        total_output_tokens,
    }
}

/// Accumulates a `RunSummary` for an imported run. Mirrors `build_run_summary`
/// but reads the import request directly. `total` counts executed cases
/// (passed/failed/errored); skipped cases are excluded from all tallies.
#[derive(Default)]
struct ImportSummaryAcc {
    total: u32,
    passed: u32,
    failed: u32,
    errored: u32,
    total_score: f64,
    total_turns: f64,
    total_latency: u64,
    total_input_tokens: u64,
    total_output_tokens: u64,
}

impl ImportSummaryAcc {
    fn add(&mut self, status: &str, case: &ImportEvalCaseEntry) {
        match status {
            "passed" => self.passed += 1,
            "failed" => self.failed += 1,
            "errored" | "timeout" => self.errored += 1,
            _ => return, // skipped / not executed: excluded from tallies
        }
        self.total += 1;
        if matches!(status, "passed" | "failed") {
            self.total_score += import_case_avg_score(case);
            self.total_turns += case.turns.unwrap_or_default() as f64;
            self.total_latency += case.latency_ms.unwrap_or_default();
            self.total_input_tokens += case.input_tokens.unwrap_or_default();
            self.total_output_tokens += case.output_tokens.unwrap_or_default();
        }
    }

    fn finish(self) -> RunSummary {
        let total = self.total;
        RunSummary {
            total,
            passed: self.passed,
            failed: self.failed,
            errored: self.errored,
            pass_rate: if total > 0 {
                self.passed as f64 / total as f64
            } else {
                0.0
            },
            avg_score: if total > 0 {
                self.total_score / total as f64
            } else {
                0.0
            },
            avg_turns: if total > 0 {
                self.total_turns / total as f64
            } else {
                0.0
            },
            avg_latency_ms: if total > 0 {
                self.total_latency / total as u64
            } else {
                0
            },
            total_input_tokens: self.total_input_tokens,
            total_output_tokens: self.total_output_tokens,
        }
    }
}

/// Mean of applicable (non-N/A) score values for an imported case.
fn import_case_avg_score(case: &ImportEvalCaseEntry) -> f64 {
    let applicable: Vec<f64> = case
        .scores
        .iter()
        .filter(|score| !score.na)
        .map(|score| score.value)
        .collect();
    if applicable.is_empty() {
        return 0.0;
    }
    applicable.iter().sum::<f64>() / applicable.len() as f64
}

fn case_result_avg_score(
    result: &crate::storage::models::EvalCaseResultRow,
    case_row: Option<&crate::storage::models::EvalCaseRow>,
) -> f64 {
    let Some(scores) = result
        .scores
        .clone()
        .and_then(|value| serde_json::from_value::<Vec<Score>>(value).ok())
    else {
        return 0.0;
    };
    if scores.is_empty() {
        return 0.0;
    }

    let scorers = case_row
        .and_then(|case| serde_json::from_value::<Vec<Scorer>>(case.scorers.clone()).ok())
        .unwrap_or_default();
    if scorers.len() == scores.len() {
        let total_weight: f64 = scorers.iter().map(scorer_weight).sum();
        if total_weight > 0.0 {
            let weighted_sum: f64 = scores
                .iter()
                .zip(scorers.iter())
                .map(|(score, scorer)| score.value * scorer_weight(scorer))
                .sum();
            return weighted_sum / total_weight;
        }
    }

    scores.iter().map(|score| score.value).sum::<f64>() / scores.len() as f64
}

fn scorer_weight(scorer: &Scorer) -> f64 {
    match scorer {
        Scorer::Contains { weight, .. } => *weight,
        Scorer::NotContains { weight, .. } => *weight,
        Scorer::Regex { weight, .. } => *weight,
        Scorer::ToolCalled { weight, .. } => *weight,
        Scorer::ToolNotCalled { weight, .. } => *weight,
        Scorer::ToolCallCount { weight, .. } => *weight,
        Scorer::TurnsWithin { weight, .. } => *weight,
        Scorer::FileContains { weight, .. } => *weight,
        Scorer::JsonSchema { weight, .. } => *weight,
        Scorer::CitationFaithful { weight, .. } => *weight,
        Scorer::CitationJudged { weight, .. } => *weight,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::evals::limits::EvalLimits;
    use crate::storage::StorageBackend;
    use crate::storage::models::{CreateEvalCaseRow, CreateEvalRow};
    use everruns_core::Caller;

    fn test_target() -> EvalTarget {
        EvalTarget::Session {
            harness_id: None,
            harness_name: None,
            agent_id: None,
            model_id: None,
            system_prompt: None,
            max_iterations: None,
        }
    }

    async fn seed_eval(db: &StorageBackend, org_id: i64, n_cases: usize) -> String {
        let eval_uuid = Uuid::now_v7();
        let eval_public_id = format!("eval_{:032x}", eval_uuid.as_u128());
        db.create_eval(
            org_id,
            CreateEvalRow {
                public_id: eval_public_id.clone(),
                name: "test eval".to_string(),
                description: None,
                target: Some(serde_json::to_value(test_target()).unwrap()),
                model_override: None,
                tags: vec![],
            },
        )
        .await
        .unwrap();

        let eval = db
            .get_eval_by_public_id(org_id, &eval_public_id)
            .await
            .unwrap()
            .unwrap();

        for i in 0..n_cases {
            let case_uuid = Uuid::now_v7();
            db.create_eval_case(
                eval.id,
                CreateEvalCaseRow {
                    public_id: format!("evalcase_{:032x}", case_uuid.as_u128()),
                    name: format!("case {i}"),
                    description: None,
                    target: None,
                    tags: vec![],
                    conversation: serde_json::json!([{"role":"user","content":"hi"}]),
                    post: None,
                    artifacts: None,
                    scorers: serde_json::json!([]),
                    max_turns: None,
                    timeout_seconds: None,
                    position: i as i32,
                },
            )
            .await
            .unwrap();
        }

        eval_public_id
    }

    #[tokio::test]
    async fn concurrent_run_limit_enforced() {
        let db = StorageBackend::in_memory();
        let org_id = 1i64;
        let caller = Caller::internal(org_id);
        let svc = Arc::new(EvalService::new(Arc::new(db)).with_limits(EvalLimits {
            max_concurrent_runs_per_org: 2,
            max_cases_per_run: 500,
            max_evals_per_import: 10,
        }));

        let eval_id = seed_eval(svc.db.as_ref(), org_id, 1).await;
        let run_req = CreateEvalRunRequest {
            target: None,
            model_override: None,
        };

        let mut tasks = Vec::new();
        for _ in 0..8 {
            let svc = Arc::clone(&svc);
            let caller = caller.clone();
            let eval_id = eval_id.clone();
            let run_req = run_req.clone();
            tasks.push(tokio::spawn(async move {
                svc.create_run(&caller, &eval_id, run_req).await
            }));
        }

        let mut successes = 0;
        let mut limit_errors = 0;
        for task in tasks {
            match task.await.unwrap() {
                Ok(_) => successes += 1,
                Err(err) if err.to_string().contains("Too many concurrent eval runs") => {
                    limit_errors += 1
                }
                Err(err) => panic!("unexpected error: {err}"),
            }
        }

        assert_eq!(successes, 2);
        assert_eq!(limit_errors, 6);
        assert_eq!(
            svc.db
                .count_running_eval_runs_for_org(org_id)
                .await
                .unwrap(),
            2
        );
    }

    #[test]
    fn validate_import_source_url_allows_only_http_urls() {
        assert_eq!(
            validate_import_source_url(Some("https://mira.example/runs/1".into())).unwrap(),
            Some("https://mira.example/runs/1".into())
        );
        assert_eq!(
            validate_import_source_url(Some("http://mira.example/runs/1".into())).unwrap(),
            Some("http://mira.example/runs/1".into())
        );
        assert!(validate_import_source_url(None).unwrap().is_none());
        assert!(validate_import_source_url(Some("javascript:alert(1)".into())).is_err());
        assert!(validate_import_source_url(Some("/runs/1".into())).is_err());
    }

    fn import_request(run_id: &str, failed_value: f64) -> ImportEvalRunRequest {
        use crate::api::evals::{
            ImportCaseStatus, ImportEvalCaseEntry, ImportEvalGroup, ImportEvalSource,
            ImportEvalTarget, ImportScore,
        };
        let target = ImportEvalTarget {
            provider: "anthropic".into(),
            model: "claude".into(),
            params: None,
        };
        ImportEvalRunRequest {
            source: ImportEvalSource {
                system: "mira".into(),
                version: Some("0.1.0".into()),
                url: None,
                run_id: run_id.into(),
                metadata: None,
            },
            evals: vec![ImportEvalGroup {
                name: "coding".into(),
                description: Some("imported".into()),
                tags: vec!["ci".into()],
                cases: vec![
                    ImportEvalCaseEntry {
                        name: "case-a".into(),
                        description: None,
                        input: vec!["solve it".into()],
                        target: target.clone(),
                        status: ImportCaseStatus::Passed,
                        scores: vec![ImportScore {
                            scorer: "contains".into(),
                            value: 1.0,
                            pass: true,
                            reason: "ok".into(),
                            na: false,
                        }],
                        transcript: Some(serde_json::json!({"messages": []})),
                        metrics: Some(serde_json::json!({"cost_usd": 0.01})),
                        turns: Some(2),
                        latency_ms: Some(1200),
                        input_tokens: Some(100),
                        output_tokens: Some(50),
                        error_message: None,
                    },
                    ImportEvalCaseEntry {
                        name: "case-b".into(),
                        description: None,
                        input: vec!["other".into()],
                        target,
                        status: ImportCaseStatus::Failed,
                        scores: vec![ImportScore {
                            scorer: "contains".into(),
                            value: failed_value,
                            pass: false,
                            reason: "no".into(),
                            na: false,
                        }],
                        transcript: None,
                        metrics: None,
                        turns: Some(1),
                        latency_ms: Some(800),
                        input_tokens: Some(80),
                        output_tokens: Some(20),
                        error_message: None,
                    },
                ],
            }],
        }
    }

    #[tokio::test]
    async fn import_run_creates_external_run() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_id = 7i64;
        let caller = Caller::internal(org_id);
        let svc = EvalService::new(db.clone());

        let runs = svc
            .import_run(&caller, import_request("mira-run-1", 0.0))
            .await
            .unwrap();

        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.source, EvalRunSource::External);
        assert_eq!(run.status, EvalRunStatus::Completed);
        let summary = run.summary.as_ref().expect("summary");
        assert_eq!(summary.total, 2);
        assert_eq!(summary.passed, 1);
        assert_eq!(summary.failed, 1);
        assert!((summary.pass_rate - 0.5).abs() < 1e-9);

        // Eval auto-provisioned by name.
        let evals = db.list_evals(org_id, None, false).await.unwrap();
        assert_eq!(evals.len(), 1);
        assert_eq!(evals[0].name, "coding");

        // Results stored with transcript/metrics in the metadata envelope.
        let result_rows = db.list_eval_case_results(run.internal_id).await.unwrap();
        assert_eq!(result_rows.len(), 2);
        let passed = result_rows
            .iter()
            .find(|r| r.status == "passed")
            .expect("passed result");
        let meta = passed.metadata.as_ref().expect("metadata envelope");
        assert!(meta.get("transcript").is_some());
        assert_eq!(meta["metrics"]["cost_usd"], 0.01);
    }

    #[tokio::test]
    async fn import_run_is_idempotent_on_source_run_id() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_id = 8i64;
        let caller = Caller::internal(org_id);
        let svc = EvalService::new(db.clone());

        svc.import_run(&caller, import_request("mira-run-1", 0.0))
            .await
            .unwrap();
        // Re-publish the same run id with a different failed score.
        svc.import_run(&caller, import_request("mira-run-1", 0.4))
            .await
            .unwrap();

        // Still one eval and exactly one run (the prior was replaced).
        let evals = db.list_evals(org_id, None, false).await.unwrap();
        assert_eq!(evals.len(), 1);
        let run_rows = db.list_eval_runs(evals[0].id).await.unwrap();
        assert_eq!(run_rows.len(), 1);
        // And exactly two results (not four) — old ones were cascaded away.
        let result_rows = db.list_eval_case_results(run_rows[0].id).await.unwrap();
        assert_eq!(result_rows.len(), 2);
    }

    #[tokio::test]
    async fn import_run_limits_fan_out_before_storage() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_id = 10i64;
        let caller = Caller::internal(org_id);
        let svc = EvalService::new(db.clone()).with_limits(EvalLimits {
            max_concurrent_runs_per_org: 5,
            max_cases_per_run: 1,
            max_evals_per_import: 1,
        });

        let err = svc
            .import_run(&caller, import_request("too-many-cases", 0.0))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("per-run limit of 1 cases"));
        assert!(db.list_evals(org_id, None, false).await.unwrap().is_empty());

        let mut req = import_request("too-many-evals", 0.0);
        req.evals[0].cases.truncate(1);
        req.evals.push(req.evals[0].clone());
        let err = svc.import_run(&caller, req).await.unwrap_err();
        assert!(err.to_string().contains("limit of 1 evals"));
        assert!(db.list_evals(org_id, None, false).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn share_link_resolves_sanitized_then_revokes() {
        let db = Arc::new(StorageBackend::in_memory());
        let org_id = 9i64;
        let caller = Caller::internal(org_id);
        let svc = EvalService::new(db.clone());

        let runs = svc
            .import_run(&caller, import_request("mira-share-1", 0.0))
            .await
            .unwrap();
        let run_pub = runs[0].public_id.to_string();
        let evals = db.list_evals(org_id, None, false).await.unwrap();
        let eval_pub = evals[0].public_id.to_string();

        // Mint a link; the run now reports an active share.
        let link = svc
            .create_run_share(&caller, &eval_pub, &run_pub)
            .await
            .unwrap();
        assert!(link.token.starts_with("evr_share_"));
        assert!(
            svc.run_has_active_share(&caller, &eval_pub, &run_pub)
                .await
                .unwrap()
        );

        // Public resolve returns the sanitized run.
        let public = svc
            .resolve_public_share(&link.token)
            .await
            .unwrap()
            .expect("token resolves");
        assert_eq!(public.id, run_pub);
        assert_eq!(public.source, EvalRunSource::External);
        let passed = public
            .results
            .iter()
            .find(|r| r.status == CaseResultStatus::Passed)
            .expect("a passed result");
        // External (label-only) target is exposed and the transcript is present.
        assert!(matches!(passed.target, Some(EvalTarget::External { .. })));
        assert!(passed.transcript.is_some());

        // Re-minting revokes the old link (one active link per run).
        let link2 = svc
            .create_run_share(&caller, &eval_pub, &run_pub)
            .await
            .unwrap();
        assert!(
            svc.resolve_public_share(&link.token)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            svc.resolve_public_share(&link2.token)
                .await
                .unwrap()
                .is_some()
        );

        // Revoke → nothing resolves; unknown tokens are a quiet None (no oracle).
        assert!(
            svc.revoke_run_share(&caller, &eval_pub, &run_pub)
                .await
                .unwrap()
        );
        assert!(
            svc.resolve_public_share(&link2.token)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            svc.resolve_public_share("evr_share_deadbeef")
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            svc.resolve_public_share("not-a-token")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn case_memory_limit_enforced() {
        let db = StorageBackend::in_memory();
        let org_id = 2i64;
        let caller = Caller::internal(org_id);
        let svc = EvalService::new(Arc::new(db)).with_limits(EvalLimits {
            max_concurrent_runs_per_org: 100,
            max_cases_per_run: 2,
            max_evals_per_import: 10,
        });

        // Eval has 3 cases — exceeds the 2-case limit.
        let eval_id = seed_eval(svc.db.as_ref(), org_id, 3).await;
        let run_req = CreateEvalRunRequest {
            target: None,
            model_override: None,
        };

        let err = svc
            .create_run(&caller, &eval_id, run_req)
            .await
            .unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("per-run limit"),
            "Expected per-run limit error, got: {msg}"
        );
    }
}
