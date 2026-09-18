//! Turning Slack files and attachments into agent input, plus session naming.

use everruns_core::channel::{SessionBinding, build_session_routing_tag, resolve_session_binding};
use everruns_core::progress_reporting::sync_slack_reply_mode_tags;
use everruns_platform::{SlackChannelConfig, SlackReplyMode};
use everruns_provider::url_validation::validate_safe_url;
use std::collections::HashMap;

use crate::api::messages::InputContentPart;
use crate::slack_delivery::SlackSurface;

use super::*;

/// Supported image MIME types for Slack file attachments.
pub(crate) const SUPPORTED_IMAGE_TYPES: &[&str] =
    &["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Validate and normalize image URLs before forwarding to model backends.
///
/// Security: only allow publicly routable HTTPS URLs to avoid SSRF through
/// model backends that may fetch `image_url` server-side.
pub(crate) fn validated_image_url(url: &str) -> Option<String> {
    let parsed = validate_safe_url(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    Some(parsed.into())
}

/// Convert Slack file attachments into content parts.
///
/// Images (png, jpeg, gif, webp) with a private URL → `InputContentPart::Image(url)`.
/// All other files → text description noting the file name and type.
pub(crate) fn build_file_content_parts(files: &[SlackFile]) -> Vec<InputContentPart> {
    files
        .iter()
        .map(|file| {
            let mime = file.mimetype.as_deref().unwrap_or("");
            let name = file.name.as_deref().unwrap_or("unnamed file");

            if SUPPORTED_IMAGE_TYPES.contains(&mime) {
                // Image with URL → image content part
                if let Some(url) = &file.url_private {
                    if let Some(url) = validated_image_url(url) {
                        return InputContentPart::image_url(url);
                    }
                    return InputContentPart::text(format!(
                        "[Attached image: {name} — blocked unsafe image URL]"
                    ));
                }
                // Image without URL → text fallback
                InputContentPart::text(format!(
                    "[Attached image: {name} — no download URL available]"
                ))
            } else {
                // Non-image file → text description (content not available inline)
                let filetype = file.filetype.as_deref().unwrap_or("unknown");
                InputContentPart::text(format!("[Attached file: {name} ({filetype})]"))
            }
        })
        .collect()
}

/// Convert Slack legacy attachments (link unfurls, app attachments, etc.) into content parts.
///
/// Attachments with an image_url → `InputContentPart::Image(url)`.
/// Others → text summary from title/text/fallback fields.
pub(crate) fn build_attachment_content_parts(
    attachments: &[SlackAttachment],
) -> Vec<InputContentPart> {
    attachments
        .iter()
        .filter_map(|att| {
            // If the attachment has an image, include it
            if let Some(url) = &att.image_url {
                if let Some(url) = validated_image_url(url) {
                    return Some(InputContentPart::image_url(url));
                }
                let title = att.title.as_deref().unwrap_or("unnamed attachment");
                return Some(InputContentPart::text(format!(
                    "[Attachment image blocked: {title}]"
                )));
            }

            // Build a text summary from available fields
            let mut parts = Vec::new();
            if let Some(service) = &att.service_name {
                parts.push(service.clone());
            }
            if let Some(title) = &att.title {
                let line = if let Some(link) = &att.title_link {
                    format!("{title} ({link})")
                } else {
                    title.clone()
                };
                parts.push(line);
            }
            if let Some(text) = &att.text {
                parts.push(text.clone());
            }

            if parts.is_empty() {
                // Use fallback as last resort
                att.fallback
                    .as_ref()
                    .map(|f| InputContentPart::text(format!("[Attachment: {f}]")))
            } else {
                Some(InputContentPart::text(format!(
                    "[Attachment: {}]",
                    parts.join(" — ")
                )))
            }
        })
        .collect()
}

pub(crate) fn build_session_tags(
    app: &crate::api::app_ingress::IngressContext,
    slack_channel: &crate::api::app_ingress::IngressEndpoint,
    slack_config: &SlackChannelConfig,
    event: &SlackEvent,
    surface: SlackSurface,
) -> Vec<String> {
    let mut tags = vec![
        format!("slack:app:{}", app.public_id),
        format!("slack:endpoint:{}", slack_channel.public_id),
    ];

    // Build routing metadata from the Slack event
    let mut routing_metadata = HashMap::new();
    let thread_ref = event
        .thread_ts
        .as_deref()
        .or(event.ts.as_deref())
        .unwrap_or("unknown");
    routing_metadata.insert("thread_ref".to_string(), thread_ref.to_string());
    if let Some(ref channel) = event.channel {
        routing_metadata.insert("channel_id".to_string(), channel.clone());
    }
    if let Some(ref user) = event.user {
        routing_metadata.insert("user_id".to_string(), user.clone());
    }

    // An agent pane is inherently one thread, so `Conversation` and `Requester`
    // have no meaning there. Rejecting the combination at config time would be
    // wrong — the same exposure also serves channels, where those bindings are
    // legitimate — so the pane overrides per event and config keeps meaning what
    // it says for channels.
    //
    // The override goes through `resolve_session_binding` rather than a match
    // here, so the pane is an instance of a general rule instead of a Slack
    // special case (EVE-1005).
    let binding = resolve_session_binding(
        slack_config.session_strategy,
        match surface {
            SlackSurface::Pane => Some(SessionBinding::Thread),
            SlackSurface::Channel => None,
        },
    );
    if let Some(routing_tag) = build_session_routing_tag("slack", &binding, &routing_metadata) {
        tags.push(routing_tag);
    }

    tags
}

pub(crate) fn desired_session_tags(
    routing_tags: &[String],
    reply_mode: SlackReplyMode,
) -> Vec<String> {
    let mut tags = routing_tags.to_vec();
    sync_slack_reply_mode_tags(&mut tags, reply_mode.into());
    tags
}

/// Build a human-readable session title.
pub(crate) fn build_session_title(slack_config: &SlackChannelConfig, event: &SlackEvent) -> String {
    let channel = event.channel.as_deref().unwrap_or("unknown");
    match slack_config.session_strategy {
        SessionBinding::Thread => {
            let ts = event
                .thread_ts
                .as_deref()
                .or(event.ts.as_deref())
                .unwrap_or("?");
            format!("Slack thread {} in {}", ts, channel)
        }
        SessionBinding::Conversation => format!("Slack channel {}", channel),
        SessionBinding::Requester => {
            let user = event.user.as_deref().unwrap_or("unknown");
            format!("Slack user {} in {}", user, channel)
        }
        // Not offerable on Slack — `ChannelType::Slack.allowed_bindings()`
        // rejects both at write time. Reachable only from a config stored
        // before that guard existed, so title it by the channel rather than
        // panicking on a live inbound event (EVE-1005).
        SessionBinding::Endpoint | SessionBinding::Ephemeral => {
            format!("Slack channel {}", channel)
        }
    }
}
