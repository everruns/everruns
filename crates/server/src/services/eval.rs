// Eval service for business logic
// Decision: Evals reuse the same permission policies as agents (OrgAgentsManage)
// Decision: Each eval case creates a real session — no mock execution

use crate::api::evals::{
    CreateEvalCaseRequest, CreateEvalRequest, CreateEvalRunRequest, UpdateEvalCaseRequest,
    UpdateEvalRequest,
};
use crate::errors::ResourceNotFoundError;
use crate::storage::StorageBackend;
use crate::storage::models::{
    CreateEvalCaseResultRow, CreateEvalCaseRow, CreateEvalRow, CreateEvalRunRow, UpdateEvalCaseRow,
    UpdateEvalRow,
};
use anyhow::Result;
use everruns_core::eval::*;
use everruns_core::typed_id::{EvalCaseId, EvalId, EvalResultId, EvalRunId, SessionId};
use everruns_core::{AgentId, Caller, HarnessId, Permission, Policy, Rule};
use everruns_macros::policy;
use std::sync::Arc;
use uuid::Uuid;

/// Policy: View evals (read-only).
pub const EVAL_VIEW: Policy = Policy {
    id: "eval.view",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

/// Policy: Manage evals (create, update, run).
pub const EVAL_MANAGE: Policy = Policy {
    id: "eval.manage",
    rules: &[Rule::UserHasPermission(Permission::OrgAgentsManage)],
};

pub struct EvalService {
    db: Arc<StorageBackend>,
}

impl EvalService {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        Self { db }
    }

    // ============================================
    // Eval CRUD
    // ============================================

    #[policy(EVAL_MANAGE)]
    pub async fn create(&self, caller: &Caller, req: CreateEvalRequest) -> Result<Eval> {
        // Validate agent exists
        let agent_row = self
            .db
            .get_agent_by_public_id(caller.org_id, &req.agent_id.to_string())
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Agent"))?;
        if agent_row.status != "active" {
            anyhow::bail!("Archived or deleted agents cannot be assigned to evals");
        }

        // Validate harness exists
        let harness_row = self
            .db
            .get_harness(caller.org_id, req.harness_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Harness"))?;
        if harness_row.status != "active" {
            anyhow::bail!("Archived or deleted harnesses cannot be assigned to evals");
        }

        let internal_uuid = Uuid::now_v7();
        let public_id = EvalId::from_uuid(internal_uuid);

        let input = CreateEvalRow {
            public_id: public_id.to_string(),
            name: req.name,
            description: req.description,
            agent_id: agent_row.id.uuid(),
            harness_id: harness_row.id.uuid(),
            model_override: req.model_override,
            tags: req.tags.unwrap_or_default(),
        };

        let row = self.db.create_eval(caller.org_id, input).await?;
        self.row_to_eval(row).await
    }

    #[policy(EVAL_VIEW)]
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

    #[policy(EVAL_VIEW)]
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

    #[policy(EVAL_MANAGE)]
    pub async fn update(
        &self,
        caller: &Caller,
        public_id: &str,
        req: UpdateEvalRequest,
    ) -> Result<Option<Eval>> {
        let existing = self
            .db
            .get_eval_by_public_id(caller.org_id, public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let agent_id = if let Some(ref agent_id) = req.agent_id {
            let agent_row = self
                .db
                .get_agent_by_public_id(caller.org_id, &agent_id.to_string())
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Agent"))?;
            Some(agent_row.id.uuid())
        } else {
            None
        };

        let harness_id = if let Some(ref harness_id) = req.harness_id {
            let harness_row = self
                .db
                .get_harness(caller.org_id, *harness_id)
                .await?
                .ok_or_else(|| ResourceNotFoundError::new("Harness"))?;
            Some(harness_row.id.uuid())
        } else {
            None
        };

        let input = UpdateEvalRow {
            name: req.name,
            description: req.description,
            agent_id,
            harness_id,
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

    #[policy(EVAL_MANAGE)]
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

    #[policy(EVAL_MANAGE)]
    pub async fn create_case(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        req: CreateEvalCaseRequest,
    ) -> Result<EvalCase> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let case_uuid = Uuid::now_v7();
        let case_public_id = EvalCaseId::from_uuid(case_uuid);

        let case_count = self.db.count_eval_cases(eval.id).await?;

        let input = CreateEvalCaseRow {
            public_id: case_public_id.to_string(),
            name: req.name,
            description: req.description,
            tags: req.tags.unwrap_or_default(),
            conversation: serde_json::to_value(&req.conversation)?,
            scorers: serde_json::to_value(&req.scorers)?,
            max_turns: req.max_turns.map(|v| v as i32),
            timeout_seconds: req.timeout_seconds.map(|v| v as i32),
            position: req.position.unwrap_or(case_count as i32),
        };

        let row = self.db.create_eval_case(eval.id, input).await?;
        Ok(case_row_to_case(row))
    }

    #[policy(EVAL_VIEW)]
    pub async fn list_cases(&self, caller: &Caller, eval_public_id: &str) -> Result<Vec<EvalCase>> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let rows = self.db.list_eval_cases(eval.id).await?;
        Ok(rows.into_iter().map(case_row_to_case).collect())
    }

    #[policy(EVAL_VIEW)]
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

    #[policy(EVAL_MANAGE)]
    pub async fn update_case(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        case_public_id: &str,
        req: UpdateEvalCaseRequest,
    ) -> Result<Option<EvalCase>> {
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

        let input = UpdateEvalCaseRow {
            name: req.name,
            description: req.description,
            tags: req.tags,
            conversation: req
                .conversation
                .map(|c| serde_json::to_value(c).unwrap_or_default()),
            scorers: req
                .scorers
                .map(|s| serde_json::to_value(s).unwrap_or_default()),
            max_turns: req.max_turns.map(|v| v as i32),
            timeout_seconds: req.timeout_seconds.map(|v| v as i32),
            position: req.position,
        };

        let row = self.db.update_eval_case(case.id, input).await?;
        Ok(row.map(case_row_to_case))
    }

    #[policy(EVAL_MANAGE)]
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

    #[policy(EVAL_MANAGE)]
    pub async fn create_run(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        req: CreateEvalRunRequest,
    ) -> Result<EvalRun> {
        let eval = self
            .db
            .get_eval_by_public_id(caller.org_id, eval_public_id)
            .await?
            .ok_or_else(|| ResourceNotFoundError::new("Eval"))?;

        let run_uuid = Uuid::now_v7();
        let run_public_id = EvalRunId::from_uuid(run_uuid);

        let input = CreateEvalRunRow {
            public_id: run_public_id.to_string(),
            eval_id: eval.id,
            model_override: req.model_override,
            filter_tags: req.filter_tags,
            triggered_by: "user".to_string(),
        };

        let run_row = self.db.create_eval_run(caller.org_id, input).await?;

        // Create pending case results for each case
        let cases = self.db.list_eval_cases(eval.id).await?;
        for case in &cases {
            let result_uuid = Uuid::now_v7();
            let result_public_id = EvalResultId::from_uuid(result_uuid);
            let result_input = CreateEvalCaseResultRow {
                public_id: result_public_id.to_string(),
                eval_run_id: run_row.id,
                eval_case_id: case.id,
            };
            self.db.create_eval_case_result(result_input).await?;
        }

        Ok(run_row_to_run(run_row, vec![]))
    }

    #[policy(EVAL_VIEW)]
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

    #[policy(EVAL_VIEW)]
    pub async fn get_run(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<Option<EvalRun>> {
        // Verify eval ownership
        let _eval = self
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

    #[policy(EVAL_MANAGE)]
    pub async fn cancel_run(
        &self,
        caller: &Caller,
        eval_public_id: &str,
        run_public_id: &str,
    ) -> Result<Option<EvalRun>> {
        let _eval = self
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

        if !matches!(run_row.status.as_str(), "pending" | "running") {
            anyhow::bail!("Can only cancel pending or running eval runs");
        }

        let updated = self
            .db
            .update_eval_run_status(run_row.id, "cancelled", None)
            .await?;
        Ok(updated.map(|r| run_row_to_run(r, vec![])))
    }

    // ============================================
    // Helpers
    // ============================================

    async fn row_to_eval(&self, row: crate::storage::models::EvalRow) -> Result<Eval> {
        let case_count = self.db.count_eval_cases(row.id).await?;
        let last_run = self.db.get_latest_eval_run(row.id).await?;

        let last_run_view = last_run.map(|r| EvalRunSummaryView {
            public_id: EvalRunId::from_uuid(r.id),
            status: EvalRunStatus::from(r.status.as_str()),
            summary: r.summary.and_then(|s| serde_json::from_value(s).ok()),
            created_at: r.created_at,
        });

        // Resolve agent and harness public IDs
        let agent_id = AgentId::from_uuid(row.agent_id);
        let harness_id = HarnessId::from_uuid(row.harness_id);

        Ok(Eval {
            public_id: EvalId::from_uuid(row.id),
            internal_id: row.id,
            org_id: row.org_id,
            name: row.name,
            description: row.description,
            agent_id,
            harness_id,
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
    EvalCase {
        public_id: EvalCaseId::from_uuid(row.id),
        internal_id: row.id,
        name: row.name,
        description: row.description,
        tags: row.tags,
        conversation: serde_json::from_value(row.conversation).unwrap_or_default(),
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
    EvalRun {
        public_id: EvalRunId::from_uuid(row.id),
        internal_id: row.id,
        org_id: row.org_id,
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
    EvalCaseResult {
        public_id: EvalResultId::from_uuid(row.id),
        internal_id: row.id,
        eval_case_id: EvalCaseId::from_uuid(row.eval_case_id),
        case_name,
        session_id: row.session_id.map(SessionId::from_uuid),
        status: CaseResultStatus::from(row.status.as_str()),
        scores: row.scores,
        turns: row.turns.map(|v| v as u32),
        latency_ms: row.latency_ms.map(|v| v as u64),
        input_tokens: row.input_tokens.map(|v| v as u64),
        output_tokens: row.output_tokens.map(|v| v as u64),
        error_message: row.error_message,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}
