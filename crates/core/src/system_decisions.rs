//! Deployment-owned decision checks, answered by the source the org picked.
//!
//! Guardrail `jev` checks ask the host's decisions service. An org can set
//! `system_decisions: organization` to have its own decision default answer
//! them instead. The choice is resolved per check through the session's
//! [`ProviderCredentialStore`], and an org-selected model runs through the
//! host's [`DecisionModelExecutor`]: the same egress, budget, and usage path as
//! the Jev tool.
//!
//! [`ProviderCredentialStore`]: crate::connection_services::ProviderCredentialStore
//! [`DecisionModelExecutor`]: crate::connection_services::DecisionModelExecutor
//!
//! Decision: an org that opted in but cannot be served (no default, a disabled
//! model, a failing lookup, a host without an executor) gets an error here,
//! which guardrails treat as fail-open. It never falls back to the deployment,
//! because that would spend deployment keys on a tenant that said not to
//! (THREAT[TM-LLM-037]).

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::runtime::connection_services::{
    DecisionModelExecutorExt, SystemDecisionModel,
};
use everruns_contracts::runtime::events::EventContext;
use everruns_contracts::runtime::execution_context::ExecutionContext;

use crate::tool_context::{ToolContext, ToolContextServices};
use crate::{DecisionOutcome, DecisionRequest, DecisionsService};

/// The decisions service a deployment-owned check in `context` should ask.
///
/// Without a credential store there is no org to ask, so the deployment's
/// service is returned unchanged.
pub fn for_context(context: &ToolContext) -> Option<Arc<dyn DecisionsService>> {
    if context.provider_credential_store.is_none() {
        return context.decisions.clone();
    }
    Some(Arc::new(SessionSystemDecisions {
        context: context.clone(),
    }))
}

/// The decisions service for a turn's post-generation output checks, which
/// run outside any tool call. Usage is attributed to the turn.
pub fn for_turn(
    services: &ToolContextServices,
    execution: &ExecutionContext,
) -> Option<Arc<dyn DecisionsService>> {
    let mut context = ToolContext::from_services(execution.session_id, services);
    context.event_context = Some(EventContext::from_execution_context(execution));
    for_context(&context)
}

/// Per-session decisions that defer the deployment/organization choice to the
/// moment a check is asked, so a setting change applies to the next check.
struct SessionSystemDecisions {
    context: ToolContext,
}

#[async_trait]
impl DecisionsService for SessionSystemDecisions {
    fn is_configured(&self) -> bool {
        // Either source may answer; which one is only known per request.
        self.context
            .decisions
            .as_ref()
            .is_some_and(|service| service.is_configured())
            || self
                .context
                .extensions
                .get::<DecisionModelExecutorExt>()
                .is_some()
    }

    async fn evaluate(&self, request: DecisionRequest) -> Result<DecisionOutcome> {
        let store = self
            .context
            .provider_credential_store
            .as_ref()
            .ok_or_else(|| AgentLoopError::tool("Decision source is unavailable"))?;
        match store
            .get_system_decision_model(self.context.session_id)
            .await?
        {
            SystemDecisionModel::Deployment => match &self.context.decisions {
                Some(service) if service.is_configured() => service.evaluate(request).await,
                _ => Err(AgentLoopError::tool(
                    "The deployment has no decisions configured",
                )),
            },
            SystemDecisionModel::Organization(binding) => {
                let executor = self
                    .context
                    .extensions
                    .get::<DecisionModelExecutorExt>()
                    .ok_or_else(|| {
                        AgentLoopError::tool("Organization decision models are unavailable")
                    })?;
                executor.0.evaluate(binding, request, &self.context).await
            }
        }
    }

    fn name(&self) -> &'static str {
        "SessionSystemDecisions"
    }
}

#[cfg(test)]
#[path = "system_decisions_tests.rs"]
mod tests;
