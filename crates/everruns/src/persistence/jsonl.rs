//! Append-only JSONL session store (EVE-836).
//!
//! [`JsonlSessionStore`] implements [`RuntimeMessageStore`] by wrapping an
//! [`InMemoryMessageRetriever`]: reads are served from memory (seeded from the
//! file on [`open`](JsonlSessionStore::open)), and every persisted message is
//! also appended to the file as a versioned JSON record. Because the runtime
//! composes each turn's model input from the message store, seeding the store
//! from a file *is* resuming the conversation.
//!
//! ## On-disk format
//!
//! One JSON object per line, newline-terminated:
//!
//! ```jsonl
//! {"v":1,"seq":0,"session_id":"session_…","message":{…}}
//! ```
//!
//! `message` is the portable [`Message`] record; provider credentials and host
//! service objects never appear in it. A final line without a trailing newline
//! is treated as a torn write from a previous crash: it is dropped (the file is
//! truncated to the last intact record) and the earlier records are kept. A
//! *complete* line that fails to parse is a hard [`JsonlError::Corruption`] with
//! line and last-valid-sequence context — earlier valid records are never
//! silently discarded.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use everruns_core::error::Result as CoreResult;
use everruns_core::in_memory::InMemoryMessageRetriever;
use everruns_core::{
    AgentLoopError, InputMessage, Message, MessageHistory, MessageId, MessageQuery,
    MessageRetriever, SessionId,
};
use everruns_runtime::RuntimeMessageStore;
use serde_json::{Value, json};

/// Current on-disk record schema version.
const FORMAT_VERSION: u64 = 1;

/// Errors from opening or resuming a [`JsonlSessionStore`].
#[derive(Debug)]
#[non_exhaustive]
pub enum JsonlError {
    /// An I/O error reading or writing the backing file.
    Io(std::io::Error),
    /// A complete (newline-terminated) record could not be parsed. Earlier valid
    /// records up to `last_valid_sequence` were loaded; the file is left intact
    /// for inspection.
    Corruption {
        /// 1-based line number of the offending record.
        line: usize,
        /// Sequence number of the last record that parsed cleanly, if any.
        last_valid_sequence: Option<i64>,
        /// Human-readable reason the record was rejected.
        detail: String,
    },
    /// A session id string could not be parsed back into a `SessionId`.
    InvalidSessionId(String),
}

impl std::fmt::Display for JsonlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonlError::Io(e) => write!(f, "jsonl session store I/O error: {e}"),
            JsonlError::Corruption {
                line,
                last_valid_sequence,
                detail,
            } => write!(
                f,
                "corrupt JSONL record at line {line} (last valid sequence: {last_valid_sequence:?}): {detail}"
            ),
            JsonlError::InvalidSessionId(s) => write!(f, "invalid session id {s:?}"),
        }
    }
}

impl std::error::Error for JsonlError {}

impl From<std::io::Error> for JsonlError {
    fn from(e: std::io::Error) -> Self {
        JsonlError::Io(e)
    }
}

/// Guards the append-only writer and the sequence/size state used to detect a
/// concurrent writer.
struct Writer {
    file: std::fs::File,
    /// The sequence number the next appended record will carry.
    next_seq: i64,
    /// File length after our last write; a mismatch on the next append means
    /// another writer appended behind our back.
    persisted_bytes: u64,
}

/// An append-only, file-backed session message store.
///
/// Hand one to [`Agent::session_with_store`](crate::Agent::session_with_store)
/// to persist a new session, or reload one and pass it to
/// [`Agent::resume_session`](crate::Agent::resume_session) to continue an
/// existing conversation in a fresh process. Cheap to share via `Arc`; writes
/// are serialized by an internal lock.
pub struct JsonlSessionStore {
    inner: InMemoryMessageRetriever,
    writer: Mutex<Writer>,
    path: PathBuf,
}

