// Per-session JSONL event log.
//
// Each session writes its replay-relevant events to
// `<sessions_dir>/<session_id>/events.jsonl`, one serialized `Event` per line.
// `--session <id>` reuses the same SessionId on the next run and replays
// the file: events are read back and messages derived from those events
// are seeded into the message store so the conversation history shows up
// in the next turn's context.
//
// JSONL is chosen because `Event` is already `Serialize + Deserialize`
// and the format degrades gracefully — a half-written line at the end is
// a parse error on one line, not a corrupt file. The writer flushes after
// every line so a crash mid-session loses at most the event in flight.
//
// Concerns explicitly handled below:
// * 0o600 file mode on Unix (owner-only): session logs contain prompts,
//   tool arguments, and tool output.
// * Replay only keeps event types we can reconstruct into messages
//   (`input.message`, `output.message.completed`, `tool.completed`).
//   Streaming `*.delta` events have no replay value and would otherwise
//   inflate the log O(n²) for long streamed responses.
// * Assistant `thinking` / `thinking_signature` fields are stripped before
//   JSONL persistence. The CLI keeps the live runtime event unchanged, but
//   session logs are user-facing durable history and must not expose hidden
//   model reasoning material.
// * Replay rejects events whose `session_id` doesn't match the resumed
//   session — guards against accidentally merging logs across sessions.
// * On open, if the file does not end with `\n` (previous run crashed
//   mid-write), append one before any new line is added — prevents the
//   first new event from being concatenated onto a partial tail.
// * Sequence numbers continue from the replayed maximum rather than
//   restarting from 0, so `Event.sequence` stays monotonic within a
//   session across resumes.
//
// Concurrency:
// * Intra-process: the file handle sits behind a `tokio::Mutex` so emits
//   serialize even when tools fire events from many tasks.
// * Inter-process: an advisory exclusive flock (`File::try_lock`) is
//   acquired on open. A second `ercode --session <same-id>` against the
//   same JSONL file fails fast with a clear error instead of silently
//   interleaving appends.

use async_trait::async_trait;
use chrono::Utc;
use everruns_core::error::{AgentLoopError, Result};
use everruns_core::events::{
    Event, EventData, EventRequest, INPUT_MESSAGE, OUTPUT_MESSAGE_COMPLETED,
    OutputMessageCompletedData, TOOL_COMPLETED,
};
use everruns_core::message::{ContentPart, Message, MessageRole};
use everruns_core::tools::ToolResultImage;
use everruns_core::traits::EventEmitter;
use everruns_core::typed_id::EventId;
use everruns_core::typed_id::SessionId;
use everruns_runtime::EventBus;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufRead, BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// Default location for ercode's per-session storage folders. Resolves via
/// `dirs::data_dir()`, which is the platform-native user data directory
/// (`~/.local/share/ercode/sessions/` on Linux,
/// `~/Library/Application Support/ercode/sessions/` on macOS,
/// `%APPDATA%\ercode\sessions\` on Windows).
///
/// Returns an error when the platform data dir can't be resolved — we
/// intentionally do NOT fall back to the current working directory,
/// because the cwd for ercode is usually the user's workspace and we
/// don't want sensitive session logs landing in the repo.
pub fn default_sessions_dir() -> Result<PathBuf> {
    dirs::data_dir()
        .map(|p| p.join("ercode").join("sessions"))
        .ok_or_else(|| {
            AgentLoopError::config(
                "could not resolve a platform data directory for session logs; \
                 pass --session-dir <PATH> explicitly",
            )
        })
}

pub fn session_dir_path(sessions_dir: &Path, session_id: SessionId) -> PathBuf {
    sessions_dir.join(session_id.to_string())
}

pub fn session_log_path(session_dir: &Path) -> PathBuf {
    session_dir.join("events.jsonl")
}

/// Result of replaying a JSONL session log from disk.
#[derive(Debug, Default)]
pub struct ReplayedSession {
    pub events: Vec<Event>,
    pub messages: Vec<Message>,
    /// Highest `Event.sequence` value found in the file (None if no
    /// events had sequence numbers). The new emitter resumes from
    /// `max_sequence + 1`.
    pub max_sequence: Option<i32>,
}

