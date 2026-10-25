//! AG-UI media parts as channel attachments.
//!
//! Decision: image, audio, video and document parts of the triggering user
//! message go through the channel host's attachment intake, like a Slack file:
//! inline bytes and `data:` URLs are stored as they are, `url` sources are
//! downloaded (public https only), and each becomes an attachment in
//! `/attachments` or a note. A provider `file` handle is not bytes Everruns can
//! read, so it is described by its id. Before this, `to_text()` dropped every
//! media part.

use base64::Engine;
use everruns_contracts::runtime::channel::InboundAttachment;
use everruns_core::ag_ui::{Content, ContentPart, MediaPart, PartSource};
use serde_json::Value;

/// The attachments in a user message's content, in order.
pub(super) fn media_attachments(content: &Content) -> Vec<InboundAttachment> {
    let Content::Parts(parts) = content else {
        return Vec::new();
    };
    parts
        .iter()
        .filter_map(|part| match part {
            ContentPart::Text(_) => None,
            ContentPart::Image(media)
            | ContentPart::Audio(media)
            | ContentPart::Video(media)
            | ContentPart::Document(media) => Some(attachment(media)),
        })
        .collect()
}

fn attachment(media: &MediaPart) -> InboundAttachment {
    let name = file_name(media);
    match &media.source {
        PartSource::Data { value, mime_type } => bytes(name, Some(mime_type.clone()), value),
        PartSource::Url { value, mime_type } => match value.strip_prefix("data:") {
            Some(data_url) => {
                let (header, data) = data_url.split_once(',').unwrap_or(("", ""));
                let declared = header
                    .strip_suffix(";base64")
                    .filter(|mime| !mime.is_empty())
                    .map(str::to_string)
                    .or_else(|| mime_type.clone());
                bytes(name, declared, data)
            }
            None => InboundAttachment::Remote {
                url: value.clone(),
                name,
                media_type: mime_type.clone(),
                size: None,
                credential: None,
            },
        },
        PartSource::File {
            value, mime_type, ..
        } => InboundAttachment::FileDescription {
            name: name.unwrap_or_else(|| format!("provider file {value}")),
            mime_type: mime_type.clone(),
        },
    }
}

fn bytes(name: Option<String>, media_type: Option<String>, base64: &str) -> InboundAttachment {
    match base64::engine::general_purpose::STANDARD.decode(base64.trim()) {
        Ok(data) => InboundAttachment::Bytes {
            name,
            media_type,
            data,
        },
        Err(_) => InboundAttachment::FileDescription {
            name: format!("{} (not valid base64)", name.as_deref().unwrap_or("file")),
            mime_type: media_type,
        },
    }
}

/// The part's file name, from `metadata.filename` or `metadata.name`.
fn file_name(media: &MediaPart) -> Option<String> {
    let metadata = media.metadata.as_ref()?;
    ["filename", "name"]
        .iter()
        .find_map(|key| metadata.get(key).and_then(Value::as_str))
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn content(parts: Value) -> Content {
        serde_json::from_value(parts).unwrap()
    }

    #[test]
    fn media_parts_become_attachments_in_order() {
        let content = content(json!([
            {"type": "text", "text": "look"},
            {"type": "document", "source": {"type": "data", "value": "JVBERi0xLjc=", "mimeType": "application/pdf"},
             "metadata": {"filename": "contract.pdf"}},
            {"type": "image", "source": {"type": "url", "value": "https://example.com/cat.png"}},
            {"type": "image", "source": {"type": "url", "value": "data:image/png;base64,iVBORw0KGgo="}},
            {"type": "document", "source": {"type": "file", "value": "file-abc", "provider": "openai"}},
            {"type": "document", "source": {"type": "data", "value": "%%%", "mimeType": "application/pdf"}}
        ]));

        let attachments = media_attachments(&content);

        assert_eq!(attachments.len(), 5);
        assert!(matches!(&attachments[0],
            InboundAttachment::Bytes { name: Some(name), media_type: Some(mt), data }
                if name == "contract.pdf" && mt == "application/pdf" && data.starts_with(b"%PDF-")));
        assert!(matches!(&attachments[1],
            InboundAttachment::Remote { url, credential: None, .. } if url == "https://example.com/cat.png"));
        assert!(matches!(&attachments[2],
            InboundAttachment::Bytes { media_type: Some(mt), data, .. }
                if mt == "image/png" && data.starts_with(b"\x89PNG")));
        assert!(matches!(&attachments[3],
            InboundAttachment::FileDescription { name, .. } if name == "provider file file-abc"));
        assert!(matches!(&attachments[4],
            InboundAttachment::FileDescription { name, .. } if name == "file (not valid base64)"));
    }
}
