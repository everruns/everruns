// Desktop recordings: collected when a Session goes idle, kept as session files.
//
// Design: the provider records the desktop from the moment it opens
// (`open_desktop`). When the Session goes idle, which is the end of a turn, the
// server stops the recording, takes the video out of the Sandbox and writes it
// read-only to `/recordings/<name>` in the Session's files, announced with a
// `file.written` event so the chat can show it. The Sandbox is disposable and
// the session files are the record, so the video outlives it.
//
// Decision: collection runs on the server, not in a turn. It needs no model
// step, and the server already owns idle handling with a provider-capable
// context (`SessionSandboxService::tool_context`).
//
// Decision: one collection per Session at a time. An idle that arrives while
// one runs is skipped; whatever it would have found is still in the Sandbox
// for the next one.

use base64::Engine;
use everruns_capabilities::capabilities::sandbox_desktop::collect_desktop_recordings;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{EventContext, EventRequest, FILE_OP_CREATE, FileWrittenData};

use super::SessionSandboxService;
use crate::domains::session_files::queries::workspace_key_unscoped;
use crate::domains::session_files::service::{CreateFileInput, WorkspaceFileService};

/// Session-files folder the recordings are written to.
pub const RECORDINGS_DIR: &str = "/recordings";

impl SessionSandboxService {
    /// Collect the desktop recordings of an idle Session. Best effort: every
    /// failure is logged, none reaches the Session.
    pub async fn collect_recordings(&self, session_id: SessionId) {
        if !self.recordings_in_flight.lock().await.insert(session_id) {
            return;
        }
        if let Err(error) = self.collect_recordings_inner(session_id).await {
            tracing::warn!(session_id = %session_id, error = %error, "Desktop recording collection failed");
        }
        self.recordings_in_flight.lock().await.remove(&session_id);
    }

    async fn collect_recordings_inner(&self, session_id: SessionId) -> anyhow::Result<()> {
        let Some(config) = self.config_for_session(session_id).await? else {
            return Ok(());
        };
        if !config.desktop_recording() {
            return Ok(());
        }
        let context = self.tool_context(session_id).await;
        let recordings = collect_desktop_recordings(&context, &config)
            .await
            .map_err(|err| anyhow::anyhow!("{err:?}"))?;
        if recordings.is_empty() {
            return Ok(());
        }
        let Some(workspace_key) = workspace_key_unscoped(&self.db, session_id).await? else {
            return Ok(());
        };
        let files = WorkspaceFileService::new(self.db.clone());
        for recording in recordings {
            let path = format!("{RECORDINGS_DIR}/{}", recording.file_name);
            let Some(content) = recording.content else {
                tracing::warn!(
                    session_id = %session_id,
                    path = %path,
                    size_bytes = recording.size_bytes,
                    "Desktop recording too large to keep"
                );
                continue;
            };
            if self.db.session_file_exists(workspace_key, &path).await? {
                continue;
            }
            let file = files
                .create_file(
                    workspace_key,
                    CreateFileInput {
                        path,
                        content: Some(base64::engine::general_purpose::STANDARD.encode(&content)),
                        encoding: Some("base64".to_string()),
                        is_readonly: Some(true),
                    },
                )
                .await?;
            if let Some(events) = self.events() {
                let event = EventRequest::new(
                    session_id,
                    EventContext::empty(),
                    FileWrittenData {
                        path: file.path.clone(),
                        operation: FILE_OP_CREATE.into(),
                        size_bytes: file.size_bytes,
                        created: true,
                    },
                );
                if let Err(error) = events.emit(event).await {
                    tracing::warn!(error = %error, path = %file.path, "Failed to emit file.written for a desktop recording");
                }
            }
        }
        Ok(())
    }
}
