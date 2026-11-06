// Connecting Slack turns on the agent's Reactions capability.
//
// Decisions:
// - The 👀/✅ acknowledgement for Slack messages is a default in the Reactions
//   capability's instructions (see `everruns_capabilities::reactions`), not a
//   platform auto-reaction, so the agent's own instructions stay in charge.
//   That default only reaches agents that have the capability, so the Slack
//   channel's first connection adds it.
// - The change is an ordinary `update_agent` run inside the channel command:
//   it lands in the agent's history with the caller's reason, and the author
//   can remove the capability afterwards. Later Slack channels do not add it
//   again, so a removal sticks.
// - Package-owned channels (`package_name` in their config) are skipped: the
//   package definition is authoritative for capabilities, and adding one would
//   make the agent drift from its source.
// - A harness that already provides Reactions counts: nothing is added.

use crate::domains::agents::{UpdateAgentCmd, types::UpdateAgentRequest};
use crate::domains::common::*;
use crate::kernel_imports::AgentCapabilityConfig;
use crate::storage::AgentRow;
use everruns_capabilities::capabilities::reactions::REACTIONS_CAPABILITY_ID;
use serde_json::Value;

/// Add the Reactions capability when `agent` gets its first Slack channel.
/// `channel_config` is the new channel's validated config.
pub(super) async fn ensure_on_first_slack_channel(
    ctx: &Ctx,
    agent: &AgentRow,
    channel_config: &Value,
) -> Result<(), CommandError> {
    if channel_config.get("package_name").is_some() {
        return Ok(());
    }
    let channels = ctx
        .db
        .list_agent_channels(ctx.org_id(), agent.id.uuid())
        .await?;
    let slack_channels = channels
        .iter()
        .filter(|channel| channel.channel_type == "slack")
        .count();
    // The channel this command just created is already in the list.
    if slack_channels != 1 {
        return Ok(());
    }
    let mut capabilities =
        crate::domains::agents::queries::get_capabilities(&ctx.db, ctx.org_id(), agent.id.uuid())
            .await?;
    let harness = crate::domains::harnesses::queries::get_capabilities(
        &ctx.db,
        ctx.org_id(),
        agent.harness_id.uuid(),
    )
    .await?;
    if capabilities
        .iter()
        .chain(harness.iter())
        .any(|capability| capability.capability_id() == REACTIONS_CAPABILITY_ID)
    {
        return Ok(());
    }
    capabilities.push(AgentCapabilityConfig::new(REACTIONS_CAPABILITY_ID));
    UpdateAgentCmd {
        id: agent.public_id.clone(),
        req: UpdateAgentRequest {
            capabilities: Some(capabilities),
            ..Default::default()
        },
    }
    .run(ctx)
    .await?;
    Ok(())
}
