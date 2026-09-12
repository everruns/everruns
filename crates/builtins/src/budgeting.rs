// Budgeting Capability
//
// Adds budget awareness to agents:
// 1. System prompt section informing the agent about budget constraints
// 2. A check_budget tool to query remaining balance
//
// The actual budget data is injected into the system prompt at runtime
// by the capability's system_prompt_contribution() method, which the
// service layer calls with budget data from the database.
//
// See knowledge/security/budgeting.md (Phase 4: Agent awareness)

use super::{Capability, CapabilityLocalization, CapabilityStatus};
use crate::tool_types::ToolHints;
use crate::tools::{Tool, ToolExecutionResult};
use async_trait::async_trait;
use everruns_core::tool_context::ToolContext;
use serde_json::Value;

pub const BUDGETING_CAPABILITY_ID: &str = "budgeting";

/// Budgeting capability — budget-aware agent behavior.
pub struct BudgetingCapability;

impl Capability for BudgetingCapability {
    fn id(&self) -> &str {
        BUDGETING_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Budgeting"
    }

    fn description(&self) -> &str {
        "Enables budget awareness. The agent receives information about active budgets \
         and can check remaining balance. When budget is running low, the agent will \
         prioritize completing current tasks efficiently."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Бюджетування",
            "Вмикає обізнаність про бюджет. Агент отримує інформацію про активні бюджети й може перевіряти залишок коштів. Коли бюджет добігає кінця, агент надає пріоритет ефективному завершенню поточних завдань.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("wallet")
    }

    fn category(&self) -> Option<&str> {
        Some("System")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(BUDGET_SYSTEM_PROMPT)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(CheckBudgetTool)]
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["budgeting"]
    }
}

const BUDGET_SYSTEM_PROMPT: &str = "This session may have enforced budgets. Check budget before expensive work; when remaining budget is low, finish the core task efficiently and avoid unnecessary output. Exhaustion may pause or stop the session.";

// ============================================================================
// Tool: check_budget
// ============================================================================

/// Tool that returns budget status for the current session.
///
/// In practice, the tool result is populated by the worker which has
/// access to the BudgetService. This implementation returns a placeholder
/// that guides the agent to check via the session context.
pub struct CheckBudgetTool;

#[async_trait]
impl Tool for CheckBudgetTool {
    fn narrate(
        &self,
        _tool_call: &crate::tool_types::ToolCall,
        phase: crate::tool_narration::ToolNarrationPhase,
        locale: Option<&str>,
        _ctx: crate::tool_narration::ToolNarrationContext<'_>,
    ) -> Option<String> {
        Some(crate::tool_narration::narrate_check_budget(phase, locale))
    }

    fn name(&self) -> &str {
        "check_budget"
    }

    fn display_name(&self) -> Option<&str> {
        Some("Check Budget")
    }

    fn description(&self) -> &str {
        "Check the remaining budget for this session. Returns budget balance, limit, currency, and status."
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        // Fallback when no context is available (shouldn't happen in practice).
        // Returns the same BudgetToolResponse shape for contract stability.
        ToolExecutionResult::success(serde_json::json!({
            "status": "no_budgets",
            "budgets": [],
            "hint": "No budgets are configured for this session. You can proceed without budget constraints."
        }))
    }

    async fn execute_with_context(
        &self,
        _arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let Some(ref checker) = context.budget_checker else {
            // No budget checker wired — return the no_budgets fallback.
            return self.execute(_arguments).await;
        };

        let session_id = context.session_id.to_string();

        match checker.check_budgets(&session_id).await {
            Ok(response) => {
                ToolExecutionResult::success(serde_json::to_value(&response).unwrap_or_else(
                    |_| serde_json::json!({"status": "no_budgets", "budgets": [], "hint": null}),
                ))
            }
            Err(_) => ToolExecutionResult::tool_error(
                "Budget check is temporarily unavailable. You can proceed normally.",
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::typed_id::SessionId;
    use serde_json::json;

    #[tokio::test]
    async fn registered_budget_tool_returns_same_complete_fallback_without_checker() {
        let tools = BudgetingCapability.tools();
        assert_eq!(tools.len(), 1);
        let context = ToolContext::new(SessionId::new());
        for result in [
            tools[0].execute(json!({})).await,
            tools[0].execute_with_context(json!({}), &context).await,
        ] {
            let ToolExecutionResult::Success(value) = result else {
                panic!("expected fallback")
            };
            assert_eq!(
                value,
                json!({"status":"no_budgets","budgets":[],"hint":"No budgets are configured for this session. You can proceed without budget constraints."})
            );
        }
    }

    #[tokio::test]
    async fn budget_tool_passes_session_and_preserves_response_or_hides_checker_error() {
        use crate::budget::{BudgetSummary, BudgetToolResponse};
        use everruns_core::tool_execution::BudgetChecker;
        use std::sync::{Arc, Mutex};
        struct Checker {
            response: Option<BudgetToolResponse>,
            sessions: Arc<Mutex<Vec<String>>>,
        }
        #[async_trait]
        impl BudgetChecker for Checker {
            async fn check_budgets(
                &self,
                session_id: &str,
            ) -> crate::error::Result<BudgetToolResponse> {
                self.sessions.lock().unwrap().push(session_id.into());
                self.response.clone().ok_or_else(|| {
                    crate::error::AgentLoopError::store("private checker diagnostic")
                })
            }
        }
        for status in [Some("active"), Some("warning"), Some("exhausted"), None] {
            let sessions = Arc::new(Mutex::new(vec![]));
            let mut context = ToolContext::new(SessionId::new());
            context.budget_checker = Some(Arc::new(Checker {
                sessions: sessions.clone(),
                response: status.map(|status| BudgetToolResponse {
                    status: status.into(),
                    budgets: vec![
                        BudgetSummary {
                            currency: "usd".into(),
                            limit: 5.0,
                            balance: 2.5,
                            soft_limit: Some(1.0),
                            percent_remaining: 50.0,
                            status: status.into(),
                        },
                        BudgetSummary {
                            currency: "tokens".into(),
                            limit: 100.0,
                            balance: 20.0,
                            soft_limit: None,
                            percent_remaining: 20.0,
                            status: "warning".into(),
                        },
                    ],
                    hint: Some("budget guidance".into()),
                }),
            }));
            let result = CheckBudgetTool
                .execute_with_context(json!({}), &context)
                .await;
            assert_eq!(
                *sessions.lock().unwrap(),
                vec![context.session_id.to_string()]
            );
            match (status, result) {
                (Some(status), ToolExecutionResult::Success(value)) => assert_eq!(
                    value,
                    json!({"status":status,"budgets":[{"currency":"usd","limit":5.0,"balance":2.5,"soft_limit":1.0,"percent_remaining":50.0,"status":status},{"currency":"tokens","limit":100.0,"balance":20.0,"percent_remaining":20.0,"status":"warning"}],"hint":"budget guidance"})
                ),
                (None, ToolExecutionResult::ToolError(message)) => assert_eq!(
                    message,
                    "Budget check is temporarily unavailable. You can proceed normally."
                ),
                other => panic!("unexpected result {other:?}"),
            }
        }
    }
}
