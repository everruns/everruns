//! Effective Agent features, shared by publication and hosted execution.
use super::record::work_chat::{ChatFeatures, WorkChatConfig};
use crate::domains::common::CommandError;
use crate::storage::StorageBackend;
use everruns_contracts::typed_id::{AgentId, HarnessId};

pub async fn features(
    db: &StorageBackend,
    org: i64,
    agent: AgentId,
    harness: HarnessId,
) -> anyhow::Result<ChatFeatures> {
    let agent = crate::domains::agents::queries::resolve(db, org, &agent.to_string())
        .await?
        .ok_or_else(|| anyhow::anyhow!("Agent unavailable"))?;
    let harness = crate::domains::harnesses::queries::resolve_effective(db, org, harness)
        .await?
        .ok_or_else(|| anyhow::anyhow!("Harness unavailable"))?;
    let registry = crate::platform::oss_capability_registry();
    let mut capabilities = harness.capabilities;
    capabilities.extend(agent.capabilities);
    let configs =
        everruns_core::capabilities::resolve_capability_configs(&capabilities, &registry)?;
    let flags = crate::services::org_feature_flags::resolve_org_feature_flags(
        db,
        org,
        &crate::records::FeatureFlagPolicy::current(),
    )
    .await?;
    let is_platform_chat = agent.is_built_in && agent.name == crate::platform_chat_agent::NAME;
    let mut result = ChatFeatures::default();
    for config in configs {
        let id = config.capability_id();
        if !flags.is_agent_capability_enabled(id, is_platform_chat) {
            continue;
        }
        if id == "coordination"
            && let Some(capability) = registry.get(id)
        {
            capability
                .validate_config(config.config_value())
                .map_err(|e| anyhow::anyhow!(e))?;
            let tools = capability.tools_with_config(config.config_value());
            result.threads = [
                "start_thread",
                "message_thread",
                "list_threads",
                "get_thread",
                "resolve_thread",
            ]
            .iter()
            .all(|name| tools.iter().any(|tool| tool.name() == *name));
        }
        if id == "session_file_system" {
            result.files = true;
        }
    }
    Ok(result)
}

pub async fn validate(
    db: &StorageBackend,
    org: i64,
    agent: AgentId,
    harness: HarnessId,
    config: &WorkChatConfig,
) -> Result<ChatFeatures, CommandError> {
    let features = features(db, org, agent, harness).await?;
    features
        .validate(config.interface_mode)
        .map_err(CommandError::bad_request)?;
    Ok(features)
}

/// Run inside the command transaction: invalidating an inherited capability or
/// publishing an incompatible channel rolls back the complete mutation.
pub async fn validate_live(ctx: &crate::domains::common::Ctx) -> Result<(), CommandError> {
    for agent in ctx.db.live_work_chat_agents(ctx.org_id()).await? {
        for row in ctx.db.list_agent_channels(ctx.org_id(), agent).await? {
            if row.channel_type != "work_chat" || row.channel_status != "live" || !row.enabled {
                continue;
            }
            let (context, channel) = super::ingress::row_to_ingress(ctx.encryption.as_ref(), row)?;
            let config: WorkChatConfig = serde_json::from_value(channel.channel_config)
                .map_err(|e| CommandError::bad_request(e.to_string()))?;
            let agent = context
                .agent_id
                .ok_or_else(|| CommandError::bad_request("Work Chat requires an Agent"))?;
            validate(&ctx.db,ctx.org_id(),agent,context.harness_id,&config).await.map_err(|e|
                CommandError::bad_request(format!("Channel {} is incompatible: {}. Change its interface or unpublish it first", channel.public_id,e)))?;
        }
    }
    Ok(())
}
