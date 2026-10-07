use crate::domains::common::{CommandError, Ctx};
use uuid::Uuid;

/// Validate references before irreversibly removing an agent's external apps.
pub(super) async fn prepare_for_removal(ctx: &Ctx, agent_id: Uuid) -> Result<(), CommandError> {
    crate::domains::apps::queries::ensure_no_app_references_to_agent(
        &ctx.db,
        ctx.org_id(),
        agent_id,
    )
    .await?;
    crate::domains::agent_channels::slack_cleanup::remove_agent_apps(ctx, agent_id).await
}
