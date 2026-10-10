// Agent keys: managed credentials for an agent's `api` channel.
//
// Decisions:
// - A key belongs to the org and is granted to one api channel. Managing keys is
//   managing the agent's channels (AGENT_MANAGE); the channel's own publish
//   gate (AGENT_DANGEROUS) still decides whether any key reaches the agent.
// - The secret is returned once, by create and rotate. Rotation keeps the key
//   id, so sessions tagged with it survive, and the replaced secret keeps
//   working for an overlap window so callers can roll over without downtime.
// - Revocation is a soft delete: the row stays for audit and is never valid
//   again.
// THREAT[TM-AGENTKEY-001]: agent keys reach only execution routes; every
// management extractor rejects them (they are neither JWTs nor PATs).

use chrono::{Duration, Utc};
use serde::Deserialize;
use utoipa::ToSchema;

use super::find_agent;
use crate::auth::audit;
use crate::domains::agent_channels::record::ChannelType;
use crate::domains::agent_channels::record::api::{
    AgentKey, AgentKeyPermission, AgentKeyWithSecret, agent_key_display_prefix,
    agent_key_public_id, generate_agent_key, hash_agent_key, parse_agent_key_id,
};
use crate::domains::agents::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::audit_logs::record::{AuditEvent, ManagementAction};
use crate::domains::common::*;
use crate::storage::{AgentKeyRow, CreateAgentKeyRow, IngressChannelRow};

/// Default and longest overlap for a rotated secret, in hours.
const DEFAULT_OVERLAP_HOURS: u32 = 24;
const MAX_OVERLAP_HOURS: u32 = 168;
/// Live keys one channel may hold; a guard against runaway scripts.
const MAX_KEYS_PER_CHANNEL: usize = 50;

pub(crate) fn row_to_key(row: AgentKeyRow, channel_public_id: &str) -> AgentKey {
    AgentKey {
        id: agent_key_public_id(row.id),
        channel_id: channel_public_id.to_string(),
        name: row.name,
        prefix: row.token_prefix,
        permissions: row
            .permissions
            .iter()
            .filter_map(|p| AgentKeyPermission::parse(p))
            .collect(),
        expires_at: row.expires_at,
        previous_valid_until: row.previous_valid_until.filter(|until| *until > Utc::now()),
        last_used_at: row.last_used_at,
        revoked_at: row.revoked_at,
        created_at: row.created_at,
    }
}

/// The agent's `api` channel, or 404 (another type is not found either).
async fn api_channel(
    ctx: &Ctx,
    agent_id: &str,
    channel_id: &str,
) -> Result<IngressChannelRow, CommandError> {
    let agent = find_agent(ctx, agent_id).await?;
    let row = ctx
        .db
        .get_agent_channel(ctx.org_id(), agent.id.uuid(), channel_id)
        .await?
        .ok_or_else(|| CommandError::not_found("Channel"))?;
    if ChannelType::from_str_opt(&row.channel_type) != Some(ChannelType::Api) {
        return Err(CommandError::not_found("Channel"));
    }
    Ok(row)
}

