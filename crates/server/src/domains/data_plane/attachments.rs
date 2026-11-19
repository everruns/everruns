//! A v2 session's attachments (data plane v2, step 11.6).
//!
//! Decisions:
//! - Uploads stay org records here (`/v1/images`, `/v1/files`). A message
//!   that refers to one is sent through the control plane: the edge passes
//!   such a message here instead of to the cell, and channels send theirs
//!   from here anyway. Each upload is copied into the session's files on
//!   its cell at the path a v1 session uses (`/attachments/<id>/<name>`,
//!   read-only, a zip unpacked next to it), then the message is admitted
//!   with its parts. The executor reads the bytes back from those files, so
//!   a cell never asks the control plane for them during a turn.
//! - A copy writes only where nothing is yet, so a retried send keeps the
//!   first copy and the agent's own files are never replaced.
//! - One upload per call keeps each call within the cell's file bound.
//! - An upload the org does not have rejects the message, as on v1.

use everruns_contracts::session_commands;
use std::sync::Arc;

use base64::Engine as _;
use everruns_contracts::runtime::attachments::{ZIP_MEDIA_TYPE, attachment_path, unpacked_dir};
use everruns_contracts::runtime::session_file::InitialFile;
use everruns_contracts::typed_id::SessionId;
use everruns_core::ContentPart;
use everruns_core::host::attachment_zip::unpack_zip;
use serde_json::json;

use super::DataPlane;
use crate::errors::BadRequestError;
use crate::storage::StorageBackend;

/// Whether `content` refers to an upload.
pub fn has_uploads(content: &[ContentPart]) -> bool {
    content
        .iter()
        .any(|part| matches!(part, ContentPart::ImageFile(_) | ContentPart::File(_)))
}

/// Copy every upload `content` refers to into `session`'s files on
/// `plane`.
pub async fn stage(
    db: &Arc<StorageBackend>,
    plane: &dyn DataPlane,
    org: &str,
    org_id: i64,
    session: SessionId,
    content: &[ContentPart],
) -> anyhow::Result<()> {
    for part in content {
        let files = match part {
            ContentPart::ImageFile(image) => {
                let id = image.image_id.to_string();
                let row = db
                    .get_image(org_id, image.image_id.uuid())
                    .await?
                    .ok_or_else(|| missing("image", &id))?;
                let name = image.filename.as_deref().unwrap_or(&row.filename);
                copies(attachment_path(&id, Some(name), "image"), &row.data, false)
            }
            ContentPart::File(file) => {
                let id = file.file_id.to_string();
                let row = db
                    .get_file(org_id, file.file_id.uuid())
                    .await?
                    .ok_or_else(|| missing("file", &id))?;
                let name = file.filename.as_deref().or(row.filename.as_deref());
                let zip = row.content_type == ZIP_MEDIA_TYPE;
                copies(attachment_path(&id, name, "file"), &row.data, zip)
            }
            _ => continue,
        };
        let reply = plane
            .session_command(
                org,
                session,
                &session_commands::FS,
                &json!({ "op": "seed", "files": files }),
            )
            .await?;
        if reply["result"] == "failed" {
            anyhow::bail!(
                "the attachment was not copied to the session: {}",
                reply["message"].as_str().unwrap_or("unknown reason")
            );
        }
    }
    Ok(())
}

fn missing(kind: &str, id: &str) -> anyhow::Error {
    BadRequestError::new(format!("Attached {kind} {id} not found")).into()
}

/// The files one upload becomes: itself, and a zip's entries next to it
/// (a zip that cannot be unpacked is kept whole, as on v1).
fn copies(path: String, data: &[u8], zip: bool) -> Vec<InitialFile> {
    let file = |path: String, bytes: &[u8]| InitialFile {
        path,
        content: base64::engine::general_purpose::STANDARD.encode(bytes),
        encoding: "base64".into(),
        is_readonly: true,
    };
    let mut files = Vec::new();
    if zip && let Ok(entries) = unpack_zip(data) {
        let dir = unpacked_dir(&path);
        files.extend(
            entries
                .iter()
                .map(|(relative, bytes)| file(format!("{dir}/{relative}"), bytes)),
        );
    }
    files.insert(0, file(path, data));
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::{FileId, ImageId};

    #[test]
    fn only_image_and_file_parts_are_uploads() {
        assert!(!has_uploads(&[ContentPart::text("hi")]));
        assert!(!has_uploads(&[ContentPart::image_url("https://x/a.png")]));
        assert!(has_uploads(&[ContentPart::image_file(ImageId::new())]));
        assert!(has_uploads(&[
            ContentPart::text("see"),
            ContentPart::File(everruns_core::message::FileContentPart::new(FileId::new())),
        ]));
    }

    #[test]
    fn an_upload_is_copied_read_only_and_a_broken_zip_kept_whole() {
        let copied = copies("/attachments/file_1/a.txt".into(), b"hello", false);
        assert_eq!(copied.len(), 1);
        assert_eq!(copied[0].path, "/attachments/file_1/a.txt");
        assert!(copied[0].is_readonly);
        assert_eq!(copied[0].encoding, "base64");
        let broken = copies("/attachments/file_2/b.zip".into(), b"not a zip", true);
        assert_eq!(broken.len(), 1);
    }
}
