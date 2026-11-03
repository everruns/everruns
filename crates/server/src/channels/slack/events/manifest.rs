//! Building the Slack app manifest served to the install flow.

use crate::domains::agent_channels::record::slack_channel::SlackChannelConfig;
pub(crate) use crate::domains::agent_channels::record::slack_channel::slack_bot_scopes;
use crate::domains::harnesses::record::ConversationStarter;
use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};

use crate::api::common::ErrorResponse;

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

pub(crate) async fn handle_slack_manifest_channel(
    State(state): State<SlackState>,
    Path(channel_id): Path<String>,
) -> Result<Json<ManifestResponse>, (StatusCode, Json<ErrorResponse>)> {
    handle_slack_manifest(state, SlackTarget::Channel(channel_id)).await
}

pub(crate) async fn handle_slack_manifest(
    state: SlackState,
    target: SlackTarget,
) -> Result<Json<ManifestResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (app, slack_channel) = resolve_slack_channel(&state, target).await?;
    let manifest_yaml = manifest_yaml_for_channel(&state, &app, &slack_channel).await?;

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

/// The manifest YAML for one resolved endpoint.
///
/// Split out of the handler so the one-click install path (`install`)
/// creates the app from exactly the manifest the copy-paste flow serves —
/// the PoC's finding that `apps.manifest.create` accepts it whole only holds
/// if the two cannot drift.
pub(crate) async fn manifest_yaml_for_channel(
    state: &SlackState,
    app: &crate::api::channel_ingress::IngressContext,
    slack_channel: &crate::api::channel_ingress::IngressChannel,
) -> Result<String, (StatusCode, Json<ErrorResponse>)> {
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
        resolve_manifest_starters(state, app).await
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

    Ok(manifest_yaml)
}

/// This server's Slack webhook endpoint for one channel.
///
/// Fully determined by the channel's public ID before the Slack app exists,
/// which makes `event_subscriptions` generatable.
pub(crate) fn slack_webhook_url(api_base_url: &str, channel_public_id: &str) -> String {
    format!(
        "{}/v1/channels/{}/slack/events",
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
        "{}/v1/channels/{}/slack/interactivity",
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
        "{}/v1/channels/{}/slack/oauth/callback",
        api_base_url.trim_end_matches('/'),
        channel_public_id
    )
}

/// Conversation starters for this App's Slack agent surface.
///
/// Suggested prompts are Agent presentation. Agent-less Apps have no starters.
///
/// A lookup failure yields no prompts rather than a 500: the manifest is how an
/// operator gets their Slack app created at all, and suggested prompts are
/// cosmetic next to that.
pub(crate) async fn resolve_manifest_starters(
    state: &SlackState,
    app: &crate::api::channel_ingress::IngressContext,
) -> Vec<ConversationStarter> {
    match app.agent_id.as_ref() {
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
    }
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
    let desc = build_agent_description(app_name, app_description);

    format!(
        "\x20 agent_view:\n\
         \x20   agent_description: \"{}\"\n\
         {}",
        yaml_escape(&desc),
        build_suggested_prompts(starters)
    )
}

pub(crate) fn build_agent_description(app_name: &str, app_description: Option<&str>) -> String {
    let desc = match app_description.filter(|d| !d.trim().is_empty()) {
        Some(d) => format!("{app_name} — {d}"),
        None => format!("{app_name}, an AI agent powered by Everruns"),
    };
    truncate_chars(&desc, SLACK_AGENT_DESC_MAX)
}

