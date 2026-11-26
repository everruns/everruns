use crate::domains::common::{CommandError, Ctx};
use uuid::Uuid;

/// Validate references before irreversibly removing an agent's external apps.
pub(super) async fn prepare_for_removal(ctx: &Ctx, agent_id: Uuid) -> Result<(), CommandError> {
    crate::domains::agent_channels::slack_cleanup::remove_agent_apps(ctx, agent_id).await
}
