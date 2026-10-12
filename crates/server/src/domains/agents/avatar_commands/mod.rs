// Agent avatar commands, and the write path every avatar source shares.
//
// Design decisions:
// - One write path. The multipart upload route, the preset route and the
//   `set_agent_avatar` command all resolve the agent with `manageable_agent`,
//   render with `render_upload` / `preset_variants`, and persist with
//   `store_avatar`, so the agent rules (not deleted, not built-in), the stored
//   variants and the Slack icon push cannot drift between surfaces. History is
//   the caller's job: `Command::run` records it for the command, the REST
//   routes record it through `change_history::rest`.
// - The CLI and MCP carry JSON arguments only, so `set_agent_avatar` takes an
//   uploaded picture inline as base64. The decoded cap (`MAX_INLINE_AVATAR_BYTES`,
//   1 MiB) keeps the base64 request (~1.4 MB) under the 2 MB default body
//   limit that `/v1/commands`, the MCP endpoint and the command's own route
//   share. Larger images take the multipart route, which accepts 10 MB.
// - The command is not transactional (`change_history::registry::transactional`):
//   decoding and resizing an image is CPU work that must not hold a database
//   connection and row locks open.

use std::sync::Arc;

use base64::Engine as _;
use everruns_contracts::typed_id::AgentId;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::avatar::{RenderedAvatarVariant, render_avatar};
use super::avatar_presets::{AvatarPreset, PRESETS, find_preset};
use super::record::AgentAvatar;
use super::{AGENT_MANAGE, AGENT_VIEW};
use crate::domains::agent_channels::record::slack_provisioning::SlackAppProvisioner;
use crate::domains::common::*;
use crate::storage::encryption::EncryptionService;
use crate::storage::{
    AgentAvatarVariantInput, AgentRow, SetAgentAvatar as StoreAvatar, StorageBackend,
};

/// Largest decoded image `set_agent_avatar` accepts inline.
pub const MAX_INLINE_AVATAR_BYTES: usize = 1024 * 1024;

/// The agent whose avatar may change: exists, not deleted, not built-in.
/// Callers check `AGENT_MANAGE` first (commands through `Command::run`).
pub async fn manageable_agent(
    db: &StorageBackend,
    org_id: i64,
    agent_id: &str,
) -> Result<AgentRow, CommandError> {
    // An id, not a name: history records the change against this exact ref.
    let agent_id: AgentId = agent_id
        .parse()
        .map_err(|_| CommandError::bad_request("agent_id must be an agent ID (agent_...)"))?;
    let row = db
        .get_agent_by_public_id(org_id, &agent_id.to_string())
        .await?
        .filter(|row| row.status != "deleted")
        .ok_or_else(|| CommandError::not_found("Agent"))?;
    if row.is_built_in {
        // Same wording as `queries::ensure_not_built_in`: a platform upgrade
        // ships built-in agents, so their branding is not the org's to edit.
        return Err(CommandError::bad_request(
            "Cannot modify built-in agent. Copy it first to create an editable version.",
        ));
    }
    Ok(row)
}

/// Decode and render an uploaded image. CPU-bound, so off the async workers.
pub async fn render_upload(
    data: impl AsRef<[u8]> + Send + 'static,
    content_type: String,
) -> Result<Vec<RenderedAvatarVariant>, CommandError> {
    tokio::task::spawn_blocking(move || render_avatar(data.as_ref(), &content_type))
        .await
        .map_err(|error| CommandError::internal(anyhow::anyhow!("avatar render task: {error}")))?
        .map_err(|error| CommandError::bad_request(error.to_string()))
}

/// The stored source label and rendered variants of a curated preset.
pub async fn preset_variants(
    preset_id: &str,
) -> Result<(String, Vec<RenderedAvatarVariant>), CommandError> {
    let preset = find_preset(preset_id).ok_or_else(|| {
        CommandError::bad_request(
            "Unknown avatar preset. Choose one from the catalog (list_avatar_presets).",
        )
    })?;
    let variants = tokio::task::spawn_blocking(move || {
        preset.variants().map(<[RenderedAvatarVariant]>::to_vec)
    })
    .await
    .map_err(|error| CommandError::internal(anyhow::anyhow!("avatar preset task: {error}")))?
    .map_err(|error| CommandError::internal(anyhow::anyhow!("avatar preset: {error}")))?;
    Ok((format!("preset:{}", preset.metadata.id), variants))
}

