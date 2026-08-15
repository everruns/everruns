use super::types::CreateBudgetRequest;
use super::{BUDGET_MANAGE, BUDGET_VIEW, queries as q};
use crate::domains::common::*;
use crate::storage::models::{CreateBudgetLedgerRow, CreateBudgetRow, UpdateBudgetRow};
use everruns_core::Policy;
use everruns_core::budget::BudgetCheckResult;
use everruns_platform::{Budget, LedgerEntry};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

fn validate_subject_type(ctx: &Ctx, subject_type: &str) -> Result<(), CommandError> {
    const ALWAYS_ON: &[&str] = &["session", "agent", "user", "org"];
    const APP_BUDGET_TYPES: &[&str] = &["app", "app_channel"];
    if ALWAYS_ON.contains(&subject_type) {
        return Ok(());
    }
    if APP_BUDGET_TYPES.contains(&subject_type) {
        if ctx.feature_flags.app_budgets {
            return Ok(());
        }
        return Err(CommandError::feature_not_enabled("app_budgets"));
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

#[derive(Debug, Deserialize)]
pub struct CreateBudget(pub CreateBudgetRequest);

impl CommandSchema for CreateBudget {
    fn param_schema() -> serde_json::Value {
        delegated_param_schema::<CreateBudgetRequest>()
    }
}

impl Command for CreateBudget {
    type Output = Budget;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "create_budget",
            category: "budgets",
            description: "Create a budget for a subject (session, agent, user, org). Sets a spending cap in the given currency.",
            method: "POST",
            path: "/v1/budgets",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&BUDGET_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        require_budget_manage(ctx)?;
        let req = self.0;
        validate_subject_type(ctx, &req.subject_type)?;
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
        let row = ctx.db.create_budget(input).await.map_err(classify_anyhow)?;
        Ok(q::row_to_budget(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<CreateBudget>() }

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct ListBudgets {
    pub subject_type: Option<String>,
    pub subject_id: Option<String>,
}

impl Command for ListBudgets {
    type Output = Vec<Budget>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_budgets",
            category: "budgets",
            description: "List budgets. Filter by subject_type and subject_id.",
            method: "GET",
            path: "/v1/budgets",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&BUDGET_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Budget>, CommandError> {
        let rows = ctx
            .db
            .list_budgets(
                ctx.org_id(),
                self.subject_type.as_deref(),
                self.subject_id.as_deref(),
            )
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_budget).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListBudgets>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct GetBudget {
    pub budget_id: String,
}

impl Command for GetBudget {
    type Output = Budget;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "get_budget",
            category: "budgets",
            description: "Get a single budget by ID.",
            method: "GET",
            path: "/v1/budgets/{budget_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("budget_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&BUDGET_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let row = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        Ok(q::row_to_budget(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<GetBudget>() }

#[derive(Debug, Deserialize, ToSchema)]
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

impl Command for UpdateBudgetCmd {
    type Output = Budget;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "update_budget",
            category: "budgets",
            description: "Update a budget limit, status, or metadata.",
            method: "PATCH",
            path: "/v1/budgets/{budget_id}",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&BUDGET_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        require_budget_manage(ctx)?;
        if let Some(limit) = self.limit {
            validate_limit(limit, self.soft_limit.flatten())?;
        }

        let budget_id = q::parse_budget_id(&self.budget_id)?;
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
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        Ok(q::row_to_budget(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<UpdateBudgetCmd>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct DeleteBudget {
    pub budget_id: String,
}

impl Command for DeleteBudget {
    type Output = BudgetDeleteResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "delete_budget",
            category: "budgets",
            description: "Delete a budget.",
            method: "DELETE",
            path: "/v1/budgets/{budget_id}",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("budget_id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&BUDGET_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<BudgetDeleteResult, CommandError> {
        require_budget_manage(ctx)?;
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let deleted = ctx
            .db
            .delete_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?;
        if !deleted {
            return Err(CommandError::not_found("Budget"));
        }
        Ok(BudgetDeleteResult { deleted })
    }
}

inventory::submit! { CommandDescriptor::of::<DeleteBudget>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct TopUpBudget {
    pub budget_id: String,
    pub amount: f64,
    /// Human-readable description. Safe to render in user-facing messages.
    pub description: Option<String>,
}

impl Command for TopUpBudget {
    type Output = Budget;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "top_up_budget",
            category: "budgets",
            description: "Add credits to a budget. Reactivates exhausted or paused budgets if balance becomes positive.",
            method: "POST",
            path: "/v1/budgets/{budget_id}/top-up",
        }
    }

    fn policy() -> Option<&'static Policy> {
        Some(&BUDGET_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Budget, CommandError> {
        require_budget_manage(ctx)?;
        if self.amount <= 0.0 {
            return Err(CommandError::bad_request("Amount must be positive"));
        }

        let budget_id = q::parse_budget_id(&self.budget_id)?;
        ctx.db
            .get_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;

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
            .await
            .map_err(classify_anyhow)?;

        if updated.balance > 0.0 && (updated.status == "paused" || updated.status == "exhausted") {
            let _ = ctx.db.set_budget_status(budget_id, "active").await;
        }

        let row = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        Ok(q::row_to_budget(&row))
    }
}

inventory::submit! { CommandDescriptor::of::<TopUpBudget>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListBudgetLedger {
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

impl Command for ListBudgetLedger {
    type Output = Vec<LedgerEntry>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_budget_ledger",
            category: "budgets",
            description: "List ledger entries for a budget.",
            method: "GET",
            path: "/v1/budgets/{budget_id}/ledger",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&BUDGET_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<LedgerEntry>, CommandError> {
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        ctx.db
            .get_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Budget"))?;
        let rows = ctx
            .db
            .list_budget_ledger(budget_id, self.limit, self.offset)
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_ledger_entry).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListBudgetLedger>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckBudget {
    pub budget_id: String,
}

impl Command for CheckBudget {
    type Output = BudgetCheckResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "check_budget",
            category: "budgets",
            description: "Check budget status for a session-scoped budget.",
            method: "GET",
            path: "/v1/budgets/{budget_id}/check",
        }
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&BUDGET_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<BudgetCheckResult, CommandError> {
        let budget_id = q::parse_budget_id(&self.budget_id)?;
        let budget = ctx
            .db
            .get_budget(ctx.org_id(), budget_id)
            .await
            .map_err(classify_anyhow)?
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

inventory::submit! { CommandDescriptor::of::<CheckBudget>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListSessionBudgets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for ListSessionBudgets {
    type Output = Vec<Budget>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_session_budgets",
            category: "budgets",
            description: "List all budgets for a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/budgets",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&BUDGET_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Budget>, CommandError> {
        let rows = ctx
            .db
            .list_budgets(ctx.org_id(), Some("session"), Some(&self.session_id))
            .await
            .map_err(classify_anyhow)?;
        Ok(rows.iter().map(q::row_to_budget).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListSessionBudgets>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct CheckSessionBudgets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for CheckSessionBudgets {
    type Output = BudgetCheckResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "check_session_budgets",
            category: "budgets",
            description: "Check all budgets for a session.",
            method: "GET",
            path: "/v1/sessions/{session_id}/budget-check",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&BUDGET_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<BudgetCheckResult, CommandError> {
        Ok(crate::domains::budgets::BudgetService::new(ctx.db.clone())
            .check_budgets_for_session(ctx.org_id(), &self.session_id, None)
            .await)
    }
}

inventory::submit! { CommandDescriptor::of::<CheckSessionBudgets>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ResumeSessionBudgets {
    /// Session's prefixed public identifier.
    pub session_id: String,
}

impl Command for ResumeSessionBudgets {
    type Output = ResumeSessionBudgetsResult;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "resume_session_budgets",
            category: "budgets",
            description: "Resume all paused session budgets for a session.",
            method: "POST",
            path: "/v1/sessions/{session_id}/resume",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("session_id")
    }

    fn policy() -> Option<&'static Policy> {
        Some(&BUDGET_MANAGE)
    }

    async fn execute(self, ctx: &Ctx) -> Result<ResumeSessionBudgetsResult, CommandError> {
        require_budget_manage(ctx)?;
        let budgets = ctx
            .db
            .list_budgets(ctx.org_id(), Some("session"), Some(&self.session_id))
            .await
            .map_err(classify_anyhow)?;

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

inventory::submit! { CommandDescriptor::of::<ResumeSessionBudgets>() }

#[derive(Debug, Deserialize, ToSchema)]
pub struct ListAppBudgets {
    /// App's prefixed public identifier.
    pub app_id: String,
    /// When true include budgets attached to any of the app's channels
    /// (`app_channel:<id>`) in addition to the app itself.
    // Bashkit's MCP flag parser forwards bools as JSON strings ("true"/"false"),
    // so the lenient deserializer is required to accept `--include_channels true`.
    #[serde(
        default = "default_true",
        deserialize_with = "deserialize_bool_lenient"
    )]
    pub include_channels: bool,
}

const fn default_true() -> bool {
    true
}

impl Command for ListAppBudgets {
    type Output = Vec<Budget>;

    fn meta() -> CommandMeta {
        CommandMeta {
            name: "list_app_budgets",
            category: "budgets",
            description: "List budgets attached to an app and (optionally) its channels.",
            method: "GET",
            path: "/v1/apps/{app_id}/budgets",
        }
    }

    fn positional_arg() -> Option<&'static str> {
        Some("app_id")
    }

    fn policy() -> Option<&'static everruns_core::Policy> {
        Some(&BUDGET_VIEW)
    }

    async fn execute(self, ctx: &Ctx) -> Result<Vec<Budget>, CommandError> {
        if !ctx.feature_flags.app_budgets {
            return Err(CommandError::feature_not_enabled("app_budgets"));
        }

        // THREAT[TM-TENANT-001]: org-scoped lookup before reading any channel
        // metadata — without this an attacker could pass another org's app_id
        // and learn its channel public_ids via the include_channels expansion
        // (the budget query that follows is org-filtered, but the channel list
        // would still leak). 404 (not 403) keeps cross-org existence opaque.
        let app_row = ctx
            .db
            .get_app_by_public_id(ctx.org_id(), &self.app_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("App"))?;

        let mut rows = ctx
            .db
            .list_budgets(ctx.org_id(), Some("app"), Some(&self.app_id))
            .await
            .map_err(classify_anyhow)?;

        if self.include_channels {
            let channels = ctx
                .db
                .list_app_channels(app_row.id)
                .await
                .map_err(classify_anyhow)?;
            for channel in channels {
                let channel_rows = ctx
                    .db
                    .list_budgets(ctx.org_id(), Some("app_channel"), Some(&channel.public_id))
                    .await
                    .map_err(classify_anyhow)?;
                rows.extend(channel_rows);
            }
        }

        Ok(rows.iter().map(q::row_to_budget).collect())
    }
}

inventory::submit! { CommandDescriptor::of::<ListAppBudgets>() }

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
        let db = Arc::new(StorageBackend::in_memory());
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

    /// `validate_subject_type` is the gate. It rejects unknown types and requires
    /// the `app_budgets` feature flag for `app` / `app_channel`. We test the
    /// always-on validation explicitly; flag-gated paths are tested via the env
    /// var seam below.
    #[test]
    fn validate_subject_type_accepts_classic_subjects() {
        for kind in ["session", "agent", "user", "org"] {
            assert!(
                validate_subject_type(&ctx_for_role(OrgRole::Owner), kind).is_ok(),
                "expected {kind} ok"
            );
        }
    }

    #[test]
    fn validate_subject_type_rejects_unknown_subjects() {
        let ctx = ctx_for_role(OrgRole::Owner);
        assert!(validate_subject_type(&ctx, "unknown").is_err());
        assert!(validate_subject_type(&ctx, "").is_err());
    }

    #[test]
    fn validate_subject_type_app_budgets_honor_org_flag() {
        // Default (flag off): app-scoped budget subjects are rejected.
        let disabled = ctx_for_role(OrgRole::Owner);
        assert!(validate_subject_type(&disabled, "app").is_err());
        assert!(validate_subject_type(&disabled, "app_channel").is_err());

        // Flag on (org opted in): app-scoped subjects are accepted.
        let enabled =
            ctx_for_role(OrgRole::Owner).with_feature_flags(everruns_platform::FeatureFlags {
                app_budgets: true,
                ..Default::default()
            });
        assert!(validate_subject_type(&enabled, "app").is_ok());
        assert!(validate_subject_type(&enabled, "app_channel").is_ok());
    }
}