/// Read a session log into memory. Missing files return an empty replay.
/// Malformed lines and events for a different session are skipped with
/// a tracing warning; a half-written tail line shouldn't take down the
/// next session.
pub fn replay(path: &Path, expected: SessionId) -> Result<ReplayedSession> {
    if !path.exists() {
        return Ok(ReplayedSession::default());
    }
    let file = File::open(path)
        .map_err(|e| AgentLoopError::config(format!("open session log {}: {e}", path.display())))?;
    let mut out = ReplayedSession::default();
    for (i, line) in BufReader::new(file).lines().enumerate() {
        let Ok(line) = line else {
            tracing::warn!(
                line = i + 1,
                "session log read error; stopping replay early"
            );
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Event>(&line) {
            Ok(event) => {
                if event.session_id != expected {
                    tracing::warn!(
                        line = i + 1,
                        found = %event.session_id,
                        expected = %expected,
                        "session log line belongs to a different session; skipping"
                    );
                    continue;
                }
                if let Some(seq) = event.sequence {
                    out.max_sequence = Some(out.max_sequence.map_or(seq, |m| m.max(seq)));
                }
                if let Some(message) = message_from_event(&event.data) {
                    out.messages.push(message);
                }
                out.events.push(event);
            }
            Err(e) => {
                tracing::warn!(line = i + 1, error = %e, "skipping malformed session-log line");
            }
        }
    }
    Ok(out)
}

/// Event types that are useful to keep on disk: they're the ones replay
/// can map back into the conversation. Everything else (streaming
/// `*.delta`, `*.started`, lifecycle, etc.) adds bytes without adding
/// resume value, and the delta types in particular bloat the log
/// O(n²) since each delta carries the accumulated text so far.
fn is_replay_relevant(event_type: &str) -> bool {
    matches!(
        event_type,
        INPUT_MESSAGE | OUTPUT_MESSAGE_COMPLETED | TOOL_COMPLETED
    )
}

/// EventEmitter that appends replay-relevant events as JSONL to a file
/// and mirrors all events into an in-memory vec for `events()` queries.
///
/// Owns its own sequence counter so resumed sessions continue past the
/// max sequence found in the replayed log (rather than restarting at 1
/// and producing non-monotonic per-session sequences).
pub struct JsonlEventEmitter {
    events: Arc<RwLock<Vec<Event>>>,
    sequence: Arc<RwLock<i32>>,
    file: Arc<Mutex<File>>,
}

impl JsonlEventEmitter {
    /// Open (or create) the session-log file and prepare the fan-out.
    /// `start_sequence` is the value `Event.sequence` should take on
    /// the next emitted event (1 for a fresh session, max_replayed + 1
    /// for a resume).
    pub fn open(path: &Path, start_sequence: i32) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                AgentLoopError::config(format!("create session log dir {}: {e}", parent.display()))
            })?;
        }
        let mut opts = OpenOptions::new();
        // `read(true)` is required so we can read the file's last byte
        // for the half-written tail repair below; `append(true)` keeps
        // every write at end-of-file even with concurrent appends.
        opts.create(true).append(true).read(true);
        #[cfg(unix)]
        {
            // Owner-only: session logs contain prompts and tool output
            // we don't want world-readable. `mode()` only applies on
            // create; existing files keep their mode.
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut file = opts.open(path).map_err(|e| {
            AgentLoopError::config(format!("open session log {}: {e}", path.display()))
        })?;

        // Advisory exclusive flock: prevents two `ercode --session <id>`
        // processes from interleaving writes on the same JSONL file.
        // Advisory only — another process that doesn't lock can still
        // write — but every JSONL writer here goes through this path.
        // The lock is released when the underlying `File` is dropped
        // (i.e. when the emitter is dropped at session end).
        match file.try_lock() {
            Ok(()) => {}
            Err(TryLockError::WouldBlock) => {
                return Err(AgentLoopError::config(format!(
                    "another ercode process is already writing {}; \
                     refusing to share a session log",
                    path.display()
                )));
            }
            Err(TryLockError::Error(e)) => {
                return Err(AgentLoopError::config(format!(
                    "lock session log {}: {e}",
                    path.display()
                )));
            }
        }

        // Repair half-written tail: if the file is non-empty and does
        // NOT end with '\n', the previous process crashed after writing
        // a partial JSON object. A naive append would concatenate the
        // new line onto that partial tail and produce a corrupt entry.
        // Add a leading '\n' so the partial tail becomes its own
        // (malformed, skipped) line and our new line stays clean.
        let len = file
            .seek(SeekFrom::End(0))
            .map_err(|e| AgentLoopError::config(format!("stat session log: {e}")))?;
        if len > 0 {
            let mut last = [0u8; 1];
            file.seek(SeekFrom::Start(len - 1))
                .and_then(|_| std::io::Read::read_exact(&mut file, &mut last))
                .map_err(|e| AgentLoopError::config(format!("read session log tail: {e}")))?;
            if last[0] != b'\n' {
                tracing::warn!(
                    "session log {} ends without newline; repairing tail before append",
                    path.display()
                );
                writeln!(file)
                    .map_err(|e| AgentLoopError::config(format!("repair session log tail: {e}")))?;
            }
        }

        Ok(Self {
            events: Arc::new(RwLock::new(Vec::new())),
            sequence: Arc::new(RwLock::new(start_sequence.saturating_sub(1))),
            file: Arc::new(Mutex::new(file)),
        })
    }

    /// Push events read back from disk into the in-memory vec that
    /// `EventBus::collected_events()` returns. Does NOT re-write them to
    /// the JSONL file (they're already there) — purely a seeding step so
    /// `runtime.events()` after resume returns the full session history
    /// instead of starting empty.
    pub async fn seed_replayed(&self, events: Vec<Event>) {
        if events.is_empty() {
            return;
        }
        self.events.write().await.extend(events);
    }
}