/// Replace the agent's avatar with `variants` and push it to the agent's
/// one-click Slack apps once the write commits.
pub async fn store_avatar(
    db: &Arc<StorageBackend>,
    slack_provisioner: Option<Arc<dyn SlackAppProvisioner>>,
    encryption: Option<Arc<EncryptionService>>,
    org_id: i64,
    agent: &AgentRow,
    source: String,
    variants: Vec<RenderedAvatarVariant>,
) -> Result<AgentAvatar, CommandError> {
    let agent_uuid = agent.id.uuid();
    let avatar_id = db
        .set_agent_avatar(StoreAvatar {
            org_id,
            agent_id: agent_uuid,
            source,
            variants: variants
                .into_iter()
                .map(|v| AgentAvatarVariantInput {
                    variant: v.variant,
                    content_type: v.content_type.to_string(),
                    data: v.data,
                })
                .collect(),
        })
        .await?
        .ok_or_else(|| CommandError::not_found("Agent"))?;

    if let Some(provisioner) = slack_provisioner {
        let db = db.clone();
        let api_base_url = super::avatar_slack::deployment_api_base();
        // Reads the new avatar row, so it starts once the change commits.
        crate::storage::transaction::spawn_after_commit(async move {
            super::avatar_slack::push_avatar_to_agent_slack_apps(
                db,
                encryption,
                provisioner,
                org_id,
                agent_uuid,
                avatar_id,
                &api_base_url,
            )
            .await;
        });
    }
    Ok(AgentAvatar::from_uuid(avatar_id))
}

/// Strip whitespace and an optional `data:<type>;base64,` prefix, enforce the
/// inline cap, decode, and settle the content type: the caller's, else the
/// data URI's, else the one the bytes announce.
fn decode_inline_image(
    encoded: &str,
    content_type: Option<&str>,
) -> Result<(String, Vec<u8>), CommandError> {
    let trimmed = encoded.trim();
    let (uri_type, payload) = match trimmed.strip_prefix("data:") {
        Some(rest) => {
            let (header, payload) = rest.split_once(',').ok_or_else(|| {
                CommandError::bad_request("image_base64 data URI has no ',' before the data")
            })?;
            let mime = header.strip_suffix(";base64").ok_or_else(|| {
                CommandError::bad_request("image_base64 data URI must be base64-encoded")
            })?;
            (Some(mime.to_string()), payload)
        }
        None => (None, trimmed),
    };
    let compact: String = payload
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    let too_large = || {
        CommandError::bad_request(format!(
            "image_base64 must decode to at most {} MB. Shrink the image, or upload it as \
             multipart field `file` to PUT /v1/agents/{{agent_id}}/avatar (up to 10 MB).",
            MAX_INLINE_AVATAR_BYTES / (1024 * 1024)
        ))
    };
    // Refuse before decoding: 4 base64 characters carry 3 bytes.
    if compact.len() > MAX_INLINE_AVATAR_BYTES.div_ceil(3) * 4 {
        return Err(too_large());
    }
    let data = base64::engine::general_purpose::STANDARD
        .decode(compact.as_bytes())
        .map_err(|_| CommandError::bad_request("image_base64 is not valid base64"))?;
    if data.len() > MAX_INLINE_AVATAR_BYTES {
        return Err(too_large());
    }
    if data.is_empty() {
        return Err(CommandError::bad_request("image_base64 is empty"));
    }
    let content_type = match content_type.map(str::to_string).or(uri_type) {
        Some(content_type) => content_type,
        None => image::guess_format(&data)
            .ok()
            .map(|format| format.to_mime_type().to_string())
            .ok_or_else(|| {
                CommandError::bad_request(
                    "Could not detect the image type; pass content_type (image/png, image/jpeg, \
                     image/gif or image/webp)",
                )
            })?,
    };
    Ok((content_type, data))
}

// ============================================================================
// SetAgentAvatar
// ============================================================================

/// Set an agent's avatar from a curated preset or inline image bytes.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct SetAgentAvatar {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    #[serde(flatten)]
    pub req: SetAgentAvatarRequest,
}