impl JsonlSessionStore {
    /// Open (creating if absent) the JSONL file at `path`, seeding in-memory
    /// history from its records.
    ///
    /// A torn final line (no trailing newline) is dropped and the file is
    /// truncated to the last intact record. A complete but unparseable record
    /// returns [`JsonlError::Corruption`] with line and sequence context.
    pub async fn open(path: impl AsRef<Path>) -> Result<Self, JsonlError> {
        let path = path.as_ref().to_path_buf();
        let inner = InMemoryMessageRetriever::new();

        let mut next_seq: i64 = 0;
        let mut valid_bytes: u64 = 0;
        let mut by_session: HashMap<SessionId, Vec<Message>> = HashMap::new();

        if path.exists() {
            let bytes = std::fs::read(&path)?;
            let text = String::from_utf8(bytes).map_err(|_| JsonlError::Corruption {
                line: 0,
                last_valid_sequence: None,
                detail: "file is not valid UTF-8".to_string(),
            })?;
            let mut last_seq: Option<i64> = None;
            for (idx, piece) in text.split_inclusive('\n').enumerate() {
                let has_newline = piece.ends_with('\n');
                if !has_newline {
                    // Torn final write from a crash: drop it, keep the rest.
                    break;
                }
                let line = piece.strip_suffix('\n').unwrap_or(piece);
                let line = line.strip_suffix('\r').unwrap_or(line);
                if line.trim().is_empty() {
                    valid_bytes += piece.len() as u64;
                    continue;
                }
                match parse_record(line) {
                    Ok(Record {
                        seq,
                        session_id,
                        message,
                    }) => {
                        by_session.entry(session_id).or_default().push(message);
                        last_seq = Some(seq);
                        next_seq = seq + 1;
                        valid_bytes += piece.len() as u64;
                    }
                    Err(detail) => {
                        return Err(JsonlError::Corruption {
                            line: idx + 1,
                            last_valid_sequence: last_seq,
                            detail,
                        });
                    }
                }
            }
        }

        for (session_id, messages) in by_session {
            inner.seed(session_id, messages).await;
        }

        // Drop any torn tail so appends continue from a clean boundary.
        let file_len = if path.exists() {
            std::fs::metadata(&path)?.len()
        } else {
            0
        };
        if valid_bytes < file_len {
            let f = OpenOptions::new().write(true).open(&path)?;
            f.set_len(valid_bytes)?;
        }

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .read(true)
            .open(&path)?;
        let persisted_bytes = file.metadata()?.len();

        Ok(Self {
            inner,
            writer: Mutex::new(Writer {
                file,
                next_seq,
                persisted_bytes,
            }),
            path,
        })
    }

    /// The path of the backing file.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Append one message record, rejecting a concurrent external writer.
    fn append(&self, session_id: SessionId, message: &Message) -> CoreResult<()> {
        let mut writer = self
            .writer
            .lock()
            .map_err(|_| AgentLoopError::store("jsonl session store lock poisoned"))?;

        let on_disk = writer
            .file
            .metadata()
            .map_err(|e| AgentLoopError::store(format!("stat jsonl session file: {e}")))?
            .len();
        if on_disk != writer.persisted_bytes {
            return Err(AgentLoopError::store(format!(
                "jsonl session file changed on disk (expected {} bytes, found {on_disk}); a concurrent writer holds it",
                writer.persisted_bytes
            )));
        }

        let message_json = serde_json::to_value(message)
            .map_err(|e| AgentLoopError::store(format!("serialize message: {e}")))?;
        let record = json!({
            "v": FORMAT_VERSION,
            "seq": writer.next_seq,
            "session_id": session_id.to_string(),
            "message": message_json,
        });
        let line = serde_json::to_string(&record)
            .map_err(|e| AgentLoopError::store(format!("encode record: {e}")))?;

        writeln!(writer.file, "{line}")
            .map_err(|e| AgentLoopError::store(format!("append record: {e}")))?;
        writer
            .file
            .flush()
            .map_err(|e| AgentLoopError::store(format!("flush jsonl session file: {e}")))?;

        writer.next_seq += 1;
        writer.persisted_bytes = writer
            .file
            .metadata()
            .map_err(|e| AgentLoopError::store(format!("stat jsonl session file: {e}")))?
            .len();
        Ok(())
    }
}

