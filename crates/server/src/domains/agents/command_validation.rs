//! Cross-resource validation for Agent create and update commands.

use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use crate::kernel_imports::AgentCapabilityConfig;
use crate::records::EnvironmentSet;
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

pub(super) async fn reject_environment_override_for_fixed_harness(
    ctx: &Ctx,
    harness_id: HarnessId,
    has_environments: bool,
) -> Result<(), CommandError> {
    if !has_environments {
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
            "Bashkit Worker fixes the Environment to Bashkit Virtual Workspace; remove Agent environments or choose a provider-neutral Harness",
        ));
    }
    Ok(())
}

pub(super) async fn validate_environment_sources(
    ctx: &Ctx,
    environments: Option<&EnvironmentSet>,
) -> Result<(), CommandError> {
    let Some(environments) = environments else {
        return Ok(());
    };
    for profile in environments.profiles.values() {
        let Some(revision_id) = profile.source_revision_id else {
            continue;
        };
        let revision = ctx
            .db
            .get_environment_revision(ctx.org_id(), revision_id)
            .await
            .map_err(classify_anyhow)?
            .ok_or_else(|| CommandError::unprocessable("Environment revision not found"))?;
        let mut authored = profile.clone();
        authored.source_revision_id = None;
        let mut stored = revision.profile;
        stored.source_revision_id = None;
        if authored != stored {
            return Err(CommandError::unprocessable(
                "Environment profile does not match its immutable source revision",
            ));
        }
    }
    Ok(())
}
