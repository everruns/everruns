use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::kernel_imports::{AgentCapabilityConfig, OrgRole};
use everruns_contracts::typed_id::HarnessId;

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

/// Selecting a harness assigns its inherited capabilities too.
// THREAT[TM-AGENT-005]: parent chains and dependencies cannot bypass assignment checks.
pub(crate) async fn check_harness_assignment(
    ctx: &Ctx,
    harness_id: HarnessId,
) -> Result<(), CommandError> {
    let harness =
        crate::domains::harnesses::queries::resolve_effective(&ctx.db, ctx.org_id(), harness_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::not_found("Harness"))?;
    // Declarative capabilities are not expanded by the builtin registry.
    check_high_risk_caps(ctx, &harness.capabilities).await?;
    let resolved = everruns_core::capabilities::resolve_capability_configs(
        &harness.capabilities,
        ctx.capability_service.registry(),
    )
    .map_err(|error| CommandError::bad_request(error.to_string()))?;
    check_high_risk_caps(ctx, &resolved).await
}

pub(super) async fn resolve_create_harness_id(
    ctx: &Ctx,
    harness_id: Option<HarnessId>,
    harness_name: Option<&str>,
) -> Result<HarnessId, CommandError> {
    resolve_harness_id(ctx, harness_id, harness_name, true)
        .await?
        .ok_or_else(|| CommandError::not_found("Harness"))
}

pub(super) async fn resolve_update_harness_id(
    ctx: &Ctx,
    harness_id: Option<HarnessId>,
    harness_name: Option<&str>,
) -> Result<Option<HarnessId>, CommandError> {
    resolve_harness_id(ctx, harness_id, harness_name, false).await
}

async fn resolve_harness_id(
    ctx: &Ctx,
    harness_id: Option<HarnessId>,
    harness_name: Option<&str>,
    default_when_omitted: bool,
) -> Result<Option<HarnessId>, CommandError> {
    if harness_id.is_some() && harness_name.is_some() {
        return Err(CommandError::bad_request(
            "harness_id and harness_name are mutually exclusive",
        ));
    }

    let row = if let Some(id) = harness_id {
        ctx.db
            .get_harness(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
    } else if let Some(name) = harness_name {
        ctx.db
            .get_harness_by_name(ctx.org_id(), name)
            .await
            .map_err(classify_anyhow)?
    } else if default_when_omitted {
        let id = crate::domains::sessions::queries::resolve_session_harness_id(
            &ctx.db,
            ctx.org_id(),
            None,
            None,
            ctx.fallback_harness_name
                .as_deref()
                .or(Some("conversation")),
        )
        .await
        .map_err(classify_anyhow)?;
        ctx.db
            .get_harness(ctx.org_id(), id)
            .await
            .map_err(classify_anyhow)?
    } else {
        None
    };

    let Some(row) = row else {
        return if default_when_omitted || harness_id.is_some() || harness_name.is_some() {
            Err(CommandError::not_found("Harness"))
        } else {
            Ok(None)
        };
    };

    if row.status != "active" {
        return Err(CommandError::bad_request(
            "Archived or deleted harnesses cannot be assigned to agents",
        ));
    }
    check_harness_assignment(ctx, row.id).await?;
    Ok(Some(row.id))
}
