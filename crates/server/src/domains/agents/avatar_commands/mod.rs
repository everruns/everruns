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
//   limit that `/v1/platform/commands`, the MCP endpoint and the command's own route
//   share. Larger images take the multipart route, which accepts 10 MB.
// - The commands are not transactional (`change_history::registry::transactional`):
//   decoding and resizing an image is CPU work, and generating one is a
//   provider call of up to a minute; neither may hold a database connection
//   and row locks open.
// - `generate_agent_avatar` draws with the org's default image model (then
//   the platform default on the org's own provider accounts, the same chain
//   as the `image_generation` capability) and stores the result through the
//   same write path, so a CLI or MCP caller gets an avatar in one step
//   instead of running an agent session to draw it and passing the bytes
//   back through `set_agent_avatar`. The provider call leaves through the
//   host egress service (THREAT[TM-LLM-047]).

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

/// Authored avatar edits participate in source ownership and durable write-back.
/// Render/provider work finishes before taking the definition row lock.
pub async fn store_authored_avatar(
    ctx: &Ctx,
    agent: &AgentRow,
    source: String,
    variants: Vec<RenderedAvatarVariant>,
) -> Result<AgentAvatar, CommandError> {
    crate::storage::transaction::scope(
        &ctx.db,
        async {
            super::source::ensure_writable(ctx, agent.id).await?;
            store_avatar(
                &ctx.db,
                ctx.slack_provisioner.clone(),
                ctx.encryption.clone(),
                ctx.org_id(),
                agent,
                source,
                variants,
            )
            .await
        },
        CommandError::internal,
    )
    .await
}

pub async fn clear_authored_avatar(ctx: &Ctx, agent: &AgentRow) -> Result<(), CommandError> {
    crate::storage::transaction::scope(
        &ctx.db,
        async {
            super::source::ensure_writable(ctx, agent.id).await?;
            ctx.db
                .clear_agent_avatar(ctx.org_id(), agent.id.uuid())
                .await?;
            Ok(())
        },
        CommandError::internal,
    )
    .await
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
             multipart field `file` to PUT /v1/platform/agents/{{agent_id}}/avatar (up to 10 MB).",
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
    summary = "Set an agent's avatar from a preset or an image",
    method = "POST",
    path = "/v1/platform/agents/{agent_id}/avatar/set",
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
        store_authored_avatar(ctx, &agent, label, variants).await
    }
}

// ============================================================================
// GenerateAgentAvatar
// ============================================================================

/// Longest prompt `generate_agent_avatar` accepts, in characters.
pub const MAX_AVATAR_PROMPT_CHARS: usize = 2000;

/// Style rules appended to every avatar prompt, so the picture still reads as
/// a 32 px circle in a sidebar or a Slack message.
const AVATAR_STYLE: &str = "Make it a square avatar icon: one clear subject centered on a simple background, bold shapes that stay recognizable when shown as a small circle, no text, letters, borders or watermarks.";

/// The prompt sent to the image model: the caller's, or one drawn from the
/// agent's name and description, then the avatar style rules.
fn avatar_prompt(agent: &AgentRow, prompt: Option<&str>) -> Result<String, CommandError> {
    let subject = match prompt.map(str::trim) {
        Some("") => return Err(CommandError::bad_request("prompt is empty")),
        Some(prompt) if prompt.chars().count() > MAX_AVATAR_PROMPT_CHARS => {
            return Err(CommandError::bad_request(format!(
                "prompt must be at most {MAX_AVATAR_PROMPT_CHARS} characters"
            )));
        }
        Some(prompt) => prompt.to_string(),
        None => {
            let name = agent
                .display_name
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&agent.name);
            let about = agent
                .short_description
                .as_deref()
                .or(agent.description.as_deref())
                .map(str::trim)
                .filter(|about| !about.is_empty())
                .map(|about| {
                    about
                        .chars()
                        .take(MAX_AVATAR_PROMPT_CHARS)
                        .collect::<String>()
                });
            match about {
                Some(about) => {
                    format!("An avatar for an AI agent named \"{name}\". What it does: {about}")
                }
                None => format!("An avatar for an AI agent named \"{name}\"."),
            }
        }
    };
    Ok(format!("{subject}\n\n{AVATAR_STYLE}"))
}

/// Generate an agent's avatar with the org's image model and set it.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct GenerateAgentAvatar {
    /// Agent's prefixed public identifier.
    pub agent_id: String,
    #[serde(flatten)]
    pub req: GenerateAgentAvatarRequest,
}

