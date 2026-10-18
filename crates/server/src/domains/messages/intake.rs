//! Stores a file a channel received with a message as an org upload.
//!
//! Decision: a channel file becomes the same upload a chat user's file does
//! (an image row for PNG, JPEG, GIF and WebP, a file row for anything else),
//! and the message refers to it by id. Message creation then copies it into
//! the session's `/attachments` and the model gets its label and, for the
//! types it reads, its bytes, exactly as for an upload. The media type was
//! already checked against the bytes by the channel host. A zip must unpack
//! within the zip limits, as at upload. See
//! knowledge/runtime-resources/session-attachments.md.
//!
//! Surfaces that are not channels (the agent execution API, MCP tools) take
//! [`MessageFile`]s through the same core intake with [`receive_files`]: the
//! caller's org stores them and downloads are attributed to it.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_contracts::execution_api::MessageFile;
use everruns_contracts::runtime::attachments::ZIP_MEDIA_TYPE;
use everruns_contracts::runtime::channel::{ChannelError, InboundAttachment};
use everruns_contracts::runtime::message::{
    ContentPart, FileContentPart, ImageFileContentPart, InputContentPart,
};
use everruns_core::channel_runtime::{
    AttachmentIntake, AttachmentPolicy, AttachmentStore, ReceivedAttachment,
};
use everruns_core::{EgressScope, EgressService, ScopedEgressService};

use crate::storage::{CreateFileRow, CreateImageRow, StorageBackend};
use everruns_core::host::attachment_zip::unpack_zip;

/// Image types stored as image uploads, which the model sees as images.
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Store `attachment` as an upload of `org_id` and return the part that
/// refers to it.
pub(crate) async fn store_received_attachment(
    db: &Arc<StorageBackend>,
    org_id: i64,
    attachment: ReceivedAttachment,
) -> Result<ContentPart, ChannelError> {
    let ReceivedAttachment {
        name,
        media_type,
        data,
    } = attachment;
    let size_bytes = data.len() as i64;
    if IMAGE_TYPES.contains(&media_type.as_str()) {
        let row = db
            .create_image(
                org_id,
                CreateImageRow {
                    org_id,
                    filename: name.clone(),
                    content_type: media_type,
                    size_bytes,
                    data,
                    thumbnail_data: None,
                    thumbnail_content_type: None,
                    metadata: serde_json::json!({ "source": "channel" }),
                },
            )
            .await?;
        return Ok(ContentPart::ImageFile(ImageFileContentPart {
            image_id: row.id,
            filename: Some(name),
        }));
    }
    if media_type == ZIP_MEDIA_TYPE {
        unpack_zip(&data).map_err(|reason| {
            ChannelError::BadRequest(format!("the zip cannot be unpacked ({reason})"))
        })?;
    }
    let row = db
        .create_file(
            org_id,
            CreateFileRow {
                org_id,
                filename: Some(name.clone()),
                content_type: media_type,
                size_bytes,
                data,
                metadata: serde_json::json!({ "source": "channel" }),
            },
        )
        .await?;
    Ok(ContentPart::File(FileContentPart {
        file_id: row.id,
        filename: Some(name),
    }))
}

/// Stores files as uploads of one org.
struct OrgStore<'a> {
    db: &'a Arc<StorageBackend>,
    org_id: i64,
}

#[async_trait]
impl AttachmentStore for OrgStore<'_> {
    async fn store(&self, attachment: ReceivedAttachment) -> Result<ContentPart, ChannelError> {
        store_received_attachment(self.db, self.org_id, attachment).await
    }
}

/// `egress` with its requests attributed to org `org_id`.
pub(crate) fn org_egress(egress: Arc<dyn EgressService>, org_id: i64) -> Arc<dyn EgressService> {
    let org_id = everruns_core::org_public_id_from_internal(org_id)
        .parse()
        .ok();
    Arc::new(ScopedEgressService::new(
        egress,
        EgressScope {
            org_id,
            session_id: None,
        },
    ))
}