fn key_id(id: &str) -> Result<uuid::Uuid, CommandError> {
    parse_agent_key_id(id).ok_or_else(|| CommandError::not_found("Agent key"))
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct ListAgentKeys {
    pub agent_id: String,
    pub channel_id: String,
}

#[command(
    name = "list_agent_keys",
    category = "agent_channels",
    description = "List the agent keys of an agent's API channel. Secrets are never returned.",
    method = "GET",
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "channels", "keys"], "list").with_examples(&[CliExample::new("See which applications hold a key before rotating or revoking one", "everruns agents channels keys list --agent-id agent_01h9 --channel-id appchan_01h9",)]),
)]
impl Command for ListAgentKeys {
    type Output = Vec<AgentKey>;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let channel = api_channel(ctx, &self.agent_id, &self.channel_id).await?;
        Ok(ctx
            .db
            .list_agent_keys(ctx.org_id(), channel.channel_id)
            .await?
            .into_iter()
            .map(|row| row_to_key(row, &channel.channel_public_id))
            .collect())
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct CreateAgentKey {
    pub agent_id: String,
    pub channel_id: String,
    /// Display name, e.g. the application that holds the key.
    #[schema(example = "Support backend")]
    pub name: String,
    /// When the key stops working. Omit for no expiry.
    #[serde(default)]
    pub expires_at: Option<chrono::DateTime<Utc>>,
}

#[command(
    name = "create_agent_key",
    category = "agent_channels",
    description = "Create an agent key for an agent's API channel. The secret is returned once.",
    method = "POST",
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys",
    policy = AGENT_MANAGE,
    cli = CliRoute::new(&["agents", "channels", "keys"], "create").with_examples(&[CliExample::new("Give a backend service its own key to call the agent", "everruns agents channels keys create --agent-id agent_01h9 --channel-id appchan_01h9 --name \"Support backend\" --reason \"New support integration\"",)]),
)]
impl Command for CreateAgentKey {
    type Output = AgentKeyWithSecret;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 128 {
            return Err(CommandError::bad_request(
                "name must be 1 to 128 characters",
            ));
        }
        if self.expires_at.is_some_and(|at| at <= Utc::now()) {
            return Err(CommandError::bad_request(
                "expires_at must be in the future",
            ));
        }
        let channel = api_channel(ctx, &self.agent_id, &self.channel_id).await?;
        let live = ctx
            .db
            .list_agent_keys(ctx.org_id(), channel.channel_id)
            .await?
            .iter()
            .filter(|row| row.revoked_at.is_none())
            .count();
        if live >= MAX_KEYS_PER_CHANNEL {
            return Err(CommandError::bad_request(format!(
                "A channel holds at most {MAX_KEYS_PER_CHANNEL} live keys; revoke one first"
            )));
        }
        let secret = generate_agent_key();
        let row = ctx
            .db
            .create_agent_key(CreateAgentKeyRow {
                org_id: ctx.org_id(),
                channel_id: channel.channel_id,
                name: name.to_string(),
                token_hash: hash_agent_key(&secret),
                token_prefix: agent_key_display_prefix(&secret),
                permissions: vec![AgentKeyPermission::Sessions.as_str().to_string()],
                expires_at: self.expires_at,
                created_by_user_id: ctx.caller.user_id,
            })
            .await?;
        let key = row_to_key(row, &channel.channel_public_id);
        emit(ctx, ManagementAction::ApiKeyCreated, &key, "created");
        Ok(AgentKeyWithSecret { key, secret })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct RotateAgentKey {
    pub agent_id: String,
    pub channel_id: String,
    pub key_id: String,
    /// Hours the replaced secret keeps working (0 to 168, default 24).
    #[serde(default)]
    pub overlap_hours: Option<u32>,
}

#[command(
    name = "rotate_agent_key",
    category = "agent_channels",
    description = "Replace an agent key's secret, keeping its id. The new secret is returned once; the old one works for the overlap.",
    method = "POST",
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/rotate",
    policy = AGENT_MANAGE,
    cli = CliRoute::new(&["agents", "channels", "keys"], "rotate").with_examples(&[CliExample::new("Replace a key secret on schedule, keeping the old one valid for a day", "everruns agents channels keys rotate --agent-id agent_01h9 --channel-id appchan_01h9 --key-id agentkey_01h9 --overlap-hours 24 --reason \"Quarterly rotation\"",)]),
)]
impl Command for RotateAgentKey {
    type Output = AgentKeyWithSecret;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let overlap = self.overlap_hours.unwrap_or(DEFAULT_OVERLAP_HOURS);
        if overlap > MAX_OVERLAP_HOURS {
            return Err(CommandError::bad_request(format!(
                "overlap_hours must be at most {MAX_OVERLAP_HOURS}"
            )));
        }
        let channel = api_channel(ctx, &self.agent_id, &self.channel_id).await?;
        let id = key_id(&self.key_id)?;
        let secret = generate_agent_key();
        let row = ctx
            .db
            .rotate_agent_key(
                ctx.org_id(),
                channel.channel_id,
                id,
                &hash_agent_key(&secret),
                &agent_key_display_prefix(&secret),
                Utc::now() + Duration::hours(i64::from(overlap)),
            )
            .await?
            .ok_or_else(|| CommandError::not_found("Agent key"))?;
        let key = row_to_key(row, &channel.channel_public_id);
        emit(ctx, ManagementAction::ApiKeyCreated, &key, "rotated");
        Ok(AgentKeyWithSecret { key, secret })
    }
}

#[derive(Debug, Deserialize, ToSchema, serde::Serialize)]
pub struct RevokeAgentKey {
    pub agent_id: String,
    pub channel_id: String,
    pub key_id: String,
}

#[command(
    name = "revoke_agent_key",
    category = "agent_channels",
    description = "Revoke an agent key. It never works again, including a secret still in its rotation overlap.",
    method = "POST",
    path = "/v1/agents/{agent_id}/channels/{channel_id}/keys/{key_id}/revoke",
    policy = AGENT_MANAGE,
    cli = CliRoute::new(&["agents", "channels", "keys"], "revoke").with_examples(&[CliExample::new("Cut off a key that leaked, immediately", "everruns agents channels keys revoke --agent-id agent_01h9 --channel-id appchan_01h9 --key-id agentkey_01h9 --reason \"Key exposed in a log\"",)]),
)]
impl Command for RevokeAgentKey {
    type Output = AgentKey;

    async fn execute(self, ctx: &Ctx) -> Result<Self::Output, CommandError> {
        let channel = api_channel(ctx, &self.agent_id, &self.channel_id).await?;
        let id = key_id(&self.key_id)?;
        let row = ctx
            .db
            .revoke_agent_key(ctx.org_id(), channel.channel_id, id)
            .await?
            .ok_or_else(|| CommandError::not_found("Agent key"))?;
        let key = row_to_key(row, &channel.channel_public_id);
        emit(ctx, ManagementAction::ApiKeyRevoked, &key, "revoked");
        Ok(key)
    }
}

fn emit(ctx: &Ctx, action: ManagementAction, key: &AgentKey, change: &str) {
    let event = AuditEvent::management(action, ctx.org_id(), ctx.caller.user_id)
        .target("agent_key", key.id.clone())
        .detail("kind", "agent_key")
        .detail("change", change)
        .detail("channel_id", key.channel_id.clone())
        .detail("prefix", key.prefix.clone());
    audit::emit_event(ctx.db.clone(), event.build());
}
