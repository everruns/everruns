//! Attachments kept in the session's own file storage, for hosts with no
//! upload store (the Framework, serve).
//!
//! Decision: such a host writes each file a message brings straight into the
//! session's files at `/attachments/<id>/<name>`, read-only, the same path the
//! server copies its uploads to, and the message refers to it by id
//! ([`ContentPart::ImageFile`] / [`ContentPart::File`]). When a request is
//! built, [`SessionAttachmentFiles`] reads the bytes back from that path, so
//! the files last as long as the session's storage does and need no store or
//! index of their own. A zip is unpacked next to itself, as on the server.

// Design: knowledge/runtime-resources/session-attachments.md.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine as _;
use uuid::Uuid;

use everruns_contracts::error::Result;
use everruns_contracts::runtime::attachments::{
    ATTACHMENTS_ROOT, ZIP_MEDIA_TYPE, attachment_path, sniff_media_type, unpacked_dir,
};
use everruns_contracts::runtime::message::{ContentPart, FileContentPart, ImageFileContentPart};
use everruns_contracts::runtime::session_file::InitialFile;
use everruns_contracts::runtime::session_files::SessionFileSystem;
use everruns_contracts::typed_id::{FileId, ImageId, SessionId};

use crate::file_services::{FileResolver, ResolvedFile};
use crate::image_services::{ImageResolver, ResolvedImage};

use super::attachment_zip::unpack_zip;

/// Image types the model reads; other files are kept as files.
const IMAGE_TYPES: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];

/// Write a file into `files` under `key` (the session's storage key) at its
/// attachment path, read-only, and return the part that refers to it.
/// `media_type` must already be checked against the bytes. A zip is unpacked
/// next to itself; one that cannot be unpacked is refused with the reason.
pub async fn stage_attachment(
    files: &dyn SessionFileSystem,
    key: SessionId,
    name: &str,
    media_type: &str,
    data: &[u8],
) -> std::result::Result<ContentPart, String> {
    let zip_entries = if media_type == ZIP_MEDIA_TYPE {
        Some(unpack_zip(data).map_err(|reason| format!("the zip cannot be unpacked ({reason})"))?)
    } else {
        None
    };
    let (part, path) = if IMAGE_TYPES.contains(&media_type) {
        let id = ImageId::new();
        let path = attachment_path(&id.to_string(), Some(name), "image");
        let part = ContentPart::ImageFile(ImageFileContentPart {
            image_id: id,
            filename: Some(name.to_string()),
        });
        (part, path)
    } else {
        let id = FileId::new();
        let path = attachment_path(&id.to_string(), Some(name), "file");
        let mut part = FileContentPart::new(id);
        part.filename = Some(name.to_string());
        (ContentPart::File(part), path)
    };
    seed(files, key, &path, data).await?;
    if let Some(entries) = zip_entries {
        let dir = unpacked_dir(&path);
        for (relative, content) in entries {
            seed(files, key, &format!("{dir}/{relative}"), &content).await?;
        }
    }
    Ok(part)
}

async fn seed(
    files: &dyn SessionFileSystem,
    key: SessionId,
    path: &str,
    data: &[u8],
) -> std::result::Result<(), String> {
    // Seeding is the host's own write: it passes the workspace policy that
    // keeps the model from writing `/attachments`.
    files
        .seed_initial_file(
            key,
            &InitialFile {
                path: path.to_string(),
                content: base64::engine::general_purpose::STANDARD.encode(data),
                encoding: "base64".to_string(),
                is_readonly: true,
            },
        )
        .await
        .map_err(|error| {
            tracing::warn!(%path, %error, "attachment not written to session files");
            "it could not be stored".to_string()
        })
}

/// Resolves the image and file parts of one session's messages from its
/// `/attachments` folder: `/attachments/<id>/<name>`, where `<id>` is the
/// part's public id.
pub struct SessionAttachmentFiles {
    files: Arc<dyn SessionFileSystem>,
    key: SessionId,
}

impl SessionAttachmentFiles {
    /// Resolver over `files` under `key`, the session's storage key.
    pub fn new(files: Arc<dyn SessionFileSystem>, key: SessionId) -> Self {
        Self { files, key }
    }

    /// Resolver for the session `context` runs in, over its workspace when
    /// it is attached to one.
    pub fn for_session(
        files: Arc<dyn SessionFileSystem>,
        context: &everruns_contracts::runtime::execution_context::ExecutionContext,
    ) -> Self {
        let files = match context.workspace_id {
            Some(workspace_id) => {
                everruns_contracts::runtime::mount_fs::scoped_prompt_file_store(files, workspace_id)
            }
            None => files,
        };
        Self::new(files, context.session_id)
    }

    /// The bytes and file name of the attachment `<public id>`, if any.
    async fn read(&self, public_id: &str) -> Result<Option<(Vec<u8>, String)>> {
        let dir = format!("{ATTACHMENTS_ROOT}/{public_id}");
        let Some(entry) = self
            .files
            .list_directory(self.key, &dir)
            .await
            .unwrap_or_default()
            .into_iter()
            .find(|entry| !entry.is_directory)
        else {
            return Ok(None);
        };
        let Some(file) = self.files.read_file(self.key, &entry.path).await? else {
            return Ok(None);
        };
        let content = file.content.unwrap_or_default();
        let bytes = if file.encoding == "base64" {
            base64::engine::general_purpose::STANDARD
                .decode(content.as_bytes())
                .unwrap_or_default()
        } else {
            content.into_bytes()
        };
        Ok(Some((bytes, file.name)))
    }
}

/// Media type of stored bytes: what they show, else text when they are UTF-8.
fn media_type_of(bytes: &[u8]) -> &'static str {
    match sniff_media_type(bytes) {
        Some(sniffed) => sniffed,
        None if std::str::from_utf8(bytes).is_ok() => "text/plain",
        None => "application/octet-stream",
    }
}

#[async_trait]
impl ImageResolver for SessionAttachmentFiles {
    async fn resolve_image(&self, image_id: Uuid) -> Result<Option<ResolvedImage>> {
        let id = ImageId::from_uuid(image_id).to_string();
        Ok(self.read(&id).await?.map(|(bytes, _)| {
            ResolvedImage::new(
                base64::engine::general_purpose::STANDARD.encode(&bytes),
                media_type_of(&bytes),
            )
        }))
    }
}

#[async_trait]
impl FileResolver for SessionAttachmentFiles {
    async fn resolve_files(&self, ids: &[Uuid]) -> Result<HashMap<Uuid, ResolvedFile>> {
        let mut resolved = HashMap::new();
        for id in ids {
            let public = FileId::from_uuid(*id).to_string();
            if let Some((bytes, name)) = self.read(&public).await? {
                resolved.insert(
                    *id,
                    ResolvedFile {
                        media_type: media_type_of(&bytes).to_string(),
                        base64: base64::engine::general_purpose::STANDARD.encode(&bytes),
                        filename: Some(name),
                    },
                );
            }
        }
        Ok(resolved)
    }
}

#[cfg(test)]
#[path = "attachment_files_tests.rs"]
mod tests;