/// Input parts for `files` sent by a caller of org `org_id`, in order: each
/// stored as an upload of the org, or a text note saying why not. `source`
/// names the surface in logs.
pub(crate) async fn receive_files(
    db: &Arc<StorageBackend>,
    egress: Option<&Arc<dyn EgressService>>,
    org_id: i64,
    source: &str,
    files: Vec<MessageFile>,
) -> Vec<InputContentPart> {
    if files.is_empty() {
        return Vec::new();
    }
    let attachments: Vec<_> = files.into_iter().map(InboundAttachment::from).collect();
    AttachmentIntake {
        store: &OrgStore { db, org_id },
        egress: egress.map(|egress| org_egress(egress.clone(), org_id)),
        policy: AttachmentPolicy::default(),
        source,
    }
    .parts(&attachments)
    .await
    .into_iter()
    .filter_map(|part| match part {
        ContentPart::Text(text) => Some(InputContentPart::Text(text)),
        ContentPart::Image(image) => Some(InputContentPart::Image(image)),
        ContentPart::ImageFile(image) => Some(InputContentPart::ImageFile(image)),
        ContentPart::File(file) => Some(InputContentPart::File(file)),
        _ => None,
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn received(name: &str, media_type: &str, data: &[u8]) -> ReceivedAttachment {
        ReceivedAttachment {
            name: name.to_string(),
            media_type: media_type.to_string(),
            data: data.to_vec(),
        }
    }

    #[tokio::test]
    async fn images_and_files_become_uploads_of_the_org() {
        let db = Arc::new(StorageBackend::test_database());

        let image = store_received_attachment(
            &db,
            1,
            received("cat.png", "image/png", b"\x89PNG\r\n\x1a\n"),
        )
        .await
        .unwrap();
        let file = store_received_attachment(&db, 1, received("data.csv", "text/csv", b"a,b\n"))
            .await
            .unwrap();

        let ContentPart::ImageFile(image) = image else {
            panic!("expected an image upload, got {image:?}");
        };
        assert_eq!(image.filename.as_deref(), Some("cat.png"));
        assert!(
            db.get_image(1, image.image_id.uuid())
                .await
                .unwrap()
                .is_some()
        );
        let ContentPart::File(file) = file else {
            panic!("expected a file upload, got {file:?}");
        };
        let row = db.get_file(1, file.file_id.uuid()).await.unwrap().unwrap();
        assert_eq!(row.content_type, "text/csv");
        assert!(db.get_file(2, file.file_id.uuid()).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn a_zip_that_cannot_be_unpacked_is_refused() {
        let db = Arc::new(StorageBackend::test_database());

        let error = store_received_attachment(
            &db,
            1,
            received("bad.zip", ZIP_MEDIA_TYPE, b"PK\x03\x04junk"),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&error, ChannelError::BadRequest(reason) if reason.starts_with("the zip cannot be unpacked")),
            "got {error:?}"
        );
    }

    #[tokio::test]
    async fn caller_files_are_stored_or_noted() {
        let db = Arc::new(StorageBackend::test_database());
        let file = |data: Option<&str>, url: Option<&str>| MessageFile {
            name: Some("notes.txt".into()),
            media_type: Some("text/plain".into()),
            data: data.map(str::to_string),
            url: url.map(str::to_string),
        };

        let parts = receive_files(
            &db,
            None,
            1,
            "api",
            vec![
                file(Some("aGk="), None),
                file(None, Some("https://example.com/notes.txt")),
                file(None, None),
                file(Some("%%%"), None),
            ],
        )
        .await;

        let InputContentPart::File(stored) = &parts[0] else {
            panic!("expected a stored file, got {:?}", parts[0]);
        };
        let row = db
            .get_file(1, stored.file_id.uuid())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.data, b"hi");
        let notes: Vec<_> = parts[1..]
            .iter()
            .map(|part| part.as_text().unwrap_or_default().to_string())
            .collect();
        assert_eq!(
            notes,
            [
                "[Attachment notes.txt not received: this host does not download files]",
                "[Attached file: notes.txt (needs exactly one of data and url) (text/plain)]",
                "[Attached file: notes.txt (not valid base64) (text/plain)]",
            ]
        );
    }
}
