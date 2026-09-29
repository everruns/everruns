//! Webhook-trigger exposure: who may publish one, and what a response may say
//! about it.
//!
//! A webhook trigger is the one trigger type that opens an unauthenticated
//! ingress and carries a bearer token, so both halves of that exposure live
//! here rather than among the ordinary trigger commands.

use everruns_platform::{AgentTrigger, AgentTriggerType};
use serde_json::Value;

use crate::domains::agents::AGENT_DANGEROUS;
use crate::domains::common::{CommandError, Ctx};

/// Publishing a webhook trigger is what makes the ingress reachable, so it
/// takes the dangerous permission rather than ordinary trigger management.
///
/// Called on create and on the transition into `enabled`; every other update is
/// unaffected, which is why the conditions live here rather than at each site.
pub(super) fn require_publication_permission(
    ctx: &Ctx,
    trigger_type: AgentTriggerType,
    publishing: bool,
) -> Result<(), CommandError> {
    if trigger_type != AgentTriggerType::Webhook || !publishing {
        return Ok(());
    }
    AGENT_DANGEROUS
        .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
        .map_err(|e| CommandError::forbidden(e.message))
}

/// Replace a webhook token with the fact that one is set.
///
/// The token authenticates callers of the ingress, so a read of the trigger
/// must confirm its presence without reproducing it.
pub(super) fn redact_for_response(mut trigger: AgentTrigger) -> AgentTrigger {
    if trigger.trigger_type == AgentTriggerType::Webhook
        && let Some(config) = trigger.config.as_object_mut()
        && config.remove("token").is_some()
    {
        config.insert("token_configured".to_string(), Value::Bool(true));
    }
    trigger
}