/// What to draw.
#[derive(Debug, Default, Deserialize, Serialize, ToSchema)]
pub struct GenerateAgentAvatarRequest {
    /// What the avatar shows, at most 2000 characters. Omit to draw one from the agent's name
    /// and description. Avatar style rules (square, centered, no text) are added either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(
        example = "A friendly fox mechanic holding a wrench, flat vector style, teal and orange"
    )]
    pub prompt: Option<String>,
    /// Image quality when the model offers levels (`low`, `medium`, `high`, ...). Defaults to
    /// the model's usual quality.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(example = "high")]
    pub quality: Option<String>,
}

/// The new avatar and the model that drew it.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GeneratedAgentAvatar {
    #[serde(flatten)]
    pub avatar: AgentAvatar,
    /// Model id the image came from, as sent to the provider.
    #[schema(example = "gpt-image-2.6-flare")]
    pub model: String,
    /// Provider type that served it.
    #[schema(example = "openai")]
    pub provider_type: String,
    /// The prompt the model rewrote, when it reports one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
}

#[command(
    name = "generate_agent_avatar",
    category = "agents",
    description = "Generate an agent's avatar with the organization's default image model and set it, in one step. Describe it in prompt, or omit prompt to draw one from the agent's name and description. Replaces the previous avatar and updates the icon of the agent's one-click Slack apps. Calls the image provider (billed like any image generation) and can take up to a minute.",
    summary = "Generate an agent's avatar with the org's image model",
    method = "POST",
    path = "/v1/platform/agents/{agent_id}/avatar/generate",
    policy = AGENT_MANAGE,
    positional = "agent_id",
    http = plain,
    request_body(GenerateAgentAvatarRequest),
    responses((status = 400, description = "Invalid prompt or quality, or no image model available"), (status = 404, description = "Agent not found"), (status = 503, description = "The image provider failed")),
    cli = CliRoute::new(&["agents", "avatar"], "generate").with_args(&[CliArg::new("agent_id").at(1).long("agent")]).with_examples(&[CliExample::new("Draw an avatar from a description", "everruns agents avatar generate agent_01h9 --prompt 'A calm owl librarian, flat vector, navy and gold' --reason 'Give the research agent a face'"), CliExample::new("Let the agent's name and description pick the picture", "everruns agents avatar generate agent_01h9 --reason 'Brand the triage agent'")]),
)]
impl Command for GenerateAgentAvatar {
    type Output = GeneratedAgentAvatar;

    fn output_schema() -> serde_json::Value {
        output_schema_for::<GeneratedAgentAvatar>()
    }

    async fn execute(self, ctx: &Ctx) -> Result<GeneratedAgentAvatar, CommandError> {
        use everruns_integrations::image_generation::{
            StandaloneImageError, StandaloneImageRequest, generate_standalone_image,
        };

        // Every check that can fail runs before the paid provider call.
        let agent = manageable_agent(&ctx.db, ctx.org_id(), &self.agent_id).await?;
        if ctx
            .db
            .get_agent_source(ctx.org_id(), agent.id.uuid())
            .await?
            .is_some_and(|source| source.mode != "two_way")
        {
            return Err(CommandError::conflict(
                "This Agent's source is managed by GitHub. Edit the repository or disconnect source sync before editing",
            ));
        }
        let prompt = avatar_prompt(&agent, self.req.prompt.as_deref())?;
        let resolver = ctx
            .provider_service
            .as_ref()
            .and_then(|service| service.resolver())
            .ok_or_else(|| {
                CommandError::unavailable("Image generation is not available on this surface")
            })?;
        let org_default = resolver
            .resolve_default_image_model(ctx.org_id())
            .await
            .map_err(|error| {
                CommandError::bad_request(format!(
                    "The organization's default image model cannot be used: {error}. Choose another with `everruns models images-default set`."
                ))
            })?;
        let credentials = crate::worker_link::direct_worker_adapters::decision_models::org_provider_credential_store(
            resolver,
            ctx.org_id(),
        );
        let egress = ctx
            .egress_service
            .clone()
            .unwrap_or_else(|| ctx.capability_service.egress_service());

        let image = generate_standalone_image(
            org_default,
            &credentials,
            egress,
            StandaloneImageRequest {
                prompt,
                quality: self.req.quality,
                // Not every image model takes a background setting.
                background: None,
                square: true,
            },
        )
        .await
        .map_err(|error| match error {
            StandaloneImageError::Setup(message) => CommandError::bad_request(message),
            StandaloneImageError::Provider(message) => CommandError::unavailable(message),
        })?;

        let variants = render_upload(image.data, image.media_type.to_string()).await?;
        let avatar = store_authored_avatar(ctx, &agent, "generated".to_string(), variants).await?;
        Ok(GeneratedAgentAvatar {
            avatar,
            model: image.model,
            provider_type: image.provider_type,
            revised_prompt: image.revised_prompt,
        })
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
