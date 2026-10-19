//! Text labels for attachment parts (images, audio, files).
//!
//! Decision: summarization and masking work on text, so an attachment that
//! renders as nothing vanishes without a trace when its message is compacted.
//! A label keeps the agent aware that the file exists, and a summary gets a
//! deterministic list of the attachments it replaced.

// Design: knowledge/runtime-resources/session-attachments.md.
// Used by the engine (summary list) and builtins (masking, history search);
// a build with only one of them leaves the other's helpers unused.
#![cfg_attr(not(all(feature = "engine", feature = "builtins")), allow(dead_code))]

use crate::driver_registry::{LlmContentPart, Message, MessageContent};
use crate::message::ContentPart;
use everruns_contracts::runtime::attachments::is_attachment_label;

/// Media type of a `data:` URL, if it has one.
fn data_url_media_type(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("data:")?;
    let media_type = rest.split([';', ',']).next()?;
    (!media_type.is_empty()).then_some(media_type)
}

/// Short text label for an attachment part, or `None` for non-attachments.
///
/// Labels never carry the bytes: a `data:` URL is reduced to its media type,
/// and a remote URL is shown (bounded) because it is the only handle there is.
pub(crate) fn attachment_label(part: &LlmContentPart) -> Option<String> {
    match part {
        LlmContentPart::Image { url } => Some(match data_url_media_type(url) {
            Some(media_type) => format!("[Attached image ({media_type})]"),
            None if url.starts_with("data:") => "[Attached image]".to_string(),
            None => format!("[Attached image: {}]", truncate_at_char_boundary(url, 200)),
        }),
        LlmContentPart::Audio { url } => Some(match data_url_media_type(url) {
            Some(media_type) => format!("[Attached audio ({media_type})]"),
            None => "[Attached audio]".to_string(),
        }),
        LlmContentPart::File { url, filename } => {
            let media_type = data_url_media_type(url);
            Some(match (filename.as_deref(), media_type) {
                (Some(name), Some(media_type)) => {
                    format!("[Attached file: {name} ({media_type})]")
                }
                (Some(name), None) => format!("[Attached file: {name}]"),
                (None, Some(media_type)) => format!("[Attached file ({media_type})]"),
                (None, None) => "[Attached file]".to_string(),
            })
        }
        _ => None,
    }
}

/// Deterministic list of the attachments in `messages`, for the summary.
///
/// The summarizer may leave attachments out, so the runtime appends this list
/// itself: it is built from the messages being summarized, not by the LLM.
/// Returns `None` when there are no attachments.
pub(crate) fn attachment_list(messages: &[Message]) -> Option<String> {
    const MAX_LISTED: usize = 50;
    let mut labels: Vec<String> = Vec::new();
    for msg in messages {
        let MessageContent::Parts(parts) = &msg.content else {
            continue;
        };
        // A path label ("Attached file /attachments/...") already names the
        // attachment that follows it, so that part is not listed twice.
        let mut labelled = false;
        for part in parts {
            match part {
                LlmContentPart::Text { text } if is_attachment_label(text) => {
                    labels.push(text.clone());
                    labelled = true;
                }
                LlmContentPart::Text { .. } => labelled = false,
                other => {
                    if let Some(label) = attachment_label(other)
                        && !std::mem::take(&mut labelled)
                    {
                        labels.push(label);
                    }
                }
            }
        }
    }
    if labels.is_empty() {
        return None;
    }
    let mut lines: Vec<String> = labels
        .iter()
        .take(MAX_LISTED)
        .map(|label| format!("- {label}"))
        .collect();
    if labels.len() > MAX_LISTED {
        lines.push(format!("- and {} more", labels.len() - MAX_LISTED));
    }
    Some(format!(
        "Attachments from the summarized part of the conversation (their content is no longer shown):\n{}",
        lines.join("\n")
    ))
}

/// Append the attachment list for `summarized` to `summary_text`.
pub(crate) fn summary_with_attachment_list(summary_text: &str, summarized: &[Message]) -> String {
    match attachment_list(summarized) {
        Some(list) => format!("{summary_text}\n\n{list}"),
        None => summary_text.to_string(),
    }
}

