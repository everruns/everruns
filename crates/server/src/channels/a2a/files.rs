//! A2A file parts as channel attachments.
//!
//! Decision: a `FilePart` of an incoming message (0.3 `{"kind": "file",
//! "file": {"bytes" | "uri", "mimeType", "name"}}`, or 1.0's flat `raw` /
//! `url` with `mediaType` and `filename`) goes through the channel host's
//! attachment intake: bytes are stored as they are, a URI is downloaded
//! (public https only), and each becomes an attachment in `/attachments` or a
//! note. Before this, file parts were dropped.

use base64::Engine;
use everruns_contracts::runtime::channel::InboundAttachment;
use serde_json::Value;

/// The attachments in a message's `parts`, in order.
pub(super) fn file_attachments(parts: &[Value]) -> Vec<InboundAttachment> {
    parts.iter().filter_map(file_attachment).collect()
}

fn file_attachment(part: &Value) -> Option<InboundAttachment> {
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_string);
    // 0.3 nests the file; 1.0 flattens it onto the part.
    let (bytes, uri, media_type, name) = match part.get("file") {
        Some(file) => (
            text(file.get("bytes")),
            text(file.get("uri")),
            text(file.get("mimeType")),
            text(file.get("name")),
        ),
        None => (
            text(part.get("raw")),
            text(part.get("url")),
            text(part.get("mediaType")),
            text(part.get("filename")),
        ),
    };
    if let Some(bytes) = bytes {
        return Some(
            match base64::engine::general_purpose::STANDARD.decode(bytes.trim()) {
                Ok(data) => InboundAttachment::Bytes {
                    name,
                    media_type,
                    data,
                },
                Err(_) => InboundAttachment::FileDescription {
                    name: format!("{} (not valid base64)", name.as_deref().unwrap_or("file")),
                    mime_type: media_type,
                },
            },
        );
    }
    Some(InboundAttachment::Remote {
        url: uri?,
        name,
        media_type,
        size: None,
        credential: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_parts_of_both_versions_become_attachments() {
        let parts = vec![
            json!({"kind": "text", "text": "summarize"}),
            json!({"kind": "file", "file": {"bytes": "JVBERi0xLjc=", "mimeType": "application/pdf", "name": "a.pdf"}}),
            json!({"kind": "file", "file": {"uri": "https://example.com/b.png", "mimeType": "image/png"}}),
            json!({"raw": "aGk=", "mediaType": "text/plain", "filename": "c.txt"}),
            json!({"url": "https://example.com/d.csv", "mediaType": "text/csv"}),
            json!({"data": {"k": 1}, "mediaType": "application/json"}),
        ];

        let attachments = file_attachments(&parts);

        assert_eq!(attachments.len(), 4);
        assert!(matches!(&attachments[0],
            InboundAttachment::Bytes { name: Some(name), data, .. } if name == "a.pdf" && data.starts_with(b"%PDF-")));
        assert!(matches!(&attachments[1],
            InboundAttachment::Remote { url, .. } if url == "https://example.com/b.png"));
        assert!(matches!(&attachments[2],
            InboundAttachment::Bytes { name: Some(name), data, .. } if name == "c.txt" && data == b"hi"));
        assert!(matches!(&attachments[3],
            InboundAttachment::Remote { url, media_type: Some(mt), .. }
                if url == "https://example.com/d.csv" && mt == "text/csv"));
    }
}
