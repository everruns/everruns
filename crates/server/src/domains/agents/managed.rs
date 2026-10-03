use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::kernel_imports::{AgentCapabilityConfig, OrgRole};

pub(super) fn validate_managed_name(name: &str) -> Result<(), CommandError> {
    if name == crate::platform_chat_agent::NAME {
        return Err(CommandError::bad_request(
            "platform-chat is reserved for the managed Agent",
        ));
    }
    Ok(())
}

pub(super) async fn check_high_risk_caps(
    ctx: &Ctx,
    caps: &[AgentCapabilityConfig],
) -> Result<(), CommandError> {
    if caps.is_empty() || ctx.caller.role.has_permission(OrgRole::Admin) {
        return Ok(());
    }
    let refs: Vec<&str> = caps.iter().map(|c| c.capability_id()).collect();
    let high = ctx
        .capability_service
        .high_risk_ids_for_org(ctx.org_id(), &refs)
        .await
        .map_err(classify_anyhow)?;
    if !high.is_empty() {
        return Err(CommandError::forbidden(format!(
            "Admin role required to assign high-risk capabilities: {}",
            high.join(", ")
        )));
    }
    Ok(())
}
