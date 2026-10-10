//! Cross-resource validation for Agent create and update commands.

use crate::domains::common::{CommandError, Ctx};
use crate::domains::sandbox_templates::record::SandboxPolicy;
use crate::kernel_imports::AgentCapabilityConfig;
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
    .await?;
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

/// Check an Agent sandbox policy against its Harness's execution rule.
///
/// Bashkit Worker is sealed, so any policy is rejected. Sandbox Worker needs a
/// full sandbox, so a policy that can only ever resolve to Bashkit is rejected;
/// no policy at all is allowed here and reported when a Session starts.
pub(super) async fn validate_sandbox_policy_for_harness(
    ctx: &Ctx,
    harness_id: HarnessId,
    sandbox_policy: Option<&SandboxPolicy>,
) -> Result<(), CommandError> {
    use crate::domains::harnesses::record::HarnessExecution;
    use crate::domains::sandbox_templates::record::SandboxTargetKind;
    let Some(sandbox_policy) = sandbox_policy else {
        return Ok(());
    };
    match crate::domains::harnesses::queries::execution_rule(&ctx.db, ctx.org_id(), harness_id)
        .await?
    {
        HarnessExecution::Unbound => Ok(()),
        HarnessExecution::FixedBashkit => Err(CommandError::unprocessable(
            "Bashkit Worker fixes the Sandbox Template to Bashkit Virtual Workspace; remove the Agent sandbox_policy or choose Worker or Sandbox Worker",
        )),
        HarnessExecution::FullSandbox => {
            if sandbox_policy
                .templates
                .values()
                .all(|template| template.target.kind == SandboxTargetKind::Vfs)
            {
                Err(CommandError::unprocessable(
                    "Sandbox Worker needs a full sandbox; choose a container or managed Sandbox Template, or use Bashkit Worker for Bashkit",
                ))
            } else {
                Ok(())
            }
        }
    }
}

pub(super) async fn validate_sandbox_template_sources(
    ctx: &Ctx,
    sandbox_policy: Option<&SandboxPolicy>,
) -> Result<(), CommandError> {
    let Some(sandbox_policy) = sandbox_policy else {
        return Ok(());
    };
    for spec in sandbox_policy.templates.values() {
        if spec.target.credential.source
            == everruns_contracts::session_sandbox::SessionSandboxCredentialSource::Organization
        {
            let connection_id = spec.target.credential.connection_id.ok_or_else(|| {
                CommandError::unprocessable(
                    "Organization Sandbox credentials require an account selection",
                )
            })?;
            let connection = ctx
                .db
                .get_organization_connection(ctx.org_id(), connection_id)
                .await?
                .ok_or_else(|| CommandError::unprocessable("Organization connection not found"))?;
            if connection.provider != spec.target.provider.as_deref().unwrap_or("") {
                return Err(CommandError::unprocessable(
                    "Organization connection does not match the Sandbox provider",
                ));
            }
        }
        let Some(revision_id) = spec.template_revision_id else {
            continue;
        };
        let revision = ctx
            .db
            .get_sandbox_template_revision(ctx.org_id(), revision_id)
            .await?
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