#[async_trait]
impl EventEmitter for JsonlEventEmitter {
    async fn emit(&self, request: EventRequest) -> Result<Event> {
        // Assign id + sequence ourselves so we own the monotonic
        // sequence even across resumes.
        let mut seq = self.sequence.write().await;
        *seq = seq.saturating_add(1);
        let seq_val = *seq;
        drop(seq);

        let mut event = request.into_event(EventId::new(), seq_val);
        // `into_event` may have set its own timestamp; ensure we always
        // record one for replay ordering.
        if event.ts.timestamp() == 0 {
            event.ts = Utc::now();
        }
        self.events.write().await.push(event.clone());

        if is_replay_relevant(&event.event_type) {
            let persisted_event = event_for_session_log(&event);
            let line = serde_json::to_string(&persisted_event).map_err(|e| {
                AgentLoopError::config(format!("serialize event for session log: {e}"))
            })?;
            let mut file = self.file.lock().await;
            writeln!(file, "{line}")
                .map_err(|e| AgentLoopError::config(format!("write session log line: {e}")))?;
            file.flush()
                .map_err(|e| AgentLoopError::config(format!("flush session log: {e}")))?;
        }
        Ok(event)
    }
}

fn event_for_session_log(event: &Event) -> Event {
    let mut persisted = event.clone();
    if let EventData::OutputMessageCompleted(data) = &mut persisted.data {
        strip_hidden_thinking(&mut data.message);
    }
    persisted
}

fn strip_hidden_thinking(message: &mut Message) {
    if message.role == MessageRole::Agent {
        message.thinking = None;
        message.thinking_signature = None;
    }
}

// `EventBus: EventEmitter` adds the `collected_events()` query so the
// runtime can ask "what events fired this session?" through the same
// trait object that handles emissions.
#[async_trait]
impl EventBus for JsonlEventEmitter {
    async fn collected_events(&self) -> Vec<Event> {
        self.events.read().await.clone()
    }
}

// ---------- event → message mapping ----------
//
// `crates/runtime/src/runtime.rs` has the same logic in a private fn;
// copied here for the replay path. The three event types matched are
// exactly those `is_replay_relevant` lets through to disk.

fn message_from_event(data: &EventData) -> Option<Message> {
    match data {
        EventData::InputMessage(d) => Some(d.message.clone()),
        EventData::OutputMessageCompleted(OutputMessageCompletedData { message, .. }) => {
            Some(message.clone())
        }
        EventData::ToolCompleted(d) => Some(tool_completed_to_message(d.clone())),
        _ => None,
    }
}

fn tool_completed_to_message(data: everruns_core::events::ToolCompletedData) -> Message {
    let mut images: Vec<ToolResultImage> = Vec::new();
    let result = data.result.map(|parts| {
        for part in &parts {
            if let ContentPart::Image(img) = part
                && let (Some(base64), Some(media_type)) = (&img.base64, &img.media_type)
            {
                images.push(ToolResultImage {
                    base64: base64.clone(),
                    media_type: media_type.clone(),
                });
            }
        }

        let text_parts: Vec<&ContentPart> = parts
            .iter()
            .filter(|part| matches!(part, ContentPart::Text(_)))
            .collect();
        if text_parts.len() == 1
            && let ContentPart::Text(text) = text_parts[0]
        {
            return parse_structured_tool_result_text(&text.text);
        }
        if !text_parts.is_empty() {
            serde_json::to_value(&text_parts).unwrap_or_default()
        } else {
            serde_json::Value::Null
        }
    });

    if images.is_empty() {
        Message::tool_result(&data.tool_call_id, result, data.error)
    } else {
        Message::tool_result_with_images(&data.tool_call_id, result, images)
    }
}

