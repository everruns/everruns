use super::types::CreateBudgetRequest;
use super::{BUDGET_MANAGE, BUDGET_VIEW, queries as q};
use crate::domains::budgets::record::{Budget, LedgerEntry};
use crate::domains::common::*;
use crate::storage::{CreateBudgetLedgerRow, CreateBudgetRow, UpdateBudgetRow};
use everruns_core::budget::BudgetCheckResult;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

fn validate_subject_type(subject_type: &str) -> Result<(), CommandError> {
    const SUPPORTED: &[&str] = &[
        "session",
        "agent",
        "user",
        "org",
        "agent_trigger",
        "agent_channel",
    ];
    if SUPPORTED.contains(&subject_type) {
        return Ok(());
    }
    Err(CommandError::bad_request("Invalid subject_type"))
}

fn validate_limit(limit: f64, soft_limit: Option<f64>) -> Result<(), CommandError> {
    if limit <= 0.0 {
        return Err(CommandError::bad_request("Limit must be positive"));
    }
    if soft_limit.is_some_and(|s| s <= 0.0 || s > limit) {
        return Err(CommandError::bad_request(
            "Soft limit must be between 0 and limit",
        ));
    }
    Ok(())
}

fn require_budget_manage(ctx: &Ctx) -> Result<(), CommandError> {
    BUDGET_MANAGE
        .evaluate(&ctx.caller)
        .map_err(|e| CommandError::forbidden(e.message))
}

#[derive(Debug, Serialize)]
pub struct BudgetDeleteResult {
    pub deleted: bool,
}

/// Result of resuming budgets paused for a session.
#[derive(Debug, Serialize, ToSchema)]
#[schema(as = ResumeSessionResponse)]
pub struct ResumeSessionBudgetsResult {
    pub resumed_budgets: usize,
    pub session_id: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
pub struct CreateBudget(pub CreateBudgetRequest);

impl CommandSchema for CreateBudget {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateBudgetRequest>()
    }
}

#[command(
    name = "create_budget",
    category = "budgets",
    description = "Create a budget for a subject (session, agent, user, org). Sets a spending cap in the given currency.",
    method = "POST",
    path = "/v1/budgets",
    policy = BUDGET_MANAGE,
    http = created_with_urls,
    request_body(CreateBudgetRequest),
)]
impl Command for CreateBudget {
    type Output = Budget;

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        require_budget_manage(ctx)?;
        let mut req = self.0;
        if req.subject_type == "agent_endpoint" {
            req.subject_type = "agent_channel".into();
        }
        validate_subject_type(&req.subject_type)?;
        validate_limit(req.limit, req.soft_limit)?;

        let input = CreateBudgetRow {
            org_id: ctx.org_id(),
            subject_type: req.subject_type,
            subject_id: req.subject_id,
            currency: req.currency,
            limit: req.limit,
            soft_limit: req.soft_limit,
            period: req
                .period
                .map(|period| serde_json::to_value(period).unwrap_or_default()),
            metadata: req.metadata,
        };
        let row = ctx.db.create_budget(input).await?;
        Ok(Budget::from(&row))
    }
}

#[derive(Debug, Default, Deserialize, ToSchema, IntoParams, serde::Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListBudgets {
    /// Only budgets on this kind of subject (session, agent, user, org, ...).
    pub subject_type: Option<String>,
    /// Only budgets on this subject's prefixed public identifier.
    pub subject_id: Option<String>,
}

