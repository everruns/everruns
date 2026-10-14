//! Copies message attachments into the session's file storage.
//!
//! Decision: an uploaded image or file referenced by a message is written to
//! `/attachments/<id>/<file name>` in the session's file storage, read-only,
//! when the message is created. The model is told that path next to the
//! bytes, so it can open the file again with `read_file` or the shell after
//! compaction drops the bytes from the request. The copy also checks that the
//! upload belongs to the org, which used to happen only when a turn resolved
//! it. See knowledge/runtime-resources/session-attachments.md.

use std::sync::Arc;

use anyhow::Result;
use base64::Engine;
use everruns_contracts::runtime::attachments::{ZIP_MEDIA_TYPE, attachment_path, unpacked_dir};
use everruns_core::ContentPart;
use uuid::Uuid;

use crate::domains::session_files::service::{CreateFileInput, WorkspaceFileService};
use crate::errors::BadRequestError;
use crate::storage::StorageBackend;
use everruns_core::host::attachment_zip::unpack_zip;

/// One upload a message references, with the session path it is copied to.
struct Upload {
    kind: &'static str,
    id: Uuid,
    public_id: String,
    path: String,
}

fn referenced_uploads(content: &[ContentPart]) -> Vec<Upload> {
    content
        .iter()
        .filter_map(|part| match part {
            ContentPart::ImageFile(image) => Some(Upload {
                kind: "image",
                id: image.image_id.uuid(),
                public_id: image.image_id.to_string(),
                path: attachment_path(
                    &image.image_id.to_string(),
                    image.filename.as_deref(),
                    "image",
                ),
            }),
            ContentPart::File(file) => Some(Upload {
                kind: "file",
                id: file.file_id.uuid(),
                public_id: file.file_id.to_string(),
                path: attachment_path(&file.file_id.to_string(), file.filename.as_deref(), "file"),
            }),
            _ => None,
        })
        .collect()
}

/// Copy every upload `content` references into the session's file storage.
///
/// An upload that does not exist in the org rejects the message. A copy that
/// fails for another reason (such as the Workspace quota) is logged and
/// skipped: the model still receives the bytes, it just cannot reopen them.
pub(crate) async fn stage_message_attachments(
    db: &Arc<StorageBackend>,
    org_id: i64,
    session_id: Uuid,
    content: &[ContentPart],
) -> Result<()> {
    let uploads = referenced_uploads(content);
    if uploads.is_empty() {
        return Ok(());
    }
    let files = WorkspaceFileService::new(db.clone());
    for upload in uploads {
        if db.session_file_exists(session_id, &upload.path).await? {
            continue;
        }
        let upload_row = match upload.kind {
            "image" => db
                .get_image(org_id, upload.id)
                .await?
                .map(|row| (row.data, false)),
            _ => db
                .get_file(org_id, upload.id)
                .await?
                .map(|row| (row.data, row.content_type == ZIP_MEDIA_TYPE)),
        };
        let Some((bytes, is_zip)) = upload_row else {
            return Err(BadRequestError::new(format!(
                "Attached {} {} not found",
                upload.kind, upload.public_id
            ))
            .into());
        };
        let input = CreateFileInput {
            path: upload.path.clone(),
            content: Some(base64::engine::general_purpose::STANDARD.encode(&bytes)),
            encoding: Some("base64".to_string()),
            is_readonly: Some(true),
        };
        if let Err(error) = files.create_file(session_id, input).await {
            // A concurrent send of the same upload already wrote it.
            if db.session_file_exists(session_id, &upload.path).await? {
                continue;
            }
            tracing::warn!(
                %session_id,
                path = %upload.path,
                error = %error,
                "attachment not copied to session file storage"
            );
            continue;
        }
        if is_zip {
            unpack_into_session(&files, session_id, &upload.path, &bytes).await;
        }
    }
    Ok(())
}

