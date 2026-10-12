//! Turning Slack files and attachments into agent input.

use everruns_contracts::url_validation::validate_safe_url;

use crate::api::messages::InputContentPart;

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