#[command(
    name = "list_budgets",
    category = "budgets",
    description = "List budgets. Filter by subject_type and subject_id.",
    method = "GET",
    path = "/v1/budgets",
    policy = BUDGET_VIEW,
    http = vec_with_urls,
    params(ListBudgets),
)]
impl Command for ListBudgets {
    type Output = Vec<Budget>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Budget>, CommandError> {
        let rows = ctx
            .db
            .list_budgets(
                ctx.org_id(),
                self.subject_type.as_deref().map(|subject| {
                    if subject == "agent_endpoint" {
                        "agent_channel"
                    } else {
                        subject
                    }
                }),
                self.subject_id.as_deref(),
            )
            .await?;
        Ok(rows.iter().map(Budget::from).collect())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct GetBudget {
    pub budget_id: String,
}

#[command(
    name = "get_budget",
    category = "budgets",
    description = "Get a single budget by ID.",
    method = "GET",
    path = "/v1/budgets/{budget_id}",
    policy = BUDGET_VIEW,
    positional = "budget_id",
    http = with_urls,
    responses((status = 404, description = "Budget not found")),
)]
impl Command for GetBudget {
    type Output = Budget;

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let row = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        Ok(Budget::from(&row))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct UpdateBudgetCmd {
    pub budget_id: String,
    /// Maximum number of items returned in this page.
    pub limit: Option<f64>,
    pub soft_limit: Option<Option<f64>>,
    /// Current lifecycle status.
    pub status: Option<String>,
    /// Free-form metadata attached to this resource.
    pub metadata: Option<serde_json::Value>,
}

#[command(
    name = "update_budget",
    category = "budgets",
    description = "Update a budget limit, status, or metadata.",
    method = "PATCH",
    path = "/v1/budgets/{budget_id}",
    policy = BUDGET_MANAGE,
    http = with_urls,
    request_body(crate::domains::budgets::types::UpdateBudgetRequest),
    responses((status = 404, description = "Budget not found")),
)]
impl Command for UpdateBudgetCmd {
    type Output = Budget;

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        require_budget_manage(ctx)?;
        if let Some(limit) = self.limit {
            validate_limit(limit, self.soft_limit.flatten())?;
        }

        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let existing = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        validate_subject_type(&existing.subject_type)?;
        let row = ctx
            .db
            .update_budget(
                ctx.org_id(),
                budget_id,
                UpdateBudgetRow {
                    limit: self.limit,
                    soft_limit: self.soft_limit,
                    status: self.status,
                    metadata: self.metadata,
                },
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        Ok(Budget::from(&row))
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct DeleteBudget {
    pub budget_id: String,
}

#[command(
    name = "delete_budget",
    category = "budgets",
    description = "Delete a budget.",
    method = "DELETE",
    path = "/v1/budgets/{budget_id}",
    policy = BUDGET_MANAGE,
    positional = "budget_id",
    http = no_content,
    responses((status = 404, description = "Budget not found")),
)]
impl Command for DeleteBudget {
    type Output = BudgetDeleteResult;

    async fn execute(self, ctx: &Ctx) -> Result<BudgetDeleteResult, CommandError> {
        require_budget_manage(ctx)?;
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let existing = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        validate_subject_type(&existing.subject_type)?;
        let deleted = ctx.db.delete_budget(ctx.org_id(), budget_id).await?;
        if !deleted {
            return Err(CommandError::not_found("Budget"));
        }
        Ok(BudgetDeleteResult { deleted })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct TopUpBudget {
    pub budget_id: String,
    pub amount: f64,
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
}

#[command(
    name = "top_up_budget",
    category = "budgets",
    description = "Add credits to a budget. Reactivates exhausted or paused budgets if balance becomes positive.",
    method = "POST",
    path = "/v1/budgets/{budget_id}/top-up",
    policy = BUDGET_MANAGE,
    http = with_urls,
    request_body(crate::domains::budgets::types::TopUpRequest),
    responses((status = 404, description = "Budget not found")),
)]
impl Command for TopUpBudget {
    type Output = Budget;

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        require_budget_manage(ctx)?;
        if self.amount <= 0.0 {
            return Err(CommandError::bad_request("Amount must be positive"));
        }

        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let existing = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        validate_subject_type(&existing.subject_type)?;

        let (_entry, updated) = ctx
            .db
            .create_budget_ledger_entry(CreateBudgetLedgerRow {
                budget_id,
                amount: -self.amount,
                meter_source: "manual".into(),
                ref_type: Some("top_up".into()),
                ref_id: None,
                session_id: None,
                description: self.description,
            })
            .await?;

        if updated.balance > 0.0 && (updated.status == "paused" || updated.status == "exhausted") {
            let _ = ctx.db.set_budget_status(budget_id, "active").await;
        }

        let row = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        Ok(Budget::from(&row))
    }
}

#[derive(Debug, Deserialize, ToSchema, IntoParams, serde::Serialize)]
#[into_params(parameter_in = Query)]
pub struct ListBudgetLedger {
    /// Budget's prefixed public identifier (a path parameter).
    #[param(ignore)]
    pub budget_id: String,
    #[serde(default = "default_budget_ledger_limit")]
    /// Maximum number of items returned in this page.
    pub limit: i64,
    #[serde(default)]
    /// Zero-based offset into the result set.
    pub offset: i64,
}

const fn default_budget_ledger_limit() -> i64 {
    50
}