/// Unpack a zip attachment into the folder next to it, read-only.
///
/// The upload passed the same check when it was stored, so a failure here is
/// storage (such as the Workspace quota): it is logged, and the files written
/// so far stay.
async fn unpack_into_session(
    files: &WorkspaceFileService,
    session_id: Uuid,
    zip_path: &str,
    bytes: &[u8],
) {
    let dir = unpacked_dir(zip_path);
    let entries = match unpack_zip(bytes) {
        Ok(entries) => entries,
        Err(reason) => {
            tracing::warn!(%session_id, path = %zip_path, %reason, "zip attachment not unpacked");
            return;
        }
    };
    for (relative, content) in entries {
        let input = CreateFileInput {
            path: format!("{dir}/{relative}"),
            content: Some(base64::engine::general_purpose::STANDARD.encode(&content)),
            encoding: Some("base64".to_string()),
            is_readonly: Some(true),
        };
        if let Err(error) = files.create_file(session_id, input).await {
            tracing::warn!(
                %session_id,
                path = %zip_path,
                error = %error,
                "zip attachment only partly unpacked"
            );
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::{FileId, ImageId};

    #[test]
    fn uploads_get_stable_paths() {
        let image = ImageId::new();
        let file = FileId::new();
        let content = vec![
            ContentPart::text("look"),
            ContentPart::image_file(image),
            ContentPart::file(file),
        ];

        let uploads = referenced_uploads(&content);

        assert_eq!(uploads.len(), 2);
        assert_eq!(uploads[0].path, format!("/attachments/{image}/image"));
        assert_eq!(uploads[1].path, format!("/attachments/{file}/file"));
    }

    async fn session(db: &StorageBackend, org_id: i64) -> Uuid {
        db.create_session(crate::storage::CreateSessionRow {
            playground_user_id: None,
            source: crate::domains::sessions::record::SessionSource::Api,
            workspace_id: None,
            org_id,
            harness_id: None,
            app_id: None,
            channel_id: None,
            trigger_id: None,
            agent_id: None,
            agent_revision: None,
            virtual_user_id: None,
            owner_principal_id: everruns_contracts::typed_id::PrincipalId::from_seed(
                org_id as u128,
            ),
            resolved_owner_user_id: None,
            title: None,
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: serde_json::json!([]),
            tools: serde_json::json!([]),
            mcp_servers: serde_json::json!({}),
            system_prompt: None,
            initial_files: serde_json::json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
        })
        .await
        .unwrap()
        .id
        .uuid()
    }

    async fn pdf(db: &StorageBackend, org_id: i64) -> FileId {
        db.create_file(
            org_id,
            crate::storage::CreateFileRow {
                org_id,
                filename: Some("contract.pdf".to_string()),
                content_type: "application/pdf".to_string(),
                size_bytes: 8,
                data: b"%PDF-1.7".to_vec(),
                metadata: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn referenced_upload_is_copied_read_only() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = session(&db, 1).await;
        let file_id = pdf(&db, 1).await;
        let content = vec![ContentPart::File(
            everruns_contracts::runtime::message::FileContentPart {
                file_id,
                filename: Some("contract.pdf".to_string()),
            },
        )];

        stage_message_attachments(&db, 1, session_id, &content)
            .await
            .unwrap();
        // Sending the same upload again is a no-op.
        stage_message_attachments(&db, 1, session_id, &content)
            .await
            .unwrap();

        let path = format!("/attachments/{file_id}/contract.pdf");
        let files = WorkspaceFileService::new(db.clone());
        let stored = files.read_file(session_id, &path).await.unwrap().unwrap();
        assert!(stored.is_readonly);
        let row = db
            .get_session_file(session_id, &path)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.content.as_deref(), Some(&b"%PDF-1.7"[..]));
        // The model cannot overwrite or delete it.
        assert!(files.delete(session_id, &path, false).await.is_err());
    }

    #[tokio::test]
    async fn zip_upload_is_unpacked_next_to_itself() {
        use std::io::Write;

        let db = Arc::new(StorageBackend::test_database());
        let session_id = session(&db, 1).await;
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        writer
            .start_file("data/report.csv", zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(b"a,b\n1,2\n").unwrap();
        let data = writer.finish().unwrap().into_inner();
        let file_id = db
            .create_file(
                1,
                crate::storage::CreateFileRow {
                    org_id: 1,
                    filename: Some("data.zip".to_string()),
                    content_type: "application/zip".to_string(),
                    size_bytes: data.len() as i64,
                    data,
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap()
            .id;
        let content = vec![ContentPart::File(
            everruns_contracts::runtime::message::FileContentPart {
                file_id,
                filename: Some("data.zip".to_string()),
            },
        )];

        stage_message_attachments(&db, 1, session_id, &content)
            .await
            .unwrap();

        let path = format!("/attachments/{file_id}/data/data/report.csv");
        let files = WorkspaceFileService::new(db.clone());
        let stored = files.read_file(session_id, &path).await.unwrap().unwrap();
        assert!(stored.is_readonly);
        let row = db
            .get_session_file(session_id, &path)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.content.as_deref(), Some(&b"a,b\n1,2\n"[..]));
    }

    #[tokio::test]
    async fn upload_from_another_org_is_rejected() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = session(&db, 1).await;
        let other_org_file = pdf(&db, 2).await;

        let err =
            stage_message_attachments(&db, 1, session_id, &[ContentPart::file(other_org_file)])
                .await
                .unwrap_err();

        assert!(
            err.downcast_ref::<BadRequestError>().is_some(),
            "got: {err}"
        );
        assert!(
            !db.session_file_exists(session_id, &format!("/attachments/{other_org_file}/file"))
                .await
                .unwrap()
        );
    }
}