/// The avatar source: exactly one of `preset_id` or `image_base64`.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct SetAgentAvatarRequest {
    /// Curated preset ID from `list_avatar_presets`. Pass this or `image_base64`, not both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "familiars-patch")]
    pub preset_id: Option<String>,
    /// Image bytes as base64 (a `data:image/...;base64,` URI also works): PNG, JPEG, GIF or
    /// WebP, at least 64x64, at most 1 MB decoded. Center-cropped to a square.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_base64: Option<String>,
    /// MIME type of `image_base64`. Detected from the bytes when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "image/png")]
    pub content_type: Option<String>,
}

#[command(
    name = "set_agent_avatar",
    category = "agents",
    description = "Set an agent's avatar from a curated preset (preset_id, see list_avatar_presets) or an image (image_base64: PNG, JPEG, GIF or WebP, at least 64x64, at most 1 MB). Replaces the previous avatar and updates the icon of the agent's one-click Slack apps.",
    method = "POST",
    path = "/v1/agents/{agent_id}/avatar/set",
    policy = AGENT_MANAGE,
    positional = "agent_id",
    http = plain,
    request_body(SetAgentAvatarRequest),
    responses((status = 400, description = "Invalid preset or image"), (status = 404, description = "Agent not found")),
    cli = CliRoute::new(&["agents", "avatar"], "set").with_args(&[CliArg::new("agent_id").at(1).long("agent"), CliArg::new("preset_id").long("preset"), CliArg::new("image_base64").long("image-base64")]).with_examples(&[CliExample::new("Give an agent a curated avatar", "everruns agents avatar set agent_01h9 --preset familiars-patch --reason 'Brand the triage agent'"), CliExample::new("Use your own image, base64-encoded into a file first (base64 logo.png > logo.b64)", "everruns agents avatar set agent_01h9 --image-base64 @logo.b64 --reason 'Use the team logo'")]),
)]
impl Command for SetAgentAvatar {
    type Output = AgentAvatar;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<AgentAvatar>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<AgentAvatar, CommandError> {
        enum Source {
            Preset(String),
            Upload(String, Vec<u8>),
        }
        let SetAgentAvatarRequest {
            preset_id,
            image_base64,
            content_type,
        } = self.req;
        let source = match (preset_id, image_base64) {
            (Some(preset_id), None) => {
                if content_type.is_some() {
                    return Err(CommandError::bad_request(
                        "content_type applies to image_base64 only",
                    ));
                }
                Source::Preset(preset_id)
            }
            (None, Some(encoded)) => {
                let (content_type, data) = decode_inline_image(&encoded, content_type.as_deref())?;
                Source::Upload(content_type, data)
            }
            _ => {
                return Err(CommandError::bad_request(
                    "Pass exactly one of preset_id or image_base64",
                ));
            }
        };

        let agent = manageable_agent(&ctx.db, ctx.org_id(), &self.agent_id).await?;
        let (label, variants) = match source {
            Source::Preset(preset_id) => preset_variants(&preset_id).await?,
            Source::Upload(content_type, data) => (
                "upload".to_string(),
                render_upload(data, content_type).await?,
            ),
        };
        store_avatar(
            &ctx.db,
            ctx.slack_provisioner.clone(),
            ctx.encryption.clone(),
            ctx.org_id(),
            &agent,
            label,
            variants,
        )
        .await
    }
}

// ============================================================================
// ListAvatarPresets
// ============================================================================

/// List the curated agent avatar presets.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct ListAvatarPresets {}

#[command(
    name = "list_avatar_presets",
    category = "agents",
    description = "List the curated agent avatars: stable id, name, family, suggested role, description and search keywords. Pass an id to set_agent_avatar as preset_id. Preview one at /v1/avatar-presets/{id}/square-256.png.",
    method = "GET",
    path = "/v1/avatar-presets",
    policy = AGENT_VIEW,
    cli = CliRoute::new(&["agents", "avatar"], "presets").with_examples(&[CliExample::new("Find a curated avatar to give an agent", "everruns agents avatar presets")]),
)]
impl Command for ListAvatarPresets {
    type Output = Vec<AvatarPreset>;

    fn output_schema() -> serde_json::Value {
        array_output_schema(output_schema_for::<AvatarPreset>())
    }

    fn output_shape() -> &'static str {
        "array"
    }

    async fn execute(self, _ctx: &Ctx) -> Result<Vec<AvatarPreset>, CommandError> {
        Ok(PRESETS.iter().map(|p| p.metadata.clone()).collect())
    }
}

#[cfg(test)]
mod tests;
