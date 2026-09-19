//! Building the Slack app manifest served to the install flow.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use everruns_platform::{ConversationStarter, SlackChannelConfig};

use super::super::common::ErrorResponse;

use super::*;

/// GET /v1/apps/{app_id}/slack/manifest — Generate a Slack App manifest.
///
/// Returns a pre-filled YAML manifest and a URL that opens Slack's "Create app
/// from manifest" flow. The manifest carries bot scopes *and*
/// `event_subscriptions`, so no hand-editing is needed after creation.
///
/// Slack verifies `request_url` when the manifest is saved, which is why this
/// endpoint serves only published apps: the webhook has to be answering before
/// the Slack app is created from the manifest.
pub(crate) async fn handle_slack_manifest_legacy(
    State(state): State<SlackState>,
    Path(app_id): Path<String>,
) -> Result<Json<ManifestResponse>, (StatusCode, Json<ErrorResponse>)> {
    handle_slack_manifest(state, SlackTarget::LegacyApp(app_id)).await
}

pub(crate) async fn handle_slack_manifest_endpoint(
    State(state): State<SlackState>,
    Path(channel_id): Path<String>,
) -> Result<Json<ManifestResponse>, (StatusCode, Json<ErrorResponse>)> {
    handle_slack_manifest(state, SlackTarget::Endpoint(channel_id)).await
}

pub(crate) async fn handle_slack_manifest(
    state: SlackState,
    target: SlackTarget,
) -> Result<Json<ManifestResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (app, slack_channel) = resolve_slack_channel(&state, target).await?;

    // A config we cannot parse still produces the channel-bot manifest rather than
    // a 500: the agent surface is additive, so defaulting it off is the safe read.
    let agent_surface_enabled =
        serde_json::from_value::<SlackChannelConfig>(slack_channel.channel_config.clone())
            .map(|config| config.agent_surface_enabled)
            .unwrap_or(false);

    let display_name = truncate_display_name(&app.name);

    // Suggested prompts come from the exposure's conversation starters (EVE-978).
    // Only the agent surface renders them, so nothing is loaded when it is off.
    let starters = if agent_surface_enabled {
        resolve_manifest_starters(&state, &app).await
    } else {
        Vec::new()
    };

    let channel_public_id = slack_channel.public_id.to_string();
    let request_url = slack_webhook_url(&state.api_base_url, &channel_public_id);
    let interactivity_url = slack_interactivity_url(&state.api_base_url, &channel_public_id);
    let redirect_url = slack_oauth_redirect_url(&state.api_base_url, &channel_public_id);
    let manifest_yaml = build_manifest_yaml(
        &app.name,
        &display_name,
        app.description.as_deref(),
        &request_url,
        &interactivity_url,
        &redirect_url,
        agent_surface_enabled,
        &starters,
    );

    // URL-encode the manifest for the Slack "create from manifest" URL
    let encoded = urlencoding_encode(&manifest_yaml);
    let create_url = format!(
        "https://api.slack.com/apps?new_app=1&manifest_yaml={}",
        encoded
    );

    Ok(Json(ManifestResponse {
        manifest_yaml,
        create_url,
    }))
}

/// This server's Slack webhook endpoint for one channel.
///
/// Fully determined by the channel's public ID before the Slack app exists,
/// which makes `event_subscriptions` generatable.
pub(crate) fn slack_webhook_url(api_base_url: &str, channel_public_id: &str) -> String {
    format!(
        "{}/v1/e/{}/slack/events",
        api_base_url.trim_end_matches('/'),
        channel_public_id
    )
}

/// Where Slack posts a click on an approval card (EVE-1025).
///
/// Determined the same way and at the same time as the webhook URL, so
/// `settings.interactivity` is generatable alongside `event_subscriptions` and
/// an operator never has to add it by hand.
pub(crate) fn slack_interactivity_url(api_base_url: &str, channel_public_id: &str) -> String {
    format!(
        "{}/v1/e/{}/slack/interactivity",
        api_base_url.trim_end_matches('/'),
        channel_public_id
    )
}

/// OAuth redirect target for this endpoint's Slack app.
///
/// Slack refuses `/oauth/v2/authorize` outright when the app declares no
/// redirect URL ("redirect_uri did not match any configured URIs"), so an app
/// without this can never be installed by OAuth — only by the copy-paste flow,
/// which is why the omission went unnoticed. The route itself does not exist
/// yet; declaring it here is what makes the app installable once it does.
pub(crate) fn slack_oauth_redirect_url(api_base_url: &str, channel_public_id: &str) -> String {
    format!(
        "{}/v1/e/{}/slack/oauth/callback",
        api_base_url.trim_end_matches('/'),
        channel_public_id
    )
}