/// A decoded on-disk record.
struct Record {
    seq: i64,
    session_id: SessionId,
    message: Message,
}

/// Parse one JSONL line into a [`Record`], returning a human-readable reason on
/// failure (used to build [`JsonlError::Corruption`]).
fn parse_record(line: &str) -> Result<Record, String> {
    let value: Value = serde_json::from_str(line).map_err(|e| format!("invalid JSON: {e}"))?;
    let version = value
        .get("v")
        .and_then(Value::as_u64)
        .ok_or("missing or non-integer `v`")?;
    if version != FORMAT_VERSION {
        return Err(format!(
            "unsupported record version {version} (expected {FORMAT_VERSION})"
        ));
    }
    let seq = value
        .get("seq")
        .and_then(Value::as_i64)
        .ok_or("missing or non-integer `seq`")?;
    let session_id_str = value
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or("missing `session_id`")?;
    let session_id: SessionId = session_id_str
        .parse()
        .map_err(|_| format!("invalid `session_id` {session_id_str:?}"))?;
    let message_value = value.get("message").ok_or("missing `message`")?;
    let message: Message = serde_json::from_value(message_value.clone())
        .map_err(|e| format!("invalid `message`: {e}"))?;
    Ok(Record {
        seq,
        session_id,
        message,
    })
}

// Reads are served from the in-memory retriever seeded on open; every method is
// delegated so filtering behavior matches the default store exactly.
#[async_trait]
impl MessageRetriever for JsonlSessionStore {
    async fn get(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> CoreResult<Option<Message>> {
        self.inner.get(session_id, message_id).await
    }

    async fn load(&self, session_id: SessionId) -> CoreResult<Vec<Message>> {
        self.inner.load(session_id).await
    }

    async fn load_filtered(&self, query: MessageQuery) -> CoreResult<Vec<Message>> {
        self.inner.load_filtered(query).await
    }

    async fn load_filtered_history(&self, query: MessageQuery) -> CoreResult<MessageHistory> {
        self.inner.load_filtered_history(query).await
    }

    async fn load_page(
        &self,
        session_id: SessionId,
        offset: usize,
        limit: usize,
    ) -> CoreResult<Vec<Message>> {
        self.inner.load_page(session_id, offset, limit).await
    }

    async fn count(&self, session_id: SessionId) -> CoreResult<usize> {
        self.inner.count(session_id).await
    }
}

#[async_trait]
impl RuntimeMessageStore for JsonlSessionStore {
    async fn add_input_message(
        &self,
        session_id: SessionId,
        input: InputMessage,
    ) -> CoreResult<Message> {
        let message = self.inner.add(session_id, input).await?;
        self.append(session_id, &message)?;
        Ok(message)
    }

    async fn store_message(&self, session_id: SessionId, message: Message) -> CoreResult<()> {
        self.inner.store(session_id, message.clone()).await?;
        self.append(session_id, &message)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test]
    async fn persists_and_reloads_messages_across_reopen() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let session_id = SessionId::new();

        {
            let store = JsonlSessionStore::open(&path).await.unwrap();
            store
                .add_input_message(session_id, InputMessage::user("first"))
                .await
                .unwrap();
            store
                .add_input_message(session_id, InputMessage::user("second"))
                .await
                .unwrap();
            assert_eq!(store.count(session_id).await.unwrap(), 2);
        }

        // Fresh store over the same file: history is reloaded.
        let reopened = JsonlSessionStore::open(&path).await.unwrap();
        let messages = reopened.load(session_id).await.unwrap();
        assert_eq!(messages.len(), 2);
    }

    #[tokio::test]
    async fn rejects_a_concurrent_writer_with_a_store_error() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let session_id = SessionId::new();

