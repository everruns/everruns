//! Schedule triggers that run in a conversation of one of the agent's
//! channels (design: schedules in Slack sessions, D2/D3).
//!
//! Decisions:
//! - A run goes through the channel host (`send_in_conversation`), not the
//!   trigger's own session routing: the session is the one a message from the
//!   conversation lands in (same binding store, same key), created and bound
//!   on the first run, and the turn's replies are delivered like an inbound
//!   message's, with the host's recovery.
//! - Who may target what (D3) is checked when the trigger is written: only the
//!   agent's own Slack channel, by a caller with `agent:manage` (the command's
//!   policy), in a conversation the channel's binding can key. The run checks
//!   again: a channel that is disabled, retired or moved fails the run, which
//!   records a failed delivery. It never falls back to a session of its own.
//! - The input carries the channel's provenance (written by the server's
//!   channel port) and the trigger id, so the trace still says which trigger
//!   ran it.

use std::sync::Arc;

use everruns_contracts::runtime::channel::ChannelError;
use everruns_contracts::runtime::message_retriever::InputMessage;
use everruns_contracts::typed_id::{AgentChannelId, SessionId, TriggerId};
use everruns_core::channel_runtime::ChannelStore;
use serde_json::Value;

use super::record::{AgentTriggerType, ScheduleTriggerTarget};
use crate::channels::host::{PgChannelStore, channel_command_error};
use crate::channels::slack::scheduled::{SlackConversation, scheduled_delivery, target_problem};
use crate::domains::agent_channels::ingress::{
    IngressChannel, IngressContext, channel_liveness, resolve_channel,
};
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::common::{CommandError, Ctx};
use crate::storage::{AgentRow, EncryptionService, StorageBackend};

/// The target a create or update asks for: both fields, neither, or an empty
/// `channel_id` that removes it (`Some(None)`).
pub(super) fn requested(
    channel_id: Option<String>,
    binding_key: Option<String>,
) -> Result<Option<Option<(String, String)>>, CommandError> {
    match (channel_id, binding_key) {
        (None, None) => Ok(None),
        (Some(channel), _) if channel.trim().is_empty() => Ok(Some(None)),
        (Some(channel), Some(key)) => Ok(Some(Some((
            channel.trim().to_string(),
            key.trim().to_string(),
        )))),
        _ => Err(CommandError::bad_request(
            "A trigger target needs both target_channel_id and target_binding_key",
        )),
    }
}

/// A target `agent` may run a schedule trigger in, or why not (D3).
pub(super) async fn validate(
    ctx: &Ctx,
    trigger_type: AgentTriggerType,
    agent: &AgentRow,
    channel_id: &str,
    binding_key: &str,
) -> Result<ScheduleTriggerTarget, CommandError> {
    if trigger_type != AgentTriggerType::Schedule {
        return Err(CommandError::bad_request(
            "Only schedule triggers can run in a channel conversation",
        ));
    }
    let channel_id: AgentChannelId = channel_id
        .parse()
        .map_err(|_| CommandError::bad_request("Invalid target_channel_id"))?;
    let (_, channel) = own_slack_channel(
        &ctx.db,
        ctx.encryption.as_ref(),
        ctx.org_id(),
        agent,
        channel_id,
    )
    .await?;
    let conversation = conversation(binding_key)?;
    let config = channel
        .slack_config()
        .ok_or_else(|| CommandError::bad_request("Invalid Slack channel configuration"))?;
    if let Some(problem) = target_problem(&config, &conversation) {
        return Err(CommandError::bad_request(problem));
    }
    Ok(ScheduleTriggerTarget {
        channel_id,
        binding_key: conversation.binding_key(),
    })
}

fn conversation(binding_key: &str) -> Result<SlackConversation, CommandError> {
    SlackConversation::parse(binding_key).ok_or_else(|| {
        CommandError::bad_request(
            "target_binding_key must be slack:user:{user_id} or slack:channel:{channel_id}",
        )
    })
}