/// Conversation starters for this App's Slack agent surface.
///
/// Agent starters win over the harness ones, resolved by
/// `everruns_platform::exposure::resolve_starters` so Slack and Platform Chat
/// cannot drift apart. The harness is resolved through `resolve_effective` to
/// pick up inherited starters, and a grandfathered agent-less App falls back to
/// the harness alone.
///
/// A lookup failure yields no prompts rather than a 500: the manifest is how an
/// operator gets their Slack app created at all, and suggested prompts are
/// cosmetic next to that.
pub(crate) async fn resolve_manifest_starters(
    state: &SlackState,
    app: &crate::api::app_ingress::IngressContext,
) -> Vec<ConversationStarter> {
    let agent_starters = match app.agent_id.as_ref() {
        Some(agent_id) => {
            match crate::domains::agents::queries::get_by_public_id(
                &state.db,
                app.org_id,
                &agent_id.to_string(),
            )
            .await
            {
                Ok(Some(agent)) => agent.starters,
                Ok(None) => Vec::new(),
                Err(error) => {
                    tracing::warn!(%agent_id, %error, "Failed to load agent for Slack manifest starters");
                    Vec::new()
                }
            }
        }
        None => Vec::new(),
    };

    let harness_starters = match crate::domains::harnesses::queries::resolve_effective(
        &state.db,
        app.org_id,
        app.harness_id,
    )
    .await
    {
        Ok(Some(harness)) => harness.starters,
        Ok(None) => Vec::new(),
        Err(error) => {
            tracing::warn!(harness_id = %app.harness_id, %error, "Failed to resolve harness for Slack manifest starters");
            Vec::new()
        }
    };

    everruns_platform::exposure::resolve_starters(&agent_starters, &harness_starters).to_vec()
}

/// Build the YAML manifest for a Slack app.
///
/// Slack requires `long_description` to be 174–4000 chars. We build it from the
/// app's description (if any) plus a standard suffix, padding if needed.
/// Slack caps `agent_view.agent_description` at 300 characters.
pub(crate) const SLACK_AGENT_DESC_MAX: usize = 300;

/// Slack renders at most four suggested prompts in an agent thread, so the
/// manifest never carries more than that even though an agent may author up to
/// `MAX_STARTERS` for Platform Chat.
pub(crate) const SLACK_SUGGESTED_PROMPT_MAX: usize = 4;

/// Slack does not document a cap on a prompt `title`, but the title renders as a
/// tappable chip and a starter may be a full 280-byte sentence. Keep the chip
/// legible and let the untruncated text ride in `message`, which is what Slack
/// actually inserts into the composer.
pub(crate) const SLACK_SUGGESTED_PROMPT_TITLE_MAX: usize = 60;

/// The `features.agent_view` block, or empty when the agent surface is off.
///
/// New apps must use `agent_view`; `assistant_view` is the legacy spelling Slack
/// is deprecating, and Everruns only ever generates manifests for new apps.
/// `agent_description` is the only required sub-field. `suggested_prompts` is
/// filled from the exposure's resolved conversation starters (EVE-978); `actions`
/// stays out because nothing authors it.
pub(crate) fn build_agent_view(
    app_name: &str,
    app_description: Option<&str>,
    starters: &[ConversationStarter],
) -> String {
    let desc = match app_description.filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("{app_name} — {d}"),
        None => format!("{app_name}, an AI agent powered by Everruns"),
    };
    let desc = truncate_chars(&desc, SLACK_AGENT_DESC_MAX);

    format!(
        "\x20 agent_view:\n\
         \x20   agent_description: \"{}\"\n\
         {}",
        yaml_escape(&desc),
        build_suggested_prompts(starters)
    )
}

/// The `suggested_prompts` list, or empty when nothing was authored.
///
/// Nothing authored must stay nothing: Slack shows an empty pane, which is a
/// better first impression than three generic prompts nobody wrote. Blank
/// starters are dropped for the same reason rather than emitting an empty chip.
pub(crate) fn build_suggested_prompts(starters: &[ConversationStarter]) -> String {
    let prompts: Vec<&str> = starters
        .iter()
        .map(|starter| starter.text.trim())
        .filter(|text| !text.is_empty())
        .take(SLACK_SUGGESTED_PROMPT_MAX)
        .collect();

    if prompts.is_empty() {
        return String::new();
    }

    let mut out = String::from("\x20   suggested_prompts:\n");
    for text in prompts {
        let title = truncate_chars(text, SLACK_SUGGESTED_PROMPT_TITLE_MAX);
        out.push_str(&format!(
            "\x20     - title: \"{}\"\n\
             \x20       message: \"{}\"\n",
            yaml_escape(&title),
            yaml_escape(text)
        ));
    }
    out
}

