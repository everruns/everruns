// Eval service for business logic
// Decision: Evals reuse the same permission policies as agents (OrgAgentsManage)
// Decision: Each eval case creates a real session — no mock execution
// Decision: EvalTarget replaces harness_id + agent_id. Resolution: run → case → eval → org default.

use crate::api::evals::{
    BulkUpdateEvalRunScoresRequest, CreateEvalCaseRequest, CreateEvalRequest, CreateEvalRunRequest,
    ExternalScoreStatus, UpdateEvalCaseRequest, UpdateEvalRequest, UpdateEvalResultScoresRequest,
};
use crate::domains::evals::limits::EvalLimits;
use crate::domains::evals::runner::{EvalRunContext, spawn_eval_run};
use crate::errors::{BadRequestError, ResourceNotFoundError};
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateEvalCaseRow, CreateEvalRow, CreateEvalRunError, CreateEvalRunRow,
    UpdateEvalCaseResultRow, UpdateEvalCaseRow, UpdateEvalRow,
};
use anyhow::Result;
use everruns_core::eval::*;
use everruns_core::typed_id::{EvalCaseId, EvalId, EvalResultId, EvalRunId, SessionId};
use everruns_core::{Caller, Permission, Policy, Rule};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;
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
        triggered_by: row.triggered_by,
        started_at: row.started_at,
        completed_at: row.completed_at,
        summary: row.summary.and_then(|s| serde_json::from_value(s).ok()),
        results,
        created_at: row.created_at,
        updated_at: row.updated_at,
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
            CaseResultStatus::Pending | CaseResultStatus::Running => {}
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

    #[tokio::test]
    async fn case_memory_limit_enforced() {
        let db = StorageBackend::in_memory();
        let org_id = 2i64;
        let caller = Caller::internal(org_id);
        let svc = EvalService::new(Arc::new(db)).with_limits(EvalLimits {
            max_concurrent_runs_per_org: 100,
            max_cases_per_run: 2,
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
