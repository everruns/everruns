//! Slack Code: durable signed ingress, one session per task, and a recoverable
//! projection. Credentials and authorization remain in the control plane.
use super::events::SlackState;
use crate::channels::host::{ServerChannelPort, ServerChannelScope};
use crate::domains::agent_channels::ingress::IngressChannel;
use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
use anyhow::{Context, bail};
use everruns_contracts::typed_id::SessionId;
use everruns_core::channel::ExternalActor;
use everruns_core::channel_runtime::{ChannelSessionPort, ResolvedChannel};
use everruns_integrations::slack_channel::web_api::{SlackApiError, slack_api_call};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::time::Duration;
use uuid::Uuid;

pub(crate) mod interaction;
mod projection;
mod store;
#[cfg(test)]
mod tests;
mod worker;
pub(crate) use projection::Projection;
use store::{Inbox, Task};
pub(crate) use store::{enqueue, request};

/// Retained by the server supervisor. Every unit is locked in PostgreSQL, so
/// restart or another API instance resumes the same intent, not a new task.
pub(crate) fn spawn(state: SlackState) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            for _ in 0..16 {
                match worker::inbox_step(&state).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        tracing::warn!(%error, "Slack Code inbox deferred");
                        break;
                    }
                }
            }
            for _ in 0..16 {
                match projection::step(&state).await {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(error) => {
                        tracing::warn!(%error, "Slack Code projection deferred");
                        break;
                    }
                }
            }
        }
    })
}

#[derive(Debug, thiserror::Error)]
#[error("Slack Code endpoint unavailable")]
pub(super) struct EndpointUnavailable;

pub(super) async fn endpoint(
    state: &SlackState,
    id: Uuid,
) -> anyhow::Result<(ResolvedChannel, SlackChannelConfig)> {
    let public_id: String = sqlx::query_scalar("SELECT public_id FROM agent_channels WHERE id=$1")
        .bind(id)
        .fetch_optional(state.db.pool())
        .await?
        .ok_or(EndpointUnavailable)?;
    let (context, channel) = crate::domains::agent_channels::ingress::resolve_channel(
        &state.db,
        state.encryption.as_ref(),
        &public_id,
    )
    .await?
    .ok_or(EndpointUnavailable)?;
    crate::domains::agent_channels::ingress::channel_liveness(&context, &channel)
        .map_err(|_| EndpointUnavailable)?;
    let config: SlackChannelConfig =
        serde_json::from_value(channel.channel_config.clone()).map_err(|_| EndpointUnavailable)?;
    if !config.code_channels_enabled || config.bot_token.is_empty() {
        return Err(EndpointUnavailable.into());
    }
    let resolved = state
        .channels
        .as_ref()
        .context("Missing channel host")?
        .resolver
        .channel_for(ServerChannelScope::new(context, channel))
        .ok_or(EndpointUnavailable)?;
    Ok((resolved, config))
}

/// Bot assignment is stronger than channel membership. The workspace and bot
/// are read using this installation's token, never supplied by the model.
pub(super) async fn channel_info(
    base: &str,
    config: &SlackChannelConfig,
    team: &str,
    channel: &str,
) -> anyhow::Result<Value> {
    static PLAIN_CHANNELS: std::sync::OnceLock<moka::sync::Cache<String, Value>> =
        std::sync::OnceLock::new();
    let plain = PLAIN_CHANNELS.get_or_init(|| {
        moka::sync::Cache::builder()
            .max_capacity(1000)
            .time_to_live(Duration::from_secs(60))
            .build()
    });
    let key = format!("{base}:{}:{team}:{channel}", installation(config));
    if let Some(info) = plain.get(&key) {
        return Ok(info);
    }
    let auth = slack_api_call(base, &config.bot_token, "auth.test", json!({})).await?;
    if auth["team_id"] != team {
        return Err(SlackApiError::Permanent("Slack Code workspace mismatch".into()).into());
    }
    let info = slack_api_call(
        base,
        &config.bot_token,
        "conversations.info",
        json!({"channel":channel}),
    )
    .await?;
    let channel = info.get("channel").context("Missing conversation")?;
    if channel["properties"]["record_channel"]["record_type"] == "agent_channel" {
        verify_assignment(&auth, channel)?;
    }
    if !is_code(channel) {
        plain.insert(key, channel.clone());
    }
    Ok(channel.clone())
}

fn verify_assignment(auth: &Value, channel: &Value) -> anyhow::Result<()> {
    let bot = auth["user_id"].as_str().context("Missing bot identity")?;
    let assigned = channel["properties"]["agent_session"]["encoded_agent_bot_user_ids"]
        .as_array()
        .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(bot)));
    if !assigned || channel["is_member"] != true {
        return Err(SlackApiError::from_code("not_authorized", None).into());
    }
    Ok(())
}

pub(super) fn is_code(channel: &Value) -> bool {
    channel["properties"]["record_channel"]["record_type"] == "agent_channel"
}

pub(crate) fn installation(config: &SlackChannelConfig) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(config.bot_token.as_bytes()))
}

pub(super) fn scope_origin(config: &SlackChannelConfig, info: &Value, channel: &str) -> bool {
    config.channel_id.as_deref().is_none_or(|expected| {
        expected == channel
            || info["properties"]["agent_session"]["origin_link"]["channel_id"].as_str()
                == Some(expected)
    })
}

pub(super) fn stable_id(endpoint: Uuid, team: &str, channel: &str, ts: &str) -> Uuid {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for part in [endpoint.to_string().as_str(), team, channel, ts] {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&hash.finalize()[..16]);
    Uuid::from_bytes(bytes)
}
