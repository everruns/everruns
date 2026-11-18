//! A `/v1` message's content on a data plane (step 11.6): text, images
//! and files.
//!
//! Decisions:
//! - Text-only content is admitted as text, as before.
//! - `image_file` and `file` parts refer to uploads, which are org records
//!   on the control plane. The edge sends a message with such parts
//!   through the control plane, which copies the uploads into the
//!   session's files first (`/attachments/<id>/<name>`, as on v1). Here
//!   they are passed on, and the node refuses one the session's files do
//!   not hold, so a turn never names bytes it cannot read.
//! - An inline image (`image` with `base64`) is written to the session's
//!   files here, like an upload, and the message refers to it by id: the
//!   bytes stay out of the session's log.
//! - An image by `url` is passed on as it is.
//! - Any other kind (audio, tool calls) is refused with its name.

use base64::Engine as _;
use everruns_contracts::runtime::attachments::attachment_path;
use everruns_contracts::runtime::message::{ContentPart, ImageFileContentPart};
use everruns_contracts::runtime::session_file::InitialFile;
use everruns_contracts::typed_id::ImageId;
use serde_json::Value;

/// One part of a message as it arrived.
#[derive(Debug)]
pub enum Part {
    /// A part to admit as it is.
    Ready(ContentPart),
    /// An inline image to write to the session's files first.
    Inline(ContentPart, InitialFile),
}

/// Why a message's content cannot be taken.
#[derive(Debug, PartialEq)]
pub enum Refused {
    /// A kind of part a data plane does not take (its name).
    Kind(String),
    /// A part of a kind it takes, but malformed.
    Invalid(String),
}

/// The parts of `message` (`{"role": "user", "content": [...]}`).
pub fn parts(message: &Value) -> Result<Vec<Part>, Refused> {
    let content = message["content"].as_array().cloned().unwrap_or_default();
    content.into_iter().map(part).collect()
}

fn part(raw: Value) -> Result<Part, Refused> {
    let kind = raw["type"].as_str().unwrap_or("unknown").to_owned();
    if !matches!(kind.as_str(), "text" | "image" | "image_file" | "file") {
        return Err(Refused::Kind(kind));
    }
    let part: ContentPart = serde_json::from_value(raw)
        .map_err(|error| Refused::Invalid(format!("invalid {kind} part: {error}")))?;
    match part {
        ContentPart::Image(image) => match (image.base64, image.url) {
            (Some(data), _) => inline(&data),
            (None, Some(url)) => Ok(Part::Ready(ContentPart::image_url(url))),
            (None, None) => Err(Refused::Invalid("an image part needs base64 or url".into())),
        },
        part => Ok(Part::Ready(part)),
    }
}

/// An inline image as a file in the session's attachments and the part
/// that refers to it.
fn inline(data: &str) -> Result<Part, Refused> {
    base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|error| Refused::Invalid(format!("an image's base64 does not decode: {error}")))?;
    let id = ImageId::new();
    let file = InitialFile {
        path: attachment_path(&id.to_string(), None, "image"),
        content: data.to_owned(),
        encoding: "base64".into(),
        is_readonly: true,
    };
    let part = ContentPart::ImageFile(ImageFileContentPart::new(id));
    Ok(Part::Inline(part, file))
}

/// The text of `parts` when they are only text.
pub fn text_only(parts: &[ContentPart]) -> Option<String> {
    let mut text = Vec::new();
    for part in parts {
        match part {
            ContentPart::Text(t) => text.push(t.text.clone()),
            _ => return None,
        }
    }
    Some(text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn of(content: Value) -> Result<Vec<Part>, Refused> {
        parts(&json!({ "role": "user", "content": content }))
    }

    #[test]
    fn text_images_and_files_are_taken_and_other_kinds_named() {
        let taken = of(json!([
            { "type": "text", "text": "look" },
            { "type": "image", "url": "https://example.com/a.png" },
            { "type": "image_file", "image_id": ImageId::new() },
            { "type": "file", "file_id": everruns_contracts::typed_id::FileId::new(), "filename": "a.pdf" },
        ]))
        .unwrap();
        assert_eq!(taken.len(), 4);
        assert!(taken.iter().all(|p| matches!(p, Part::Ready(_))));
        assert_eq!(
            of(json!([{ "type": "input_audio", "url": "x" }])).err(),
            Some(Refused::Kind("input_audio".into()))
        );
        assert!(matches!(
            of(json!([{ "type": "image_file" }])).err(),
            Some(Refused::Invalid(_))
        ));
        assert!(matches!(
            of(json!([{ "type": "image" }])).err(),
            Some(Refused::Invalid(_))
        ));
    }

    #[test]
    fn an_inline_image_becomes_a_read_only_attachment_it_refers_to() {
        let data = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n");
        let mut taken =
            of(json!([{ "type": "image", "base64": data, "media_type": "image/png" }])).unwrap();
        let Some(Part::Inline(ContentPart::ImageFile(image), file)) = taken.pop() else {
            panic!("an inline image is written to the session's files");
        };
        assert!(
            file.path
                .starts_with(&format!("/attachments/{}/", image.image_id))
        );
        assert!(file.is_readonly);
        assert_eq!(file.encoding, "base64");
        assert_eq!(file.content, data);
        assert!(matches!(
            of(json!([{ "type": "image", "base64": "not base64!" }])).err(),
            Some(Refused::Invalid(_))
        ));
    }

    #[test]
    fn only_text_parts_read_as_text() {
        assert_eq!(
            text_only(&[ContentPart::text("a"), ContentPart::text("b")]),
            Some("a\nb".into())
        );
        assert_eq!(text_only(&[ContentPart::image_url("https://x")]), None);
    }
}