        let a = JsonlSessionStore::open(&path).await.unwrap();
        a.add_input_message(session_id, InputMessage::user("a1"))
            .await
            .unwrap();

        // A second store opens the same file and appends behind A's back.
        let b = JsonlSessionStore::open(&path).await.unwrap();
        b.add_input_message(session_id, InputMessage::user("b1"))
            .await
            .unwrap();

        // A's next append sees the file changed and refuses.
        let err = a
            .add_input_message(session_id, InputMessage::user("a2"))
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("concurrent writer"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn tolerates_a_torn_final_line() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let session_id = SessionId::new();

        {
            let store = JsonlSessionStore::open(&path).await.unwrap();
            store
                .add_input_message(session_id, InputMessage::user("intact"))
                .await
                .unwrap();
        }
        // Simulate a crash mid-write: append a partial line with no newline.
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"{\"v\":1,\"seq\":99,\"session_i").unwrap();
        }

        let reopened = JsonlSessionStore::open(&path).await.unwrap();
        // The torn line is dropped; the intact record survives.
        assert_eq!(reopened.load(session_id).await.unwrap().len(), 1);
        // And appends resume cleanly on the truncated file.
        reopened
            .add_input_message(session_id, InputMessage::user("after recovery"))
            .await
            .unwrap();
        let again = JsonlSessionStore::open(&path).await.unwrap();
        assert_eq!(again.load(session_id).await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn resume_in_a_fresh_store_keeps_history_and_persists_more() {
        use crate::{Agent, Model};
        use std::sync::Arc;

        let dir = tempdir().unwrap();
        let path = dir.path().join("conversation.jsonl");
        let agent = Agent::builder()
            .instructions("Be brief.")
            .model(Model::simulated("ok"))
            .build()
            .unwrap();

        // First "process": two turns on a persisted session, then drop it.
        let session_id;
        {
            let store = Arc::new(JsonlSessionStore::open(&path).await.unwrap());
            let mut session = agent.session_with_store(store);
            session_id = session.id();
            session.run("first").await.unwrap();
            session.run("second").await.unwrap();
        }

        // A fresh store over the same file reloads the history.
        let reopened = Arc::new(JsonlSessionStore::open(&path).await.unwrap());
        let sid: SessionId = session_id.parse().unwrap();
        let before = reopened.load(sid).await.unwrap().len();
        assert!(before >= 2, "expected persisted history, got {before}");

        // Second "process": resume by id and run a third turn.
        let mut resumed = agent.resume_session(reopened.clone(), &session_id).unwrap();
        let turn = resumed.run("third").await.unwrap();
        assert!(turn.success);

        let after = reopened.load(sid).await.unwrap().len();
        assert!(
            after > before,
            "the third turn should persist more messages ({after} !> {before})"
        );

        // Provider credentials never reach disk.
        let contents = std::fs::read_to_string(&path).unwrap();
        assert!(
            !contents.contains("fake-key"),
            "the configured api key leaked into the persisted file"
        );
    }

    #[tokio::test]
    async fn reports_corruption_with_line_and_sequence_context() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("session.jsonl");
        let session_id = SessionId::new();

        {
            let store = JsonlSessionStore::open(&path).await.unwrap();
            store
                .add_input_message(session_id, InputMessage::user("ok"))
                .await
                .unwrap();
        }
        // A complete (newline-terminated) but unparseable record after a valid one.
        {
            let mut f = OpenOptions::new().append(true).open(&path).unwrap();
            f.write_all(b"not json at all\n").unwrap();
        }

        match JsonlSessionStore::open(&path).await {
            Err(JsonlError::Corruption {
                line,
                last_valid_sequence,
                ..
            }) => {
                assert_eq!(line, 2);
                assert_eq!(last_valid_sequence, Some(0));
            }
            Err(other) => panic!("expected a corruption error, got: {other}"),
            Ok(_) => panic!("expected a corruption error, got Ok"),
        }
    }
}