pub(crate) fn build_short_description(app_name: &str, app_description: Option<&str>) -> String {
    let description = app_description
        .filter(|d| !d.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{app_name} (Powered by Everruns)"));
    shorten_at_word(&description, SLACK_SHORT_DESC_MAX_BYTES)
}

/// Our cap for `display_information.description`, in bytes.
///
/// Slack documents 140 characters, but its real check is stricter than that:
/// a 129-character Ukrainian description (236 bytes) and a 140-byte ASCII one
/// both came back `invalid_manifest`, while 132 ASCII bytes passed. Slack does
/// not say which rule it applies, so we stay well clear of the boundary rather
/// than guess it exactly. The full text still reaches Slack in
/// `long_description`.
pub(crate) const SLACK_SHORT_DESC_MAX_BYTES: usize = 120;

/// `s` if it fits in `max` bytes, else its longest whole-word prefix that fits
/// with a trailing ellipsis. Falls back to a byte cut for a single long word.
pub(crate) fn shorten_at_word(s: &str, max: usize) -> String {
    const ELLIPSIS: &str = "…";
    let s = s.trim();
    if s.len() <= max {
        return s.to_string();
    }
    let room = truncate_bytes(s, max - ELLIPSIS.len());
    let cut = match room.rfind(char::is_whitespace) {
        Some(end) if end > 0 => &room[..end],
        _ => room.as_str(),
    };
    let cut = cut.trim_end_matches(|c: char| c.is_whitespace() || ",;:-—(".contains(c));
    format!("{cut}{ELLIPSIS}")
}

/// The longest prefix of `s` that fits in `max` bytes without splitting a
/// character.
pub(crate) fn truncate_bytes(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
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
    let escaped_name = yaml_escape(&truncate_chars(app_name, 35));
    let name = &escaped_name;
    let display_name = yaml_escape(display_name);
    let short_desc = yaml_escape(&build_short_description(app_name, app_description));
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
    let bot_scopes: String = slack_bot_scopes(agent_surface_enabled)
        .into_iter()
        .map(|scope| format!("      - {scope}\n"))
        .collect();
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
         \x20 description: \"{short_desc}\"\n\
         \x20 long_description: \"{long_desc}\"\n\
         \x20 background_color: \"#1a1a2e\"\n\
         features:\n\
         \x20 bot_user:\n\
         \x20   display_name: \"{display_name}\"\n\
         \x20   always_online: true\n\
         \x20 app_home:\n\
         \x20   home_tab_enabled: false\n\
         \x20   messages_tab_enabled: true\n\
         \x20   messages_tab_read_only_enabled: false\n\
         {agent_view}\
         oauth_config:\n\
         \x20 redirect_urls:\n\
         \x20   - \"{redirect_url}\"\n\
         \x20 scopes:\n\
         \x20   bot:\n\
         {bot_scopes}\
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
         \x20     - reaction_added\n\
         \x20     - reaction_removed\n\
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

/// Escape quoted YAML text, including authored multiline descriptions.
pub(crate) fn yaml_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
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
    if desc.chars().count() < SLACK_LONG_DESC_MIN {
        let pad = " Everruns lets you build, deploy, and run AI agents with durable execution, tool use, and real-time streaming. Learn more at https://everruns.com.";
        desc.push_str(pad);
    }

    truncate_chars(&desc, SLACK_LONG_DESC_MAX)
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

/// Manifests for the agents most likely to break Slack's limits: very long
/// names and descriptions, non-ASCII text, characters YAML must escape, the
/// description production refused, and every starter slot filled. The system
/// doctor sends each to Slack's `apps.manifest.validate`, and a committed
/// fixture keeps a copy for checks that run without a server.
pub(crate) fn worst_case_manifest_yamls() -> Vec<(&'static str, String)> {
    let long_ascii = "Watches agent frameworks for features worth adopting. ".repeat(100);
    let long_cyrillic = "Тримає розпорядок дня дітей і відповідає в Slack. ".repeat(100);
    let refused = "Daily watch of agent frameworks (Vercel AI SDK, LangGraph, DeepAgents, PydanticAI and more) for features worth adopting in Everruns or Yolop.";
    let tricky = "Quotes \"here\", a backslash \\, a tab\tand\nnew lines: ok? — yes # no";
    let long_name = "Дуже довга назва агента, що не влазить у ліміт Slack";
    let very_long_name = "x".repeat(200);
    let starters: Vec<ConversationStarter> = (0..8)
        .map(|i| ConversationStarter {
            icon: None,
            text: format!(
                "Starter {i}: {}",
                "a long prompt that keeps going ".repeat(20)
            ),
        })
        .collect();
    let cases: [(&'static str, &str, Option<&str>, bool); 6] = [
        ("plain", "Ecosystem Watch", Some("Answers questions"), false),
        ("refused-in-prod", "Ecosystem Watch", Some(refused), true),
        (
            "long-ascii",
            "Ecosystem Watch",
            Some(long_ascii.as_str()),
            true,
        ),
        (
            "long-cyrillic",
            long_name,
            Some(long_cyrillic.as_str()),
            true,
        ),
        ("tricky-characters", "Bot", Some(tricky), true),
        (
            "no-description-long-name",
            very_long_name.as_str(),
            None,
            true,
        ),
    ];
    cases
        .into_iter()
        .map(|(case, name, description, agent_surface)| {
            let yaml = build_manifest_yaml(
                name,
                &truncate_display_name(name),
                description,
                "https://example.com/events",
                "https://example.com/interactivity",
                "https://example.com/callback",
                agent_surface,
                &starters,
            );
            (case, yaml)
        })
        .collect()
}
