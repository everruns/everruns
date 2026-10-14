//! Files a message brings, kept in the session's own file storage (see
//! `crate::host::attachment_files`).

use super::super::*;

impl InProcessRuntime {
    /// Write a file into `session_id`'s files at `/attachments/<id>/<name>`,
    /// read-only, and return the part a message refers to it with. The model
    /// gets it with the message and can open it again later, including after
    /// compaction. `media_type` is what the bytes were checked to be. Errors
    /// are short reasons fit to show the model (`the zip cannot be unpacked
    /// (...)`).
    pub async fn stage_attachment(
        &self,
        session_id: SessionId,
        name: &str,
        media_type: &str,
        data: &[u8],
    ) -> std::result::Result<ContentPart, String> {
        let snapshot = self
            .resolved_execution_snapshot(session_id)
            .await
            .map_err(|error| {
                tracing::warn!(%session_id, %error, "attachment: session not found");
                "the session could not be found".to_string()
            })?;
        let files = everruns_contracts::runtime::mount_fs::scoped_prompt_file_store(
            self.file_store.clone(),
            snapshot.workspace_id,
        );
        crate::host::stage_attachment(files.as_ref(), session_id, name, media_type, data).await
    }
}