/// Label for a stored-message attachment part, or `None` for other parts.
pub(crate) fn runtime_attachment_label(part: &ContentPart) -> Option<String> {
    match part {
        ContentPart::Image(image) => Some(match image.media_type.as_deref() {
            Some(media_type) if !media_type.starts_with("image/") => {
                format!("[Attached file ({media_type})]")
            }
            _ => "[Attached image]".to_string(),
        }),
        ContentPart::ImageFile(image) => Some(match &image.filename {
            Some(name) => format!("[Attached image: {name}]"),
            None => "[Attached image]".to_string(),
        }),
        ContentPart::File(file) => Some(match &file.filename {
            Some(name) => format!("[Attached file: {name}]"),
            None => "[Attached file]".to_string(),
        }),
        _ => None,
    }
}

fn truncate_at_char_boundary(content: &str, max_bytes: usize) -> &str {
    if content.len() <= max_bytes {
        return content;
    }
    let mut end = max_bytes;
    while end > 0 && !content.is_char_boundary(end) {
        end -= 1;
    }
    &content[..end]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::driver_registry::MessageRole;

    fn make_user_msg(text: &str) -> Message {
        Message::text(MessageRole::User, text)
    }

    fn make_assistant_msg(text: &str) -> Message {
        Message::text(MessageRole::Assistant, text)
    }

    fn user_msg_with_parts(parts: Vec<LlmContentPart>) -> Message {
        let mut msg = make_user_msg("");
        msg.content = MessageContent::Parts(parts);
        msg
    }

    #[cfg(feature = "builtins")]
    #[test]
    fn summarization_input_keeps_attachment_labels() {
        let messages = vec![user_msg_with_parts(vec![
            LlmContentPart::text("Please review this contract."),
            LlmContentPart::file(
                "data:application/pdf;base64,JVBERi0=",
                Some("contract.pdf".to_string()),
            ),
            LlmContentPart::image("data:image/png;base64,iVBORw0KGgo="),
        ])];

        let formatted = crate::builtins::format_messages_for_summarization(&messages);

        assert!(formatted.contains("Please review this contract."));
        assert!(formatted.contains("[Attached file: contract.pdf (application/pdf)]"));
        assert!(formatted.contains("[Attached image (image/png)]"));
        // Labels never carry the bytes.
        assert!(!formatted.contains("JVBERi0="));
        assert!(!formatted.contains("iVBORw0KGgo="));
    }

    #[test]
    fn summary_gets_a_deterministic_attachment_list() {
        let summarized = vec![
            make_user_msg("hello"),
            user_msg_with_parts(vec![LlmContentPart::file(
                "data:application/pdf;base64,JVBERi0=",
                Some("contract.pdf".to_string()),
            )]),
            user_msg_with_parts(vec![LlmContentPart::image("https://example.com/photo.jpg")]),
        ];

        let summary = summary_with_attachment_list("The user shared a contract.", &summarized);

        assert!(summary.starts_with("The user shared a contract.\n\nAttachments from"));
        assert!(summary.contains("- [Attached file: contract.pdf (application/pdf)]"));
        assert!(summary.contains("- [Attached image: https://example.com/photo.jpg]"));
    }

    #[test]
    fn summary_without_attachments_is_unchanged() {
        let summarized = vec![make_user_msg("hello"), make_assistant_msg("hi")];
        assert_eq!(
            summary_with_attachment_list("Greetings.", &summarized),
            "Greetings."
        );
    }

    #[test]
    fn attachment_list_is_capped() {
        let summarized: Vec<Message> = (0..60)
            .map(|i| {
                user_msg_with_parts(vec![LlmContentPart::file(
                    "data:application/pdf;base64,JVBERi0=",
                    Some(format!("doc-{i}.pdf")),
                )])
            })
            .collect();
        let list = attachment_list(&summarized).expect("list");
        assert!(list.contains("doc-49.pdf"));
        assert!(!list.contains("doc-50.pdf"));
        assert!(list.ends_with("- and 10 more"));
    }

    #[test]
    fn path_labelled_attachments_are_listed_once() {
        let summarized = vec![user_msg_with_parts(vec![
            LlmContentPart::text(
                "Attached file /attachments/file_01/contract.pdf (application/pdf)",
            ),
            LlmContentPart::file(
                "data:application/pdf;base64,JVBERi0=",
                Some("contract.pdf".to_string()),
            ),
        ])];

        let list = attachment_list(&summarized).expect("list");

        assert!(
            list.contains("- Attached file /attachments/file_01/contract.pdf (application/pdf)")
        );
        assert!(!list.contains("[Attached file: contract.pdf"));
    }
}