#[command(
    name = "list_budget_ledger",
    category = "budgets",
    description = "List ledger entries for a budget.",
    method = "GET",
    path = "/v1/budgets/{budget_id}/ledger",
    policy = BUDGET_VIEW,
    http = plain,
    params(ListBudgetLedger),
)]
impl Command for ListBudgetLedger {
    type Output = Vec<LedgerEntry>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<LedgerEntry>, CommandError> {
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        ctx.db
            .get_budget(ctx.org_id(), budget_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        let rows = ctx
            .db
            .list_budget_ledger(budget_id, self.limit, self.offset)
            .await?;
        Ok(rows.iter().map(LedgerEntry::from).collect())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CheckBudget {
    pub budget_id: String,
}

#[command(
    name = "check_budget",
    category = "budgets",
    description = "Check budget status for a session-scoped budget.",
    method = "GET",
    path = "/v1/budgets/{budget_id}/check",
    policy = BUDGET_VIEW,
    http = plain,
)]
impl Command for CheckBudget {
    type Output = BudgetCheckResult;

    async fn execute(self, ctx: &Ctx) -> Result<BudgetCheckResult, CommandError> {
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let budget = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        if budget.subject_type != "session" {
            return Err(CommandError::bad_request(
                "Budget check only supported for session-scoped budgets via this endpoint",
            ));
        }
        Ok(crate::domains::budgets::BudgetService::new(ctx.db.clone())
            .check_budgets_for_session(ctx.org_id(), &budget.subject_id, None)
            .await)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListSessionBudgets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "list_session_budgets",
    category = "budgets",
    description = "List all budgets for a session.",
    method = "GET",
    path = "/v1/sessions/{session_id}/budgets",
    policy = BUDGET_VIEW,
    positional = "session_id",
    http = vec_with_urls,
)]
impl Command for ListSessionBudgets {
    type Output = Vec<Budget>;

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Budget>, CommandError> {
        let rows = ctx
            .db
            .list_budgets(ctx.org_id(), Some("session"), Some(&self.session_id))
            .await?;
        Ok(rows.iter().map(Budget::from).collect())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CheckSessionBudgets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "check_session_budgets",
    category = "budgets",
    description = "Check all budgets for a session.",
    method = "GET",
    path = "/v1/sessions/{session_id}/budget-check",
    policy = BUDGET_VIEW,
    positional = "session_id",
    http = plain,
)]
impl Command for CheckSessionBudgets {
    type Output = BudgetCheckResult;

    async fn execute(self, ctx: &Ctx) -> Result<BudgetCheckResult, CommandError> {
        Ok(crate::domains::budgets::BudgetService::new(ctx.db.clone())
            .check_budgets_for_session(ctx.org_id(), &self.session_id, None)
            .await)
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ResumeSessionBudgets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

#[command(
    name = "resume_session_budgets",
    category = "budgets",
    description = "Resume all paused session budgets for a session.",
    method = "POST",
    path = "/v1/sessions/{session_id}/resume",
    policy = BUDGET_MANAGE,
    positional = "session_id",
    http = plain,
)]
impl Command for ResumeSessionBudgets {
    type Output = ResumeSessionBudgetsResult;