/// The agent's own live, enabled Slack channel `channel_id`. Another tenant's
/// or another agent's channel reads as not found.
async fn own_slack_channel(
    db: &StorageBackend,
    encryption: Option<&Arc<EncryptionService>>,
    org_id: i64,
    agent: &AgentRow,
    channel_id: AgentChannelId,
) -> Result<(IngressContext, IngressChannel), CommandError> {
    let not_found = || CommandError::not_found("Agent channel");
    let (context, channel) = resolve_channel(db, encryption, &channel_id.to_string())
        .await?
        .ok_or_else(not_found)?;
    if context.org_id != org_id || context.agent_internal_id != agent.id.uuid() {
        return Err(not_found());
    }
    if channel.channel_type != ChannelType::Slack {
        return Err(CommandError::bad_request(
            "A schedule can run only in a Slack channel's conversation",
        ));
    }
    if !channel.enabled {
        return Err(CommandError::bad_request("The target channel is disabled"));
    }
    if let Err(reason) = channel_liveness(&context, &channel) {
        return Err(CommandError::bad_request(format!(
            "The target channel cannot post: {}",
            reason.as_str()
        )));
    }
    Ok((context, channel))
}

/// Run one schedule in its target conversation. Returns the session and
/// whether this run created it.
pub(super) async fn run(
    db: &Arc<StorageBackend>,
    route: &super::events::ChannelRun<'_>,
    agent: &AgentRow,
    trigger_id: TriggerId,
    rendered_message: String,
    request_id: Option<String>,
) -> Result<(SessionId, bool), CommandError> {
    let super::events::ChannelRun {
        target,
        channels,
        encryption,
    } = *route;
    let channels = channels.ok_or_else(|| {
        CommandError::internal(anyhow::anyhow!(
            "channel host unavailable for a channel-targeted trigger"
        ))
    })?;
    let (context, channel) =
        own_slack_channel(db, encryption, agent.org_id, agent, target.channel_id).await?;
    let conversation = conversation(&target.binding_key)?;
    let config = channel
        .slack_config()
        .ok_or_else(|| CommandError::bad_request("Invalid Slack channel configuration"))?;
    // The channel's binding may have changed since the trigger was written.
    if let Some(problem) = target_problem(&config, &conversation) {
        return Err(CommandError::bad_request(problem));
    }
    let resolved = channels
        .resolver
        .channel_for(crate::channels::host::ServerChannelScope {
            request_id,
            ..crate::channels::host::ServerChannelScope::new(context.clone(), channel.clone())
        })
        .ok_or_else(|| {
            CommandError::internal(anyhow::anyhow!("Slack channels are not running here"))
        })?;
    let key = conversation.binding_key();
    let bound = PgChannelStore::new(db.clone())
        .session_for(&channel.internal_id.to_string(), &key)
        .await
        .map_err(channel_command_error)?
        .and_then(|session| session.parse().ok());
    let delivery = scheduled_delivery(db, &context, &channel, &config, &conversation, bound).await;

    let mut message = InputMessage::user(rendered_message)
        .with_metadata("source", Value::String("agent_trigger".into()))
        .with_metadata("agent_trigger_id", Value::String(trigger_id.to_string()));
    for (key, value) in delivery.metadata {
        message = message.with_metadata(key, value);
    }
    let (conversation, _) = channels
        .host
        .send_in_conversation(&resolved, &key, message, delivery.target)
        .await
        .map_err(|error| match error {
            ChannelError::NotFound(_) => {
                CommandError::bad_request("The target channel cannot post")
            }
            error => channel_command_error(error),
        })?;
    let session_id = conversation.session_id.parse().map_err(|_| {
        CommandError::internal(anyhow::anyhow!("channel host returned a bad session id"))
    })?;
    Ok((session_id, conversation.created))
}

#[cfg(test)]
mod tests;
