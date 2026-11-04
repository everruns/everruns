//! Moves a tool result's inline images out of the event and into the image store.
//!
//! Decisions:
//! - Screenshots from browser and desktop tools arrive as base64 inside
//!   `tool.completed`. Kept inline, every later turn reloads them with the
//!   history, which is what grew sessions past the worker message cap
//!   (#4948). At the event write path each inline raster image is saved as an
//!   image row of the session's org and replaced with an `image_file`
//!   reference, so history stays small however many screenshots a session
//!   takes.
//! - Only `image/*` parts move. Tool results also carry PDFs through the same
//!   inline part; those stay as they are.
//! - The model sees the same bytes: `image_file` in a tool result converts back
//!   to the identical inline image, without an attachment label
//!   (`llm_conversions`).
//! - Best effort: if an image cannot be stored, it stays inline and the event
//!   is still written. Losing a tool result is worse than a large row.
//! - Events already stored are not rewritten here.

use crate::storage::{CreateImageRow, StorageBackend};
use base64::Engine;
use everruns_contracts::runtime::message::{ContentPart, ImageContentPart, ImageFileContentPart};
use everruns_core::events::EventData;

/// Same bound as an uploaded image.
const MAX_OFFLOAD_BYTES: usize = 100 * 1024 * 1024;

/// Whether `part` is an inline raster image this module would move.
fn inline_raster(part: &ContentPart) -> Option<(&str, &str)> {
    match part {
        ContentPart::Image(ImageContentPart {
            url: None,
            base64: Some(data),
            media_type: Some(media_type),
        }) if media_type.starts_with("image/") && !media_type.starts_with("image/svg") => {
            Some((data.as_str(), media_type.as_str()))
        }
        _ => None,
    }
}

/// Whether a `tool.completed` event carries anything to move. Cheap; lets the
/// caller skip the session lookup for the common text-only result.
pub(super) fn has_inline_images(data: &EventData) -> bool {
    matches!(data, EventData::ToolCompleted(completed)
        if completed.result.as_ref().is_some_and(|parts| parts.iter().any(|p| inline_raster(p).is_some())))
}

/// Replace inline raster images in a `tool.completed` result with references
/// to image rows of `org_id`.
pub(super) async fn offload(db: &StorageBackend, org_id: i64, data: &mut EventData) {
    let EventData::ToolCompleted(completed) = data else {
        return;
    };
    let tool_name = completed.tool_name.clone();
    let Some(parts) = completed.result.as_mut() else {
        return;
    };
    for (index, part) in parts.iter_mut().enumerate() {
        let Some((encoded, media_type)) = inline_raster(part) else {
            continue;
        };
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(encoded) else {
            tracing::warn!(tool = %tool_name, "tool image is not valid base64; kept inline");
            continue;
        };
        if bytes.len() > MAX_OFFLOAD_BYTES {
            continue;
        }
        let extension = media_type
            .trim_start_matches("image/")
            .split(['+', ';'])
            .next()
            .unwrap_or("img");
        let filename = format!("{tool_name}-{index}.{extension}");
        let stored = db
            .create_image(
                org_id,
                CreateImageRow {
                    org_id,
                    filename: filename.clone(),
                    content_type: media_type.to_string(),
                    size_bytes: bytes.len() as i64,
                    data: bytes,
                    thumbnail_data: None,
                    thumbnail_content_type: None,
                    metadata: serde_json::json!({ "source": "tool_result", "tool": tool_name }),
                },
            )
            .await;
        match stored {
            Ok(row) => {
                *part = ContentPart::ImageFile(ImageFileContentPart {
                    image_id: row.id,
                    filename: Some(filename),
                });
            }
            Err(error) => {
                tracing::warn!(tool = %tool_name, error = %error, "failed to store tool image; kept inline");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::ToolCompletedData;

    fn completed(result: Vec<ContentPart>) -> EventData {
        let mut data: ToolCompletedData = serde_json::from_value(serde_json::json!({
            "tool_call_id": "call_1",
            "tool_name": "browser_screenshot",
            "success": true,
            "status": "success",
        }))
        .unwrap();
        data.result = Some(result);
        EventData::ToolCompleted(data)
    }

    #[test]
    fn only_inline_raster_images_count() {
        let png = ContentPart::Image(ImageContentPart::from_base64("aGk=", "image/png"));
        let pdf = ContentPart::Image(ImageContentPart::from_base64("aGk=", "application/pdf"));
        let svg = ContentPart::Image(ImageContentPart::from_base64("aGk=", "image/svg+xml"));
        let linked = ContentPart::Image(ImageContentPart::from_url("https://example.com/a.png"));

        assert!(has_inline_images(&completed(vec![png])));
        assert!(!has_inline_images(&completed(vec![pdf, svg, linked])));
        assert!(!has_inline_images(&completed(vec![ContentPart::text(
            "ok"
        )])));
    }
}