fn parse_structured_tool_result_text(text: &str) -> serde_json::Value {
    let trimmed = text.trim_start();
    if !trimmed.starts_with('{') && !trimmed.starts_with('[') {
        return serde_json::Value::String(text.to_string());
    }

    match serde_json::from_str(text) {
        Ok(value @ (serde_json::Value::Object(_) | serde_json::Value::Array(_))) => value,
        _ => serde_json::Value::String(text.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::events::{
        EventContext, InputMessageData, OutputMessageCompletedData, ToolCompletedData,
    };
    use everruns_core::message::Message;

    fn input_event(session_id: SessionId, text: &str) -> Event {
        Event::new(
            session_id,
            EventContext::default(),
            InputMessageData::new(Message::user(text)),
        )
    }

    #[tokio::test]
    async fn seed_replayed_populates_collected_events_without_rewrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session_id = SessionId::from_seed(482);
        let session_dir = session_dir_path(dir.path(), session_id);
        let path = session_log_path(&session_dir);

        let emitter = JsonlEventEmitter::open(&path, 1).expect("open");
        let prior = vec![
            input_event(session_id, "first prior turn"),
            input_event(session_id, "second prior turn"),
        ];
        emitter.seed_replayed(prior.clone()).await;

        // Acceptance: collected_events() returns the seeded events.
        let collected = emitter.collected_events().await;
        assert_eq!(collected.len(), prior.len());
        assert_eq!(collected[0].id, prior[0].id);
        assert_eq!(collected[1].id, prior[1].id);

        // Acceptance: seeding must NOT re-write to the JSONL file.
        // The file was opened fresh and nothing was emitted; it should
        // still be empty on disk.
        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert!(
            on_disk.is_empty(),
            "seed_replayed must not re-persist; found: {on_disk:?}"
        );
    }

    #[tokio::test]
    async fn seed_replayed_then_emit_keeps_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session_id = SessionId::from_seed(4820);
        let session_dir = session_dir_path(dir.path(), session_id);
        let path = session_log_path(&session_dir);

        let emitter = JsonlEventEmitter::open(&path, 3).expect("open");
        let prior = vec![input_event(session_id, "a"), input_event(session_id, "b")];
        emitter.seed_replayed(prior.clone()).await;

        let req = EventRequest::new(
            session_id,
            EventContext::default(),
            InputMessageData::new(Message::user("new")),
        );
        let _new = emitter.emit(req).await.expect("emit");

        let collected = emitter.collected_events().await;
        assert_eq!(collected.len(), 3, "seeded + 1 new");
        // New event sequence continues from start_sequence we opened with.
        assert_eq!(collected[2].sequence, Some(3));
    }

    #[tokio::test]
    async fn output_message_thinking_is_not_written_to_session_log() {
        let dir = tempfile::tempdir().expect("tempdir");
        let session_id = SessionId::from_seed(4821);
        let session_dir = session_dir_path(dir.path(), session_id);
        let path = session_log_path(&session_dir);
        let emitter = JsonlEventEmitter::open(&path, 1).expect("open");

        let mut message = Message::assistant("I will inspect the files.");
        message.thinking = Some("private model reasoning".to_string());
        message.thinking_signature = Some("encrypted-thinking-token".to_string());
        let req = EventRequest::new(
            session_id,
            EventContext::default(),
            OutputMessageCompletedData::new(message),
        );

        emitter.emit(req).await.expect("emit");

        let on_disk = std::fs::read_to_string(&path).expect("read");
        assert!(
            !on_disk.contains("private model reasoning"),
            "session log must not persist assistant thinking: {on_disk}"
        );
        assert!(
            !on_disk.contains("thinking_signature"),
            "session log must not persist thinking signatures: {on_disk}"
        );
        let replayed = replay(&path, session_id).expect("replay");
        let replayed_message = replayed.messages.first().expect("message replayed");
        assert!(replayed_message.thinking.is_none());
        assert!(replayed_message.thinking_signature.is_none());

        let collected = emitter.collected_events().await;
        match &collected[0].data {
            EventData::OutputMessageCompleted(data) => {
                assert_eq!(
                    data.message.thinking.as_deref(),
                    Some("private model reasoning")
                );
            }
            other => panic!("unexpected event data: {other:?}"),
        }
    }

    #[test]
    fn tool_completed_replay_preserves_json_result_shape() {
        let data = ToolCompletedData::success(
            "call_read".to_string(),
            "read_file".to_string(),
            vec![ContentPart::text(
                serde_json::json!({
                    "path": "/workspace/src/lib.rs",
                    "content": "1|fn main() {}"
                })
                .to_string(),
            )],
            Some(1),
        );

        let message = tool_completed_to_message(data);
        let result = message
            .tool_result_content()
            .and_then(|content| content.result.as_ref())
            .expect("tool result should be present");

        assert_eq!(result["path"], "/workspace/src/lib.rs");
        assert_eq!(result["content"], "1|fn main() {}");
    }

    #[test]
    fn tool_completed_replay_keeps_scalar_json_as_text() {
        let data = ToolCompletedData::success(
            "call_scalar".to_string(),
            "custom_tool".to_string(),
            vec![ContentPart::text("123")],
            Some(1),
        );

        let message = tool_completed_to_message(data);
        let result = message
            .tool_result_content()
            .and_then(|content| content.result.as_ref())
            .expect("tool result should be present");

        assert_eq!(result, &serde_json::Value::String("123".to_string()));
    }
}
