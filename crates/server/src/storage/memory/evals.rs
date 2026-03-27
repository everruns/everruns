// In-memory eval storage

use anyhow::Result;
use uuid::Uuid;

use super::{InMemoryDatabase, matches_search_tokens};
use crate::storage::models::*;

impl InMemoryDatabase {
    // ============================================
    // Eval CRUD
    // ============================================

    pub async fn create_eval(&self, org_id: i64, input: CreateEvalRow) -> Result<EvalRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = EvalRow {
            id,
            org_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            agent_id: input.agent_id,
            harness_id: input.harness_id,
            model_override: input.model_override,
            tags: input.tags,
            status: "active".to_string(),
            created_at: now,
            updated_at: now,
            archived_at: None,
            deleted_at: None,
        };
        self.evals.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn get_eval_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRow>> {
        let evals = self.evals.read();
        Ok(evals
            .values()
            .find(|e| e.org_id == org_id && e.public_id == public_id)
            .cloned())
    }

    pub async fn list_evals(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<EvalRow>> {
        let evals = self.evals.read();
        let mut result: Vec<EvalRow> = evals
            .values()
            .filter(|e| {
                e.org_id == org_id
                    && if include_archived {
                        e.status != "deleted"
                    } else {
                        e.status != "archived" && e.status != "deleted"
                    }
            })
            .filter(|e| {
                matches_search_tokens(search, &[&e.name, e.description.as_deref().unwrap_or("")])
            })
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn update_eval(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateEvalRow,
    ) -> Result<Option<EvalRow>> {
        let mut evals = self.evals.write();
        let Some(eval) = evals.get_mut(&id) else {
            return Ok(None);
        };
        if eval.org_id != org_id {
            return Ok(None);
        }
        if let Some(name) = input.name {
            eval.name = name;
        }
        if let Some(description) = input.description {
            eval.description = Some(description);
        }
        if let Some(agent_id) = input.agent_id {
            eval.agent_id = agent_id;
        }
        if let Some(harness_id) = input.harness_id {
            eval.harness_id = harness_id;
        }
        if let Some(model_override) = input.model_override {
            eval.model_override = Some(model_override);
        }
        if let Some(tags) = input.tags {
            eval.tags = tags;
        }
        if let Some(status) = input.status {
            eval.status = status;
        }
        eval.updated_at = Self::now();
        Ok(Some(eval.clone()))
    }

    pub async fn delete_eval(&self, org_id: i64, id: Uuid) -> Result<bool> {
        let mut evals = self.evals.write();
        let Some(eval) = evals.get_mut(&id) else {
            return Ok(false);
        };
        if eval.org_id != org_id || eval.status != "active" {
            return Ok(false);
        }
        eval.status = "archived".to_string();
        eval.archived_at = Some(Self::now());
        eval.updated_at = Self::now();
        Ok(true)
    }

    // ============================================
    // Eval Case CRUD
    // ============================================

    pub async fn create_eval_case(
        &self,
        eval_id: Uuid,
        input: CreateEvalCaseRow,
    ) -> Result<EvalCaseRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = EvalCaseRow {
            id,
            eval_id,
            public_id: input.public_id,
            name: input.name,
            description: input.description,
            tags: input.tags,
            conversation: input.conversation,
            scorers: input.scorers,
            max_turns: input.max_turns,
            timeout_seconds: input.timeout_seconds,
            position: input.position,
            created_at: now,
            updated_at: now,
        };
        self.eval_cases.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn list_eval_cases(&self, eval_id: Uuid) -> Result<Vec<EvalCaseRow>> {
        let cases = self.eval_cases.read();
        let mut result: Vec<EvalCaseRow> = cases
            .values()
            .filter(|c| c.eval_id == eval_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| {
            a.position
                .cmp(&b.position)
                .then(a.created_at.cmp(&b.created_at))
        });
        Ok(result)
    }

    pub async fn get_eval_case_by_public_id(
        &self,
        eval_id: Uuid,
        public_id: &str,
    ) -> Result<Option<EvalCaseRow>> {
        let cases = self.eval_cases.read();
        Ok(cases
            .values()
            .find(|c| c.eval_id == eval_id && c.public_id == public_id)
            .cloned())
    }

    pub async fn update_eval_case(
        &self,
        id: Uuid,
        input: UpdateEvalCaseRow,
    ) -> Result<Option<EvalCaseRow>> {
        let mut cases = self.eval_cases.write();
        let Some(case) = cases.get_mut(&id) else {
            return Ok(None);
        };
        if let Some(name) = input.name {
            case.name = name;
        }
        if let Some(description) = input.description {
            case.description = Some(description);
        }
        if let Some(tags) = input.tags {
            case.tags = tags;
        }
        if let Some(conversation) = input.conversation {
            case.conversation = conversation;
        }
        if let Some(scorers) = input.scorers {
            case.scorers = scorers;
        }
        if let Some(max_turns) = input.max_turns {
            case.max_turns = Some(max_turns);
        }
        if let Some(timeout_seconds) = input.timeout_seconds {
            case.timeout_seconds = Some(timeout_seconds);
        }
        if let Some(position) = input.position {
            case.position = position;
        }
        case.updated_at = Self::now();
        Ok(Some(case.clone()))
    }

    pub async fn delete_eval_case(&self, id: Uuid) -> Result<bool> {
        let mut cases = self.eval_cases.write();
        Ok(cases.remove(&id).is_some())
    }

    pub async fn count_eval_cases(&self, eval_id: Uuid) -> Result<i64> {
        let cases = self.eval_cases.read();
        let count = cases.values().filter(|c| c.eval_id == eval_id).count();
        Ok(count as i64)
    }

    // ============================================
    // Eval Run CRUD
    // ============================================

    pub async fn create_eval_run(
        &self,
        org_id: i64,
        input: CreateEvalRunRow,
    ) -> Result<EvalRunRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = EvalRunRow {
            id,
            eval_id: input.eval_id,
            org_id,
            public_id: input.public_id,
            model_override: input.model_override,
            filter_tags: input.filter_tags,
            status: "pending".to_string(),
            triggered_by: input.triggered_by,
            started_at: None,
            completed_at: None,
            summary: None,
            created_at: now,
            updated_at: now,
        };
        self.eval_runs.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn list_eval_runs(&self, eval_id: Uuid) -> Result<Vec<EvalRunRow>> {
        let runs = self.eval_runs.read();
        let mut result: Vec<EvalRunRow> = runs
            .values()
            .filter(|r| r.eval_id == eval_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(result)
    }

    pub async fn get_eval_run_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRunRow>> {
        let runs = self.eval_runs.read();
        Ok(runs
            .values()
            .find(|r| r.org_id == org_id && r.public_id == public_id)
            .cloned())
    }

    pub async fn update_eval_run_status(
        &self,
        id: Uuid,
        status: &str,
        summary: Option<serde_json::Value>,
    ) -> Result<Option<EvalRunRow>> {
        let mut runs = self.eval_runs.write();
        let Some(run) = runs.get_mut(&id) else {
            return Ok(None);
        };
        let now = Self::now();
        if status == "running" && run.started_at.is_none() {
            run.started_at = Some(now);
        }
        if matches!(status, "completed" | "failed" | "cancelled") {
            run.completed_at = Some(now);
        }
        run.status = status.to_string();
        if let Some(s) = summary {
            run.summary = Some(s);
        }
        run.updated_at = now;
        Ok(Some(run.clone()))
    }

    pub async fn get_latest_eval_run(&self, eval_id: Uuid) -> Result<Option<EvalRunRow>> {
        let runs = self.eval_runs.read();
        let mut matching: Vec<&EvalRunRow> =
            runs.values().filter(|r| r.eval_id == eval_id).collect();
        matching.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        Ok(matching.first().cloned().cloned())
    }

    // ============================================
    // Eval Case Result CRUD
    // ============================================

    pub async fn create_eval_case_result(
        &self,
        input: CreateEvalCaseResultRow,
    ) -> Result<EvalCaseResultRow> {
        let now = Self::now();
        let id = Uuid::now_v7();
        let row = EvalCaseResultRow {
            id,
            eval_run_id: input.eval_run_id,
            eval_case_id: input.eval_case_id,
            public_id: input.public_id,
            session_id: None,
            status: "pending".to_string(),
            scores: None,
            turns: None,
            latency_ms: None,
            input_tokens: None,
            output_tokens: None,
            error_message: None,
            created_at: now,
            updated_at: now,
        };
        self.eval_case_results.write().insert(id, row.clone());
        Ok(row)
    }

    pub async fn list_eval_case_results(
        &self,
        eval_run_id: Uuid,
    ) -> Result<Vec<EvalCaseResultRow>> {
        let results = self.eval_case_results.read();
        let mut result: Vec<EvalCaseResultRow> = results
            .values()
            .filter(|r| r.eval_run_id == eval_run_id)
            .cloned()
            .collect();
        result.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        Ok(result)
    }

    pub async fn update_eval_case_result(
        &self,
        id: Uuid,
        input: UpdateEvalCaseResultRow,
    ) -> Result<Option<EvalCaseResultRow>> {
        let mut results = self.eval_case_results.write();
        let Some(result) = results.get_mut(&id) else {
            return Ok(None);
        };
        if let Some(session_id) = input.session_id {
            result.session_id = Some(session_id);
        }
        if let Some(status) = input.status {
            result.status = status;
        }
        if let Some(scores) = input.scores {
            result.scores = Some(scores);
        }
        if let Some(turns) = input.turns {
            result.turns = Some(turns);
        }
        if let Some(latency_ms) = input.latency_ms {
            result.latency_ms = Some(latency_ms);
        }
        if let Some(input_tokens) = input.input_tokens {
            result.input_tokens = Some(input_tokens);
        }
        if let Some(output_tokens) = input.output_tokens {
            result.output_tokens = Some(output_tokens);
        }
        if let Some(error_message) = input.error_message {
            result.error_message = Some(error_message);
        }
        result.updated_at = Self::now();
        Ok(Some(result.clone()))
    }
}
