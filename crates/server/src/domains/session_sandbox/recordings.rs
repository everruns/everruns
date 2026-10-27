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
// Decision: the video streams from the Sandbox into storage, so a recording
// costs the server a chunk of memory, not its size, and is bounded only by the
// session-file limits (2 GiB per file with object storage). A recording is
// deleted from the Sandbox once stored, already stored, or too large to keep;
// a failed store leaves it there for the next collection.
//
// Decision: one collection per Session at a time. An idle that arrives while
// one runs is skipped; whatever it would have found is still in the Sandbox
// for the next one.

use everruns_capabilities::capabilities::sandbox_desktop::collect_desktop_recordings;
use everruns_contracts::session_sandbox::RecordingBody;
use everruns_contracts::typed_id::SessionId;
use everruns_core::events::{EventContext, EventRequest, FILE_OP_CREATE, FileWrittenData};
use futures::StreamExt;

use super::SessionSandboxService;
use crate::domains::session_files::limits::QuotaLimits;
use crate::domains::session_files::queries::workspace_key_unscoped;
use crate::domains::session_files::service::WorkspaceFileService;

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
        let per_file_max = QuotaLimits::from_env().per_file_bytes.max(0) as u64;
        for recording in recordings {
            let path = format!("{RECORDINGS_DIR}/{}", recording.file_name);
            let keep = match recording.body {
                Some(body) if recording.size_bytes <= per_file_max => {
                    if self.db.session_file_exists(workspace_key, &path).await? {
                        None
                    } else {
                        Some(body)
                    }
                }
                _ => {
                    tracing::warn!(
                        session_id = %session_id,
                        path = %path,
                        size_bytes = recording.size_bytes,
                        "Desktop recording too large to keep"
                    );
                    None
                }
            };
            let stored = match keep {
                Some(body) => {
                    self.store_recording(
                        session_id,
                        &files,
                        workspace_key,
                        &path,
                        recording.size_bytes,
                        body,
                    )
                    .await
                }
                None => true,
            };
            if stored && let Err(error) = recording.remove.await {
                tracing::warn!(session_id = %session_id, path = %path, error = %error, "Failed to delete a desktop recording from the sandbox");
            }
        }
        Ok(())
    }

    /// Stream one recording into the Session's files and announce it. Returns
    /// whether it was stored.
    async fn store_recording(
        &self,
        session_id: SessionId,
        files: &WorkspaceFileService,
        workspace_key: uuid::Uuid,
        path: &str,
        size_bytes: u64,
        body: RecordingBody,
    ) -> bool {
        let body = body
            .map(|chunk| chunk.map(bytes::Bytes::from).map_err(anyhow::Error::msg))
            .boxed();
        let declared = (size_bytes > 0).then_some(size_bytes);
        let file = match files
            .write_file_stream(workspace_key, path, body, declared, Some(true))
            .await
        {
            Ok(file) => file,
            Err(error) => {
                tracing::warn!(session_id = %session_id, path = %path, error = %error, "Failed to store a desktop recording; left in the sandbox");
                return false;
            }
        };
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
        true
    }
}
