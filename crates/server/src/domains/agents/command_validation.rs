//! Cross-resource validation for Agent create and update commands.

use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::kernel_imports::AgentCapabilityConfig;
use crate::records::SandboxPolicy;
use everruns_contracts::typed_id::HarnessId;

pub(super) async fn normalize_capability_refs(
    ctx: &Ctx,
    caps: Vec<AgentCapabilityConfig>,
) -> Result<Vec<AgentCapabilityConfig>, CommandError> {
    let caps = crate::domains::capabilities::validation::normalize_capability_refs(
        &ctx.db,
        ctx.org_id(),
        caps,
    )
    .await
    .map_err(classify_anyhow)?;
    crate::domains::capabilities::validation::validate_feature_gated_capability_refs(
        &ctx.feature_flags,
        &caps,
    )?;
    crate::domains::capabilities::validation::validate_hydrated_capability_size_for_org(
        &ctx.db,
        ctx.org_id(),
        &caps,
    )
    .await?;
    Ok(caps)
}

pub(super) async fn reject_sandbox_override_for_fixed_harness(
    ctx: &Ctx,
    harness_id: HarnessId,
    has_sandbox_policy: bool,
) -> Result<(), CommandError> {
    if !has_sandbox_policy {
        return Ok(());
    }
    let fixed = crate::domains::harnesses::queries::inherits_from_name(
        &ctx.db,
        ctx.org_id(),
        harness_id,
        "bashkit-worker",
    )
    .await
    .map_err(classify_anyhow)?;
    if fixed {
        return Err(CommandError::unprocessable(
            "Bashkit Worker fixes the Sandbox Template to Bashkit Virtual Workspace; remove the Agent sandbox_policy or choose a provider-neutral Harness",
        ));
    }
    Ok(())
}

pub(super) async fn validate_sandbox_template_sources(
    ctx: &Ctx,
    sandbox_policy: Option<&SandboxPolicy>,
) -> Result<(), CommandError> {
    let Some(sandbox_policy) = sandbox_policy else {
        return Ok(());
    };
    for spec in sandbox_policy.templates.values() {
        let Some(revision_id) = spec.template_revision_id else {
            continue;
        };
        let revision = ctx
            .db
            .get_sandbox_template_revision(ctx.org_id(), revision_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::unprocessable("Sandbox Template revision not found"))?;
        let mut authored = spec.clone();
        authored.template_revision_id = None;
        let mut stored = revision.spec;
        stored.template_revision_id = None;
        if authored != stored {
            return Err(CommandError::unprocessable(
                "Sandbox specification does not match its immutable template revision",
            ));
        }
    }
    Ok(())
}