/// Truncate to at most `max` characters, on a char boundary.
pub(crate) fn truncate_chars(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((idx, _)) => s[..idx].to_string(),
        None => s.to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_manifest_yaml(
    app_name: &str,
    display_name: &str,
    app_description: Option<&str>,
    request_url: &str,
    interactivity_url: &str,
    redirect_url: &str,
    agent_surface_enabled: bool,
    starters: &[ConversationStarter],
) -> String {
    let escaped_name = yaml_escape(app_name);
    let name = &escaped_name;
    let display_name = yaml_escape(display_name);
    let long_desc = build_long_description(app_name, app_description);
    let long_desc = yaml_escape(&long_desc);
    let request_url = yaml_escape(request_url);
    let interactivity_url = yaml_escape(interactivity_url);
    let redirect_url = yaml_escape(redirect_url);

    // The agent surface is additive: it adds a scope, a feature block and four
    // events on top of the channel bot, which keeps working exactly as before.
    let agent_view = if agent_surface_enabled {
        build_agent_view(app_name, app_description, starters)
    } else {
        String::new()
    };
    let agent_scope = if agent_surface_enabled {
        "\x20     - assistant:write\n"
    } else {
        ""
    };
    let agent_events = if agent_surface_enabled {
        "\x20     - app_home_opened\n\
         \x20     - app_context_changed\n\
         \x20     - agent_session_stopped\n\
         \x20     - agent_session_title_changed\n"
    } else {
        ""
    };

    format!(
        "display_information:\n\
         \x20 name: \"{name}\"\n\
         \x20 description: \"{name} (Powered by Everruns)\"\n\
         \x20 long_description: \"{long_desc}\"\n\
         \x20 background_color: \"#1a1a2e\"\n\
         features:\n\
         \x20 bot_user:\n\
         \x20   display_name: \"{display_name}\"\n\
         \x20   always_online: true\n\
         {agent_view}\
         oauth_config:\n\
         \x20 redirect_urls:\n\
         \x20   - \"{redirect_url}\"\n\
         \x20 scopes:\n\
         \x20   bot:\n\
         \x20     - chat:write\n\
         \x20     - channels:history\n\
         \x20     - groups:history\n\
         \x20     - im:history\n\
         \x20     - mpim:history\n\
         \x20     - app_mentions:read\n\
         \x20     - users:read\n\
         \x20     - files:read\n\
         {agent_scope}\
         settings:\n\
         \x20 interactivity:\n\
         \x20   is_enabled: true\n\
         \x20   request_url: \"{interactivity_url}\"\n\
         \x20 event_subscriptions:\n\
         \x20   request_url: \"{request_url}\"\n\
         \x20   bot_events:\n\
         \x20     - app_mention\n\
         \x20     - message.channels\n\
         \x20     - message.groups\n\
         \x20     - message.im\n\
         \x20     - message.mpim\n\
         {agent_events}\
         \x20 org_deploy_enabled: false\n\
         \x20 socket_mode_enabled: false\n\
         \x20 token_rotation_enabled: false\n",
    )
}

/// Truncate display name to 80 chars (Slack limit for bot_user display_name).
/// Uses char boundary to avoid panicking on multi-byte UTF-8.
pub(crate) fn truncate_display_name(name: &str) -> String {
    if name.len() <= 80 {
        return name.to_string();
    }
    // Find the last char boundary at or before 80 bytes
    let mut end = 80;
    while !name.is_char_boundary(end) {
        end -= 1;
    }
    name[..end].to_string()
}

/// Simple YAML string escaping: escape backslashes and double quotes.
pub(crate) fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Slack requires long_description to be 174–4000 characters.
pub(crate) const SLACK_LONG_DESC_MIN: usize = 174;
pub(crate) const SLACK_LONG_DESC_MAX: usize = 4000;

/// Build a long_description that meets Slack's 174-char minimum.
///
/// Structure: "{app_name} — {description} | AI agent powered by Everruns — https://everruns.com"
/// If the app has no description or the result is still short, pads with a
/// generic blurb to reach the minimum.
pub(crate) fn build_long_description(app_name: &str, app_description: Option<&str>) -> String {
    let suffix = "AI agent powered by Everruns — https://everruns.com";

    let mut desc = match app_description.filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("{app_name} — {d} | {suffix}"),
        None => format!("{app_name} — {suffix}"),
    };

    // Pad to meet minimum if needed.
    if desc.len() < SLACK_LONG_DESC_MIN {
        let pad = " Everruns lets you build, deploy, and run AI agents with durable execution, tool use, and real-time streaming. Learn more at https://everruns.com.";
        desc.push_str(pad);
    }

    // Truncate to max (UTF-8 safe).
    if desc.len() > SLACK_LONG_DESC_MAX {
        let mut end = SLACK_LONG_DESC_MAX;
        while !desc.is_char_boundary(end) {
            end -= 1;
        }
        desc.truncate(end);
    }

    desc
}

/// Percent-encode a string for URL query parameters.
pub(crate) fn urlencoding_encode(s: &str) -> String {
    let mut result = String::new();
    for byte in s.as_bytes() {
        match *byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(*byte as char);
            }
            _ => {
                result.push_str(&format!("%{:02X}", byte));
            }
        }
    }
    result
}
