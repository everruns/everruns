//! The Slack relevance check answered by the org's own decision model.
//!
//! An org with `system_decisions: organization` has its default decision model
//! decide whether an unmentioned message deserves a reply, instead of the
//! deployment's decisions service. Every failure here means silence, never a
//! fall back to the deployment (THREAT[TM-LLM-037]).
//!
//! Decision: a message in a thread that already has a session is decided on
//! that session's budget and billed to it, the Jev tool's path. A message that
//! would start a session has neither yet, so the check runs session-less:
//! personal providers fail closed, there is no budget to check, and usage goes
//! to the structured log (THREAT[TM-LLM-049]).

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::connection_services::{DecisionModelExecutor, SystemDecisionModel};
use everruns_core::tool_context::ToolContext;
use everruns_core::{DecisionOutcome, DecisionRequest};
use everruns_integrations::typesafe::{BoundDecisionExecutor, evaluate_unmetered};

use super::SlackState;
use crate::domains::budgets::BudgetService;
use crate::storage::SystemDecisions;
use crate::storage::models::SessionRow;

/// Host services the org-selected path needs, wired by the app builder.
#[derive(Clone)]
pub struct SlackOrgDecisions {
    pub provider_resolver: Arc<crate::services::ProviderResolverService>,
    pub budget_service: Arc<BudgetService>,
    pub egress: Arc<dyn everruns_core::EgressService>,
}

/// Whether this org answers deployment-owned decision checks itself.
pub(super) async fn org_selected(state: &SlackState, org_id: i64) -> anyhow::Result<bool> {
    Ok(state
        .db
        .get_organization_settings(org_id)
        .await?
        .is_some_and(|settings| {
            SystemDecisions::from_db(&settings.system_decisions) == SystemDecisions::Organization
        }))
}

/// Ask the org's decision default.
pub(super) async fn evaluate(
    state: &SlackState,
    org_id: i64,
    session: Option<&SessionRow>,
    request: DecisionRequest,
) -> anyhow::Result<DecisionOutcome> {
    let host = state
        .org_decisions
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("organization decisions are not wired"))?;
    let source = host
        .provider_resolver
        .resolve_system_decision_model(org_id, session.map(|s| s.id.uuid()))
        .await?;
    let SystemDecisionModel::Organization(binding) = source else {
        anyhow::bail!("organization no longer selects its own decision model");
    };
    let Some(session) = session else {
        return evaluate_unmetered(&binding, request, host.egress.as_ref())
            .await
            .map_err(anyhow::Error::msg);
    };
    let mut context = ToolContext::new(session.id);
    context.egress_service = Some(host.egress.clone());
    context.event_emitter = Some(state.event_service.clone());
    context.event_context = Some(everruns_core::events::EventContext::empty());
    context.budget_checker = Some(Arc::new(SessionBudget {
        budgets: host.budget_service.clone(),
        org_id,
        agent_id: session.agent_id.map(|id| id.to_string()),
    }));
    Ok(BoundDecisionExecutor
        .evaluate(binding, request, &context)
        .await?)
}

/// The session's budget state, read the way the runtime's check does.
struct SessionBudget {
    budgets: Arc<BudgetService>,
    org_id: i64,
    agent_id: Option<String>,
}

#[async_trait]
impl everruns_core::tool_execution::BudgetChecker for SessionBudget {
    async fn check_budgets(
        &self,
        session_id: &str,
    ) -> everruns_contracts::error::Result<everruns_core::budget::BudgetToolResponse> {
        let check = self
            .budgets
            .check_budgets_for_session(self.org_id, session_id, self.agent_id.as_deref())
            .await;
        let status = match check.action.as_str() {
            "stop" => "exhausted",
            "pause" => "paused",
            _ => "active",
        };
        Ok(everruns_core::budget::BudgetToolResponse {
            status: status.into(),
            budgets: vec![],
            hint: None,
        })
    }
}
