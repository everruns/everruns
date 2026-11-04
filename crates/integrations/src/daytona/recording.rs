//! Video recordings of a Daytona Sandbox desktop.
//!
//! The toolbox Computer Use API records the screen itself
//! (`/computeruse/recordings/*`) into a file inside the Sandbox. The provider
//! starts a recording when the desktop opens and the server collects it when
//! the Session goes idle (`collect_desktop_recordings`), so each turn's desktop
//! work becomes one video that outlives the disposable Sandbox.
//!
//! Decision: no recording state of our own. The Sandbox's recording list is
//! the record: a recording without an end time is the one running, and a
//! stored recording is deleted from the Sandbox, so whatever is listed has
//! not been stored yet. The video streams out of the Sandbox, so it is deleted
//! only once the collector has stored it ([`DesktopRecording::remove`]). A turn that starts while the last one is still being
//! collected may lose its first seconds of video, never the whole turn: its
//! next desktop action starts a new recording.

use std::sync::Arc;

use everruns_contracts::session_sandbox::DesktopRecording;
use futures::FutureExt;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::daytona::client::DaytonaClient;
use crate::daytona::computer::computer_use;

/// Largest recording collected: the object-storage per-file limit. A bigger
/// file is still reported, without its video. The collector applies its own
/// storage limit on top.
pub const MAX_RECORDING_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// Label on recordings Everruns starts.
const RECORDING_LABEL: &str = "everruns";

/// One entry of `GET /computeruse/recordings`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    pub id: String,
    pub file_name: String,
    pub file_path: String,
    pub status: String,
    pub start_time: String,
    #[serde(default)]
    pub end_time: Option<String>,
    #[serde(default)]
    pub duration_seconds: Option<f64>,
    #[serde(default)]
    pub size_bytes: Option<u64>,
}

impl Recording {
    /// Still recording: no end time and not in a final state.
    pub fn is_active(&self) -> bool {
        self.end_time.as_deref().is_none_or(str::is_empty)
            && !matches!(
                self.status.as_str(),
                "completed" | "failed" | "stopped" | "error"
            )
    }

    fn failed(&self) -> bool {
        matches!(self.status.as_str(), "failed" | "error")
    }

    /// The name the video is stored under: its start time, which sorts in
    /// recording order, with the file's own extension.
    pub fn stored_name(&self) -> String {
        let stamp: String = self
            .start_time
            .chars()
            .map(|c| if c == ':' { '-' } else { c })
            .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_'))
            .collect();
        let extension = self
            .file_name
            .rsplit_once('.')
            .map(|(_, ext)| ext)
            .filter(|ext| !ext.is_empty() && ext.chars().all(|c| c.is_ascii_alphanumeric()))
            .unwrap_or("mp4");
        format!("desktop-{stamp}.{extension}")
    }
}

/// A recording id as one URL path segment. Daytona ids are UUIDs; anything
/// else is refused rather than escaped into the path.
fn path_segment(id: &str) -> Result<&str, String> {
    if !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
    {
        Ok(id)
    } else {
        Err(format!("unexpected recording id `{id}`"))
    }
}

pub async fn list(client: &DaytonaClient, sandbox_id: &str) -> Result<Vec<Recording>, String> {
    let listing = computer_use(client, sandbox_id, Method::GET, "/recordings", None).await?;
    let recordings = listing.get("recordings").cloned().unwrap_or(Value::Null);
    if recordings.is_null() {
        return Ok(Vec::new());
    }
    serde_json::from_value(recordings).map_err(|e| format!("unreadable recording list: {e}"))
}

/// Start recording the desktop unless a recording already runs.
pub async fn ensure_started(client: &DaytonaClient, sandbox_id: &str) -> Result<(), String> {
    if list(client, sandbox_id)
        .await?
        .iter()
        .any(Recording::is_active)
    {
        return Ok(());
    }
    computer_use(
        client,
        sandbox_id,
        Method::POST,
        "/recordings/start",
        Some(json!({ "label": RECORDING_LABEL })),
    )
    .await
    .map(|_| ())
}

/// Stop every running recording and hand back the finished ones, each with
/// its video as a stream and a handle that deletes it from the Sandbox.
/// Failed recordings are deleted here, without a download.
pub async fn collect(
    client: Arc<DaytonaClient>,
    sandbox_id: &str,
) -> Result<Vec<DesktopRecording>, String> {
    let mut finished = Vec::new();
    for recording in list(&client, sandbox_id).await? {
        if !recording.is_active() {
            finished.push(recording);
            continue;
        }
        let stopped = computer_use(
            &client,
            sandbox_id,
            Method::POST,
            "/recordings/stop",
            Some(json!({ "id": recording.id })),
        )
        .await?;
        // The stop answer carries the final size and duration; fall back to
        // the listed entry when it does not parse.
        finished.push(serde_json::from_value(stopped).unwrap_or(recording));
    }

    let mut collected = Vec::new();
    for recording in finished {
        let delete_path = format!("/recordings/{}", path_segment(&recording.id)?);
        if recording.failed() {
            computer_use(&client, sandbox_id, Method::DELETE, &delete_path, None).await?;
            continue;
        }
        let size_bytes = recording.size_bytes.unwrap_or(0);
        let body = if size_bytes > MAX_RECORDING_BYTES {
            None
        } else {
            Some(
                client
                    .file_download_stream(sandbox_id, &recording.file_path)
                    .await?,
            )
        };
        let (client, sandbox_id) = (client.clone(), sandbox_id.to_string());
        collected.push(DesktopRecording {
            file_name: recording.stored_name(),
            duration_seconds: recording.duration_seconds,
            size_bytes,
            body,
            remove: async move {
                computer_use(&client, &sandbox_id, Method::DELETE, &delete_path, None)
                    .await
                    .map(|_| ())
            }
            .boxed(),
        });
    }
    Ok(collected)
}

#[cfg(test)]
#[path = "recording_tests.rs"]
mod tests;
