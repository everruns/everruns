//! Playground is a shared session, with an immutable end-user test subject.
use crate::domains::common::{CommandError, Ctx, classify_anyhow};
use everruns_core::{Permission, Policy, Rule};
use everruns_provider::typed_id::{PrincipalId, VirtualUserId};

const IMPERSONATE: Policy = Policy {
    id: "playground.impersonate",
    rules: &[Rule::UserHasPermission(
        Permission::OrgPlaygroundImpersonate,
    )],
};

pub async fn validate_subject(
    ctx: &Ctx,
    requested: Option<VirtualUserId>,
) -> Result<VirtualUserId, CommandError> {
    if !ctx.feature_flags.playground {
        return Err(CommandError::feature_not_enabled("playground"));
    }
    let user = ctx.caller.user_id.ok_or_else(|| {
        CommandError::forbidden("Playground requires a signed-in organisation member")
    })?;
    let own = ctx
        .db
        .default_virtual_user(ctx.org_id(), user)
        .await
        .map_err(classify_anyhow)?;
    let id = requested.unwrap_or(own.id);
    // THREAT[TM-AUTHZ-021]: Only authorised operators may select another active in-org subject.
    if id != own.id {
        IMPERSONATE
            .evaluate_with(ctx.permission_resolver.as_ref(), &ctx.caller)
            .map_err(|e| CommandError::forbidden(e.to_string()))?;
    }
    let subject = ctx
        .db
        .get_virtual_user(ctx.org_id(), id)
        .await
        .map_err(classify_anyhow)?
        .ok_or_else(|| CommandError::not_found("Virtual user"))?;
    if subject.status != "active" || subject.usage != "end_user" {
        return Err(CommandError::bad_request(
            "Playground requires an active end-user virtual user",
        ));
    }
    Ok(id)
}

pub async fn subject_principal(ctx: &Ctx, id: VirtualUserId) -> Result<PrincipalId, CommandError> {
    validate_subject(ctx, Some(id)).await?;
    // A simulated subject has no management authority. Do not attach the operator's
    // user lineage to it; existing subject principals retain their verified lineage.
    let service = crate::services::PrincipalService::new(ctx.db.clone());
    let parent = service
        .ensure_system_principal(ctx.org_id(), "playground")
        .await
        .map_err(classify_anyhow)?;
    Ok(service
        .ensure_virtual_user_principal(ctx.org_id(), id, parent.id)
        .await
        .map_err(classify_anyhow)?
        .id)
}