    async fn execute(self, ctx: &Ctx) -> Result<ResumeSessionBudgetsResult, CommandError> {
        require_budget_manage(ctx)?;
        let budgets = ctx
            .db
            .list_budgets(ctx.org_id(), Some("session"), Some(&self.session_id))
            .await?;

        let mut resumed_budgets = 0;
        for budget in &budgets {
            if budget.status == "paused"
                && let Ok(Some(_)) = ctx.db.set_budget_status(budget.id, "active").await
            {
                resumed_budgets += 1;
            }
        }

        Ok(ResumeSessionBudgetsResult {
            resumed_budgets,
            session_id: self.session_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::CapabilityService;
    use crate::storage::StorageBackend;
    use everruns_core::{Caller, DEFAULT_ORG_ID, organization::OrgRole};
    use std::sync::Arc;
    use uuid::Uuid;

    fn caller_with_role(role: OrgRole) -> Caller {
        Caller {
            org_id: DEFAULT_ORG_ID,
            org_public_id: "org_00000000000000000000000000000001".to_string(),
            user_id: Some(Uuid::nil()),
            role,
            is_platform_user: false,
            is_internal: false,
        }
    }

    fn ctx_for_role(role: OrgRole) -> Ctx {
        let db = Arc::new(StorageBackend::test_database());
        let capability_service = Arc::new(CapabilityService::new(db.clone(), None));
        Ctx::new(
            caller_with_role(role),
            db,
            capability_service,
            None,
            Arc::new(everruns_core::DefaultPermissionResolver),
        )
    }

    #[tokio::test]
    async fn create_budget_denies_member() {
        let ctx = ctx_for_role(OrgRole::Member);
        let err = CreateBudget(CreateBudgetRequest {
            subject_type: "session".to_string(),
            subject_id: "session_123".to_string(),
            currency: "usd".to_string(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .execute(&ctx)
        .await
        .expect_err("member should be denied by BUDGET_MANAGE");
        assert!(matches!(
            err,
            CommandError {
                kind: CommandErrorKind::Forbidden(_),
                ..
            }
        ));
    }

    #[tokio::test]
    async fn create_budget_allows_owner() {
        let ctx = ctx_for_role(OrgRole::Owner);
        let budget = CreateBudget(CreateBudgetRequest {
            subject_type: "session".to_string(),
            subject_id: "session_123".to_string(),
            currency: "usd".to_string(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .execute(&ctx)
        .await
        .expect("owner should pass BUDGET_MANAGE");
        assert_eq!(budget.subject_type.to_string(), "session");
    }

    #[test]
    fn validate_subject_type_accepts_supported_subjects() {
        for kind in [
            "session",
            "agent",
            "user",
            "org",
            "agent_trigger",
            "agent_channel",
        ] {
            assert!(validate_subject_type(kind).is_ok(), "expected {kind} ok");
        }
    }

    #[test]
    fn validate_subject_type_rejects_unknown_subjects() {
        assert!(validate_subject_type("unknown").is_err());
        assert!(validate_subject_type("").is_err());
    }

    #[test]
    fn validate_subject_type_rejects_retired_app_subjects() {
        assert!(validate_subject_type("app").is_err());
        assert!(validate_subject_type("app_channel").is_err());
    }

    #[tokio::test]
    async fn create_budget_rejects_retired_app_subjects() {
        let ctx = ctx_for_role(OrgRole::Owner);
        for subject_type in ["app", "app_channel"] {
            let err = CreateBudget(CreateBudgetRequest {
                subject_type: subject_type.to_string(),
                subject_id: "retired".to_string(),
                currency: "usd".to_string(),
                limit: 10.0,
                soft_limit: None,
                period: None,
                metadata: None,
            })
            .execute(&ctx)
            .await
            .expect_err("retired App budget subjects must remain invalid");
            assert!(matches!(err.kind, CommandErrorKind::BadRequest(_)));
        }
    }

    #[tokio::test]
    async fn agent_channel_budget_remains_creatable_and_updatable() {
        let ctx = ctx_for_role(OrgRole::Owner);
        let budget = CreateBudget(CreateBudgetRequest {
            subject_type: "agent_channel".to_string(),
            subject_id: "endpoint".to_string(),
            currency: "usd".to_string(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .execute(&ctx)
        .await
        .expect("agent channel budget remains supported");

        let updated = UpdateBudgetCmd {
            budget_id: budget.id.to_string(),
            limit: Some(20.0),
            soft_limit: None,
            status: None,
            metadata: None,
        }
        .execute(&ctx)
        .await
        .expect("agent channel budget remains writable");
        assert_eq!(updated.limit, 20.0);
    }

    /// The successor level has a write path. `app_channel` never did — it was
    /// only ever reachable through migration 138 — which is why an operator
    /// could not cap a webhook trigger by hand before EVE-1138.
    #[tokio::test]
    async fn agent_trigger_budget_is_creatable_and_updatable() {
        let ctx = ctx_for_role(OrgRole::Owner);
        let budget = CreateBudget(CreateBudgetRequest {
            subject_type: "agent_trigger".to_string(),
            subject_id: "trg_0199a1b2c3d44e5f8091a2b3c4d5e6f7".to_string(),
            currency: "usd".to_string(),
            limit: 10.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .execute(&ctx)
        .await
        .expect("agent trigger budgets are creatable");

        let updated = UpdateBudgetCmd {
            budget_id: budget.id.to_string(),
            limit: Some(20.0),
            soft_limit: None,
            status: None,
            metadata: None,
        }
        .execute(&ctx)
        .await
        .expect("agent trigger budgets are writable");
        assert_eq!(updated.limit, 20.0);
    }
}
