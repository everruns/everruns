// Per-session JSONL event log.
//
// Each session writes its replay-relevant events to
// `<storage_dir>/<session_id>.jsonl`, one serialized `Event` per line.
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
use everruns_core::message::{ContentPart, Message};
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

/// Default location for ercode's per-session JSONL files. Resolves via
/// `dirs::data_dir()`, which is the platform-native user data directory
/// (`~/.local/share/ercode/sessions/` on Linux,
/// `~/Library/Application Support/ercode/sessions/` on macOS,
/// `%APPDATA%\ercode\sessions\` on Windows).
///
/// Returns an error when the platform data dir can't be resolved — we
/// intentionally do NOT fall back to the current working directory,
/// because the cwd for ercode is usually the user's workspace and we
/// don't want sensitive session logs landing in the repo.
pub fn default_storage_dir() -> Result<PathBuf> {
    dirs::data_dir()
        .map(|p| p.join("ercode").join("sessions"))
        .ok_or_else(|| {
            AgentLoopError::config(
                "could not resolve a platform data directory for session logs; \
                 pass --session-dir <PATH> explicitly",
            )
        })
}

pub fn session_log_path(storage_dir: &Path, session_id: SessionId) -> PathBuf {
    storage_dir.join(format!("{session_id}.jsonl"))
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
            let line = serde_json::to_string(&event).map_err(|e| {
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
            return serde_json::Value::String(text.text.clone());
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
