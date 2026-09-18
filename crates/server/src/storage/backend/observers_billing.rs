//! Observers, evals, health checks, budgets, and payments.

use super::*;

impl StorageBackend {
    pub async fn get_observer_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<ObserverRow>> {
        dispatch!(self, get_observer_by_public_id, org_id, public_id)
    }

    pub async fn get_observer(&self, id: Uuid) -> Result<Option<ObserverRow>> {
        dispatch!(self, get_observer, id)
    }

    pub async fn list_observers(
        &self,
        org_id: i64,
        include_archived: bool,
    ) -> Result<Vec<ObserverRow>> {
        dispatch!(self, list_observers, org_id, include_archived)
    }

    pub async fn list_active_observers(&self, org_id: i64) -> Result<Vec<ObserverRow>> {
        dispatch!(self, list_active_observers, org_id)
    }

    pub async fn count_active_observers(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_active_observers, org_id)
    }

    pub async fn update_observer(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateObserverRow,
    ) -> Result<Option<ObserverRow>> {
        dispatch!(self, update_observer, org_id, id, input)
    }

    pub async fn delete_observer(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_observer, org_id, id)
    }

    pub async fn enqueue_trace_scores(
        &self,
        org_id: i64,
        inputs: &[CreateTraceScoreRow],
    ) -> Result<u64> {
        dispatch!(self, enqueue_trace_scores, org_id, inputs)
    }

    pub async fn claim_trace_scores(
        &self,
        limit: i64,
        stale_after_seconds: i64,
        max_attempts: i32,
    ) -> Result<Vec<TraceScoreRow>> {
        dispatch!(
            self,
            claim_trace_scores,
            limit,
            stale_after_seconds,
            max_attempts
        )
    }

    pub async fn complete_trace_score(
        &self,
        id: Uuid,
        input: CompleteTraceScoreRow,
    ) -> Result<Option<TraceScoreRow>> {
        dispatch!(self, complete_trace_score, id, input)
    }

    pub async fn list_trace_scores(
        &self,
        org_id: i64,
        params: ListTraceScoresParams,
    ) -> Result<Vec<TraceScoreRow>> {
        dispatch!(self, list_trace_scores, org_id, params)
    }

    // ============================================
    // Eval CRUD
    // ============================================

    pub async fn create_eval(&self, org_id: i64, input: CreateEvalRow) -> Result<EvalRow> {
        dispatch!(self, create_eval, org_id, input)
    }

    pub async fn get_eval_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRow>> {
        dispatch!(self, get_eval_by_public_id, org_id, public_id)
    }

    pub async fn list_evals(
        &self,
        org_id: i64,
        search: Option<&str>,
        include_archived: bool,
    ) -> Result<Vec<EvalRow>> {
        dispatch!(self, list_evals, org_id, search, include_archived)
    }

    pub async fn update_eval(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateEvalRow,
    ) -> Result<Option<EvalRow>> {
        dispatch!(self, update_eval, org_id, id, input)
    }

    pub async fn delete_eval(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_eval, org_id, id)
    }

    // ============================================
    // Eval Case CRUD
    // ============================================

    pub async fn create_eval_case(
        &self,
        eval_id: Uuid,
        input: CreateEvalCaseRow,
    ) -> Result<EvalCaseRow> {
        dispatch!(self, create_eval_case, eval_id, input)
    }

    pub async fn list_eval_cases(&self, eval_id: Uuid) -> Result<Vec<EvalCaseRow>> {
        dispatch!(self, list_eval_cases, eval_id)
    }

    pub async fn get_eval_case(&self, id: Uuid) -> Result<Option<EvalCaseRow>> {
        dispatch!(self, get_eval_case, id)
    }

    pub async fn get_eval_case_by_public_id(
        &self,
        eval_id: Uuid,
        public_id: &str,
    ) -> Result<Option<EvalCaseRow>> {
        dispatch!(self, get_eval_case_by_public_id, eval_id, public_id)
    }

    pub async fn update_eval_case(
        &self,
        id: Uuid,
        input: UpdateEvalCaseRow,
    ) -> Result<Option<EvalCaseRow>> {
        dispatch!(self, update_eval_case, id, input)
    }

    pub async fn delete_eval_case(&self, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_eval_case, id)
    }

    pub async fn count_eval_cases(&self, eval_id: Uuid) -> Result<i64> {
        dispatch!(self, count_eval_cases, eval_id)
    }

    pub async fn count_running_eval_runs_for_org(&self, org_id: i64) -> Result<i64> {
        dispatch!(self, count_running_eval_runs_for_org, org_id)
    }

    // ============================================
    // Eval Run CRUD
    // ============================================

    pub async fn create_eval_run(
        &self,
        org_id: i64,
        input: CreateEvalRunRow,
    ) -> Result<EvalRunRow> {
        dispatch!(self, create_eval_run, org_id, input)
    }

    pub async fn create_eval_run_with_case_results(
        &self,
        org_id: i64,
        input: CreateEvalRunRow,
        eval_target: Option<serde_json::Value>,
        max_concurrent_runs_per_org: usize,
        max_cases_per_run: usize,
    ) -> Result<EvalRunRow> {
        dispatch!(
            self,
            create_eval_run_with_case_results,
            org_id,
            input,
            eval_target,
            max_concurrent_runs_per_org,
            max_cases_per_run
        )
    }

    /// Ingest one externally-executed eval run (upsert eval + cases by name,
    /// replace any prior run sharing `source_run_id`, write a completed external
    /// run with fully-populated results). See `ImportEvalRunInput`.
    pub async fn import_eval_run(
        &self,
        org_id: i64,
        input: ImportEvalRunInput,
    ) -> Result<EvalRunRow> {
        dispatch!(self, import_eval_run, org_id, input)
    }

    pub async fn list_eval_runs(&self, eval_id: Uuid) -> Result<Vec<EvalRunRow>> {
        dispatch!(self, list_eval_runs, eval_id)
    }

    pub async fn get_eval_run_by_public_id(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRunRow>> {
        dispatch!(self, get_eval_run_by_public_id, org_id, public_id)
    }

    pub async fn get_eval_run_by_id(&self, id: Uuid) -> Result<Option<EvalRunRow>> {
        dispatch!(self, get_eval_run_by_id, id)
    }

    // Eval run share tokens (migration 091)

    pub async fn create_eval_run_share_token(
        &self,
        org_id: i64,
        input: CreateEvalRunShareTokenRow,
    ) -> Result<EvalRunShareTokenRow> {
        dispatch!(self, create_eval_run_share_token, org_id, input)
    }

    pub async fn revoke_eval_run_share_tokens(
        &self,
        org_id: i64,
        eval_run_id: Uuid,
    ) -> Result<u64> {
        dispatch!(self, revoke_eval_run_share_tokens, org_id, eval_run_id)
    }

    pub async fn eval_run_has_active_share(&self, org_id: i64, eval_run_id: Uuid) -> Result<bool> {
        dispatch!(self, eval_run_has_active_share, org_id, eval_run_id)
    }

    pub async fn get_eval_run_share_token_by_hash(
        &self,
        token_hash: &str,
    ) -> Result<Option<EvalRunShareTokenRow>> {
        dispatch!(self, get_eval_run_share_token_by_hash, token_hash)
    }

    pub async fn update_eval_run_status(
        &self,
        id: Uuid,
        status: &str,
        summary: Option<serde_json::Value>,
    ) -> Result<Option<EvalRunRow>> {
        dispatch!(self, update_eval_run_status, id, status, summary)
    }

    pub async fn get_latest_eval_run(&self, eval_id: Uuid) -> Result<Option<EvalRunRow>> {
        dispatch!(self, get_latest_eval_run, eval_id)
    }

    // ============================================
    // Agent Health Check Runs (knowledge/evaluation/agent-checks.md)
    // ============================================

    pub async fn create_agent_health_check_run(
        &self,
        org_id: i64,
        input: CreateAgentHealthCheckRunRow,
    ) -> Result<AgentHealthCheckRunRow> {
        dispatch!(self, create_agent_health_check_run, org_id, input)
    }

    pub async fn get_agent_health_check_run(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        dispatch!(self, get_agent_health_check_run, org_id, public_id)
    }

    pub async fn list_agent_health_check_runs(
        &self,
        org_id: i64,
        agent_id: Uuid,
        limit: i64,
    ) -> Result<Vec<AgentHealthCheckRunRow>> {
        dispatch!(self, list_agent_health_check_runs, org_id, agent_id, limit)
    }

    pub async fn latest_agent_health_check_run(
        &self,
        org_id: i64,
        agent_id: Uuid,
        config_hash: &str,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        dispatch!(
            self,
            latest_agent_health_check_run,
            org_id,
            agent_id,
            config_hash
        )
    }

    pub async fn update_agent_health_check_run(
        &self,
        id: Uuid,
        input: UpdateAgentHealthCheckRunRow,
    ) -> Result<Option<AgentHealthCheckRunRow>> {
        dispatch!(self, update_agent_health_check_run, id, input)
    }

    pub async fn reap_running_agent_health_check_runs(&self) -> Result<u64> {
        dispatch!(self, reap_running_agent_health_check_runs)
    }

    // ============================================
    // Agent Check Rules (knowledge/evaluation/agent-checks.md, phase 4)
    // ============================================

    pub async fn list_agent_check_rules(&self, org_id: i64) -> Result<Vec<AgentCheckRuleRow>> {
        dispatch!(self, list_agent_check_rules, org_id)
    }

    pub async fn count_custom_agent_check_rules_excluding(
        &self,
        org_id: i64,
        excluded_rule_id: &str,
    ) -> Result<i64> {
        dispatch!(
            self,
            count_custom_agent_check_rules_excluding,
            org_id,
            excluded_rule_id
        )
    }

    pub async fn upsert_agent_check_rule(
        &self,
        org_id: i64,
        input: UpsertAgentCheckRuleRow,
    ) -> Result<AgentCheckRuleRow> {
        dispatch!(self, upsert_agent_check_rule, org_id, input)
    }

    pub async fn delete_agent_check_rule(&self, org_id: i64, rule_id: &str) -> Result<bool> {
        dispatch!(self, delete_agent_check_rule, org_id, rule_id)
    }

    // ============================================
    // Eval Case Result CRUD
    // ============================================

    pub async fn create_eval_case_result(
        &self,
        input: CreateEvalCaseResultRow,
    ) -> Result<EvalCaseResultRow> {
        dispatch!(self, create_eval_case_result, input)
    }

    pub async fn list_eval_case_results(
        &self,
        eval_run_id: Uuid,
    ) -> Result<Vec<EvalCaseResultRow>> {
        dispatch!(self, list_eval_case_results, eval_run_id)
    }

    pub async fn update_eval_case_result(
        &self,
        id: Uuid,
        input: UpdateEvalCaseResultRow,
    ) -> Result<Option<EvalCaseResultRow>> {
        dispatch!(self, update_eval_case_result, id, input)
    }

    // ============================================
    // Eval Run Dataset (async export handles — knowledge/evaluation/dataset-export.md)
    // ============================================

    pub async fn create_eval_run_dataset(
        &self,
        org_id: i64,
        input: CreateEvalRunDatasetRow,
    ) -> Result<(EvalRunDatasetRow, bool)> {
        dispatch!(self, create_eval_run_dataset, org_id, input)
    }

    pub async fn find_eval_run_dataset_by_request(
        &self,
        org_id: i64,
        eval_run_id: Uuid,
        request: &serde_json::Value,
    ) -> Result<Option<EvalRunDatasetRow>> {
        dispatch!(
            self,
            find_eval_run_dataset_by_request,
            org_id,
            eval_run_id,
            request
        )
    }

    pub async fn get_eval_run_dataset(
        &self,
        org_id: i64,
        public_id: &str,
    ) -> Result<Option<EvalRunDatasetRow>> {
        dispatch!(self, get_eval_run_dataset, org_id, public_id)
    }

    pub async fn update_eval_run_dataset(
        &self,
        id: Uuid,
        input: UpdateEvalRunDatasetRow,
    ) -> Result<Option<EvalRunDatasetRow>> {
        dispatch!(self, update_eval_run_dataset, id, input)
    }

    // ============================================
    // Budget CRUD
    // ============================================

    pub async fn create_budget(&self, input: CreateBudgetRow) -> Result<BudgetRow> {
        dispatch!(self, create_budget, input)
    }

    pub async fn get_budget(&self, org_id: i64, id: Uuid) -> Result<Option<BudgetRow>> {
        dispatch!(self, get_budget, org_id, id)
    }

    pub async fn list_budgets(
        &self,
        org_id: i64,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        dispatch!(self, list_budgets, org_id, subject_type, subject_id)
    }

    pub async fn get_active_budgets_for_session(
        &self,
        org_id: i64,
        session_id: &str,
        agent_id: Option<&str>,
        user_id: Option<&str>,
        org_public_id: Option<&str>,
    ) -> Result<Vec<BudgetRow>> {
        dispatch!(
            self,
            get_active_budgets_for_session,
            org_id,
            session_id,
            agent_id,
            user_id,
            org_public_id
        )
    }

    pub async fn get_active_budgets_for_subjects(
        &self,
        org_id: i64,
        lookup: crate::storage::repositories::BudgetSubjectLookup<'_>,
    ) -> Result<Vec<BudgetRow>> {
        dispatch!(self, get_active_budgets_for_subjects, org_id, lookup)
    }

    pub async fn reset_budget_period(
        &self,
        id: Uuid,
        period_started_at: DateTime<Utc>,
    ) -> Result<Option<BudgetRow>> {
        dispatch!(self, reset_budget_period, id, period_started_at)
    }

    pub async fn update_budget(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdateBudgetRow,
    ) -> Result<Option<BudgetRow>> {
        dispatch!(self, update_budget, org_id, id, input)
    }

    pub async fn delete_budget(&self, org_id: i64, id: Uuid) -> Result<bool> {
        dispatch!(self, delete_budget, org_id, id)
    }

    // ============================================
    // Usage Journal + Ledger
    // ============================================

    pub async fn create_usage_journal_entry(
        &self,
        input: CreateUsageJournalRow,
    ) -> Result<UsageJournalRow> {
        dispatch!(self, create_usage_journal_entry, input)
    }

    pub async fn get_usage_journal(&self, id: Uuid) -> Result<Option<UsageJournalRow>> {
        dispatch!(self, get_usage_journal, id)
    }

    pub async fn create_usage_ledger_entry(
        &self,
        input: CreateUsageLedgerRow,
    ) -> Result<(UsageLedgerRow, Option<BudgetRow>)> {
        dispatch!(self, create_usage_ledger_entry, input)
    }

    pub async fn create_budget_ledger_entry(
        &self,
        input: CreateBudgetLedgerRow,
    ) -> Result<(BudgetLedgerRow, BudgetRow)> {
        dispatch!(self, create_budget_ledger_entry, input)
    }

    pub async fn list_usage_ledger_for_budget(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<UsageLedgerRow>> {
        dispatch!(self, list_usage_ledger_for_budget, budget_id, limit, offset)
    }

    pub async fn list_budget_ledger(
        &self,
        budget_id: Uuid,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<BudgetLedgerRow>> {
        dispatch!(self, list_budget_ledger, budget_id, limit, offset)
    }

    pub async fn set_budget_status(&self, id: Uuid, status: &str) -> Result<Option<BudgetRow>> {
        dispatch!(self, set_budget_status, id, status)
    }

    // ============================================
    // Machine payments
    // ============================================

    pub async fn create_payment_account(
        &self,
        org_id: i64,
        input: CreatePaymentAccountRow,
    ) -> Result<PaymentAccountRow> {
        dispatch!(self, create_payment_account, org_id, input)
    }

    pub async fn list_payment_accounts(
        &self,
        org_id: i64,
        owner_type: Option<&str>,
        owner_id: Option<&str>,
    ) -> Result<Vec<PaymentAccountRow>> {
        dispatch!(self, list_payment_accounts, org_id, owner_type, owner_id)
    }

    pub async fn get_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentAccountRow>> {
        dispatch!(self, get_payment_account, org_id, id)
    }

    pub async fn update_payment_account(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentAccountRow,
    ) -> Result<Option<PaymentAccountRow>> {
        dispatch!(self, update_payment_account, org_id, id, input)
    }

    pub async fn create_payment_policy(
        &self,
        org_id: i64,
        input: CreatePaymentPolicyRow,
    ) -> Result<PaymentPolicyRow> {
        dispatch!(self, create_payment_policy, org_id, input)
    }

    pub async fn list_payment_policies(
        &self,
        org_id: i64,
        payment_account_id: Option<Uuid>,
        subject_type: Option<&str>,
        subject_id: Option<&str>,
    ) -> Result<Vec<PaymentPolicyRow>> {
        dispatch!(
            self,
            list_payment_policies,
            org_id,
            payment_account_id,
            subject_type,
            subject_id
        )
    }

    pub async fn get_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
    ) -> Result<Option<PaymentPolicyRow>> {
        dispatch!(self, get_payment_policy, org_id, id)
    }

    pub async fn update_payment_policy(
        &self,
        org_id: i64,
        id: Uuid,
        input: UpdatePaymentPolicyRow,
    ) -> Result<Option<PaymentPolicyRow>> {
        dispatch!(self, update_payment_policy, org_id, id, input)
    }

    pub async fn list_payment_attempts(
        &self,
        org_id: i64,
        session_id: Option<Uuid>,
        limit: i64,
    ) -> Result<Vec<PaymentAttemptRow>> {
        dispatch!(self, list_payment_attempts, org_id, session_id, limit)
    }

    pub async fn create_payment_attempt(
        &self,
        org_id: i64,
        input: CreatePaymentAttemptRow,
    ) -> Result<PaymentAttemptRow> {
        dispatch!(self, create_payment_attempt, org_id, input)
    }
}
