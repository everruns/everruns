use super::*;

/// Resolve a trigger and confirm it belongs to the named agent (org-scoped).
pub(in crate::domains::agent_triggers) async fn resolve_trigger_for_agent(
    ctx: &Ctx,
    agent_id: &str,
    trigger_id: &str,
) -> Result<(AgentRow, AgentTriggerRow), CommandError> {
    let agent_public = parse_agent_id(agent_id)?;
    let agent = q::require_active_agent(&ctx.db, ctx.org_id(), &agent_public).await?;
    let trigger_id = parse_trigger_id(trigger_id)?;
    let trigger = q::get_by_id(&ctx.db, ctx.org_id(), trigger_id)
        .await?
        .filter(|row| row.status != "deleted")
        .ok_or_else(|| CommandError::not_found("Agent trigger"))?;
    if trigger.agent_id != agent.id {
        return Err(CommandError::not_found("Agent trigger"));
    }
    Ok((agent, trigger))
}
