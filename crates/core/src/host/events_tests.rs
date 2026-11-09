use super::*;
use crate::event_emitter::EventEmitter as _;
use crate::events::{EventContext, InputMessageData, OutputMessageDeltaData, SessionStartedData};
use everruns_contracts::typed_id::{HarnessId, TurnId};

#[cfg(unix)]
#[tokio::test]
async fn jsonl_open_rejects_symlinks_without_touching_the_target() {
    use std::os::unix::fs::symlink;

    let root = tempfile::tempdir().expect("tempdir");
    let target = root.path().join("target");
    let path = root.path().join("events.jsonl");
    tokio::fs::write(&target, b"unchanged")
        .await
        .expect("write target");
    symlink(&target, &path).expect("create symlink");

    let error = JsonlEventLog::open(&path)
        .await
        .err()
        .expect("symlink is rejected");

    assert!(matches!(error, EventLogError::Backend { .. }));
    assert_eq!(tokio::fs::read(&target).await.unwrap(), b"unchanged");
}

#[cfg(unix)]
#[tokio::test]
async fn jsonl_open_hardens_existing_file_permissions() {
    use std::os::unix::fs::PermissionsExt;

    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("events.jsonl");
    tokio::fs::write(&path, b"").await.expect("write fixture");
    tokio::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
        .await
        .expect("set fixture permissions");

    let _log = JsonlEventLog::open(&path).await.expect("open event log");

    let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
}

#[tokio::test]
async fn jsonl_recovery_rejects_oversize_files_before_indexing() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("events.jsonl");
    tokio::fs::write(&path, b"123456789")
        .await
        .expect("write fixture");

    let error = JsonlEventLog::open_with_limits(&path, 8, 100)
        .await
        .err()
        .expect("oversize log is rejected");
    assert!(matches!(error, EventLogError::RecoveryLimitExceeded { .. }));
}

#[tokio::test]
async fn jsonl_recovery_rejects_excessive_event_counts() {
    let root = tempfile::tempdir().expect("tempdir");
    let path = root.path().join("events.jsonl");
    let session_id = SessionId::new();
    let first = EventRequest::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(RuntimeMessage::user("one")),
    )
    .into_event(EventId::new(), 1);
    let second = EventRequest::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(RuntimeMessage::user("two")),
    )
    .into_event(EventId::new(), 2);
    let bytes = format!(
        "{}\n{}\n",
        serde_json::to_string(&first).unwrap(),
        serde_json::to_string(&second).unwrap()
    );
    tokio::fs::write(&path, bytes).await.expect("write fixture");

    let error = JsonlEventLog::open_with_limits(&path, 1_000_000, 1)
        .await
        .err()
        .expect("event-heavy log is rejected");
    assert!(matches!(error, EventLogError::RecoveryLimitExceeded { .. }));
}

struct LifecycleHeavyReader;

#[async_trait]
impl EventReader for LifecycleHeavyReader {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        let after = request
            .cursor
            .as_ref()
            .map_or(0, EventCursor::after_sequence);
        let high_watermark = (MAX_EVENT_HISTORY_REPLAY + 1) as i32;
        let end = after
            .saturating_add(request.limit.get() as i32)
            .min(high_watermark);
        let events = ((after + 1)..=end)
            .map(|sequence| {
                EventRequest::new(
                    request.session_id,
                    EventContext::empty(),
                    OutputMessageDeltaData {
                        turn_id: TurnId::new(),
                        message_id: MessageId::new(),
                        delta: String::new(),
                        accumulated: String::new(),
                        phase: None,
                    },
                )
                .into_event(EventId::new(), sequence)
            })
            .collect();
        let next_cursor = (end < high_watermark)
            .then(|| EventCursor::continuation(request.session_id, end, high_watermark))
            .transpose()?;
        EventPage::new(events, next_cursor, high_watermark)
    }
}

#[tokio::test]
async fn full_projection_caps_examined_lifecycle_envelopes() {
    let history = EventHistory::new(Arc::new(LifecycleHeavyReader));
    let error = match history.project(SessionId::new()).await {
        Ok(_) => panic!("lifecycle-heavy replay must be bounded"),
        Err(error) => error,
    };
    assert!(matches!(error, EventLogError::InvalidRead { .. }));
    assert!(error.to_string().contains("examined more than"));
}

#[tokio::test]
async fn a_suffix_projection_reads_only_the_suffix() {
    // The full replay is over the bound; the suffix after a checkpoint is not.
    let history = EventHistory::new(Arc::new(LifecycleHeavyReader));
    let after = (MAX_EVENT_HISTORY_REPLAY - 10) as i64;
    let projected = history
        .project_after(SessionId::new(), Some(after))
        .await
        .expect("a short suffix stays within the replay bound");
    assert!(projected.is_empty());
}

const DELTAS: &[&str] = &[crate::events::OUTPUT_MESSAGE_DELTA];

#[tokio::test]
async fn a_typed_read_skips_other_events_and_resumes_after_its_last() {
    // A projection asks the in-memory log for message events only, so a
    // turn's `llm.generation` (the whole prompt) is never copied for it.
    let session_id = SessionId::new();
    let log = InMemoryEventLog::new();
    for n in 0..5 {
        log.append(EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(RuntimeMessage::user(format!("q{n}"))),
        ))
        .await
        .unwrap();
        log.append(EventRequest::new(
            session_id,
            EventContext::empty(),
            SessionStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                model_id: None,
            },
        ))
        .await
        .unwrap();
    }
    let request = EventReadRequest::new(session_id, EventReadLimit::new(2).unwrap())
        .with_types(&[crate::events::INPUT_MESSAGE]);
    let first = log.read_page(request).await.unwrap();
    let sequences =
        |page: &EventPage| -> Vec<i32> { page.events.iter().filter_map(|e| e.sequence).collect() };
    assert_eq!(sequences(&first), vec![1, 3]);
    let rest = log
        .read_page(
            EventReadRequest::from_cursor(
                first.next_cursor.clone().unwrap(),
                EventReadLimit::default(),
            )
            .with_types(&[crate::events::INPUT_MESSAGE]),
        )
        .await
        .unwrap();
    assert_eq!(sequences(&rest), vec![5, 7, 9]);
    assert!(rest.next_cursor.is_none());
    // A rare type costs pages, not a scan of the whole session: a page
    // examines a bounded run of other events, then continues after them.
    let long = SessionId::new();
    log.append(EventRequest::new(
        long,
        EventContext::empty(),
        InputMessageData::new(RuntimeMessage::user("first")),
    ))
    .await
    .unwrap();
    for _ in 0..(MAX_EVENT_PAGE_SIZE + 10) {
        log.append(EventRequest::new(
            long,
            EventContext::empty(),
            SessionStartedData {
                harness_id: HarnessId::new(),
                agent_id: None,
                model_id: None,
            },
        ))
        .await
        .unwrap();
    }
    let first = log
        .read_page(
            EventReadRequest::new(long, EventReadLimit::new(2).unwrap())
                .with_types(&[crate::events::INPUT_MESSAGE]),
        )
        .await
        .unwrap();
    assert_eq!(sequences(&first), vec![1]);
    let cursor = first.next_cursor.clone().expect("the scan stopped early");
    assert_eq!(cursor.after_sequence(), 1 + MAX_EVENT_PAGE_SIZE as i32);
    let rest = log
        .read_page(
            EventReadRequest::from_cursor(cursor, EventReadLimit::new(2).unwrap())
                .with_types(&[crate::events::INPUT_MESSAGE]),
        )
        .await
        .unwrap();
    assert!(rest.events.is_empty() && rest.next_cursor.is_none());
    // A read after a cursor in the middle starts there.
    let after = log
        .read_page(EventReadRequest::from_cursor(
            EventCursor::after(session_id, 6).unwrap(),
            EventReadLimit::default(),
        ))
        .await
        .unwrap();
    assert_eq!(sequences(&after), vec![7, 8, 9, 10]);
}

#[tokio::test]
async fn a_tail_read_reads_only_the_tail() {
    // The whole log is over the replay bound; its last window is not.
    let high = (MAX_EVENT_HISTORY_REPLAY + 1) as i32;
    let tail = read_tail_until(&LifecycleHeavyReader, SessionId::new(), DELTAS, |events| {
        events
            .first()
            .and_then(|e| e.sequence)
            .is_some_and(|s| s < high - 300)
    })
    .await
    .expect("a tail read stops once it has enough");
    let sequences: Vec<i32> = tail.iter().filter_map(|e| e.sequence).collect();
    assert_eq!(sequences.last(), Some(&high));
    assert!(
        sequences.windows(2).all(|w| w[1] == w[0] + 1),
        "in order, no gaps"
    );
    assert_eq!(sequences.len(), 256 + 512, "two windows");
    // Never enough: the bound still holds.
    let error = read_tail_until(&LifecycleHeavyReader, SessionId::new(), DELTAS, |_| false)
        .await
        .expect_err("an unbounded tail read is refused");
    assert!(matches!(error, EventLogError::InvalidRead { .. }));
}

#[tokio::test]
async fn history_after_a_sequence_keeps_its_source_sequence() {
    let session_id = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    for text in ["one", "two", "three"] {
        log.append(EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(RuntimeMessage::user(text)),
        ))
        .await
        .expect("append input message");
    }
    let history = EventHistory::new(log);
    let suffix = history
        .load_filtered_history(MessageQuery::new(session_id).after_sequence(1))
        .await
        .expect("read suffix");
    assert_eq!(suffix.messages.len(), 2);
    assert_eq!(suffix.source_sequence, Some(3));
    // Nothing after the boundary: the newest message still names the source.
    let empty = history
        .load_filtered_history(MessageQuery::new(session_id).after_sequence(3))
        .await
        .expect("read empty suffix");
    assert!(empty.messages.is_empty());
    assert_eq!(empty.source_sequence, Some(3));
    // A boundary past the log's end (a checkpoint claiming more history than
    // the log holds) reads as nothing after it, not as an error.
    let beyond = history
        .load_filtered_history(MessageQuery::new(session_id).after_sequence(10_000))
        .await
        .expect("read past the end");
    assert!(beyond.messages.is_empty());
    assert_eq!(beyond.source_sequence, Some(3));
}

#[tokio::test]
async fn exact_message_boundary_ignores_trailing_lifecycle_events() {
    let session_id = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    log.append(EventRequest::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(RuntimeMessage::user("hello")),
    ))
    .await
    .expect("append input message");
    log.append(EventRequest::new(
        session_id,
        EventContext::empty(),
        OutputMessageCompletedData::new(RuntimeMessage::assistant("hi")),
    ))
    .await
    .expect("append output message");
    log.append(EventRequest::new(
        session_id,
        EventContext::empty(),
        SessionStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            model_id: None,
        },
    ))
    .await
    .expect("append lifecycle event");

    let page = EventHistory::new(log)
        .read_page(EventHistoryReadRequest::new(
            session_id,
            EventHistoryReadLimit::new(2).expect("valid history limit"),
        ))
        .await
        .expect("read history");

    assert_eq!(page.messages.len(), 2);
    assert!(page.next_cursor.is_none());
}

#[test]
fn tool_completion_projection_preserves_structured_result_and_fingerprints() {
    let event = EventRequest::new(
        SessionId::new(),
        EventContext::empty(),
        crate::events::ToolCompletedData::success(
            "call_read".into(),
            "read_file".into(),
            vec![ContentPart::text(
                serde_json::json!({
                    "path": "/workspace/src/lib.rs",
                    "content": "1|fn main() {}"
                })
                .to_string(),
            )],
            Some(1),
        )
        .with_fingerprints("sha256:call".into(), "sha256:result".into()),
    )
    .into_event(EventId::new(), 1);
    let message = message_from_event(&event).expect("tool result message");
    let result = message
        .tool_result_content()
        .and_then(|content| content.result.as_ref())
        .expect("projected result");
    assert_eq!(result["path"], "/workspace/src/lib.rs");
    let metadata = message.metadata.expect("tool metadata");
    assert_eq!(metadata["tool_name"], "read_file");
    assert_eq!(metadata["tool_call_fingerprint"], "sha256:call");
    assert_eq!(metadata["tool_result_fingerprint"], "sha256:result");
}

#[test]
fn tool_completion_projection_keeps_scalar_json_as_text() {
    let event = EventRequest::new(
        SessionId::new(),
        EventContext::empty(),
        crate::events::ToolCompletedData::success(
            "call_scalar".into(),
            "custom_tool".into(),
            vec![ContentPart::text("123")],
            Some(1),
        ),
    )
    .into_event(EventId::new(), 1);
    let message = message_from_event(&event).expect("tool result message");
    let result = message
        .tool_result_content()
        .and_then(|content| content.result.as_ref())
        .expect("projected result");
    assert_eq!(result, &serde_json::Value::String("123".into()));
}

/// Records what the live sink saw, in order.
#[derive(Default)]
struct SeenSink(std::sync::Mutex<Vec<Event>>);

impl EventSink for SeenSink {
    fn try_send(&self, event: Event) -> Result<(), EventSinkError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

impl SeenSink {
    fn types(&self) -> Vec<String> {
        let seen = self.0.lock().unwrap();
        seen.iter().map(|event| event.event_type.clone()).collect()
    }
}

async fn committed(log: &InMemoryEventLog, session_id: SessionId) -> usize {
    log.read_page(EventReadRequest::new(
        session_id,
        EventReadLimit::new(100).unwrap(),
    ))
    .await
    .unwrap()
    .events
    .len()
}

fn held_lifecycle(session_id: SessionId) -> EventRequest {
    EventRequest::new(
        session_id,
        EventContext::empty(),
        crate::events::SessionActivatedData {
            turn_id: TurnId::new(),
            input_message_id: MessageId::new(),
        },
    )
}

#[tokio::test]
async fn committed_batches_hold_lifecycle_events_until_the_next_barrier() {
    let session_id = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let sink = Arc::new(SeenSink::default());
    let emitter = HostEventEmitter::new(log.clone(), sink.clone());
    emitter.enable_committed_batches();

    let held = emitter.emit(held_lifecycle(session_id)).await.unwrap();
    assert_eq!(held.sequence, None, "a held event is provisional");
    assert!(
        sink.types().is_empty(),
        "no listener sees an uncommitted event"
    );
    assert_eq!(committed(&log, session_id).await, 0);

    let barrier = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(RuntimeMessage::user("hi")),
        ))
        .await
        .unwrap();
    assert_eq!(
        barrier.sequence,
        Some(2),
        "the barrier commits after what it carries"
    );
    assert_eq!(
        sink.types(),
        ["session.activated", "input.message"],
        "the sink sees committed events in emission order"
    );

    emitter.emit(held_lifecycle(session_id)).await.unwrap();
    emitter.flush().await.unwrap();
    assert_eq!(committed(&log, session_id).await, 3);
    assert_eq!(sink.types().len(), 3);
}

#[tokio::test]
async fn committed_batches_commit_held_events_before_a_live_delta() {
    let session_id = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let sink = Arc::new(SeenSink::default());
    let emitter = HostEventEmitter::new(log.clone(), sink.clone());
    emitter.enable_committed_batches();

    emitter.emit(held_lifecycle(session_id)).await.unwrap();
    let delta = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            OutputMessageDeltaData {
                turn_id: TurnId::new(),
                message_id: MessageId::new(),
                delta: "h".into(),
                accumulated: "h".into(),
                phase: None,
            },
        ))
        .await
        .unwrap();
    assert_eq!(delta.sequence, None);
    emitter.flush().await.unwrap();
    assert_eq!(committed(&log, session_id).await, 1);
    assert_eq!(sink.types(), ["session.activated", "output.message.delta"]);
}

fn reason_started(session_id: SessionId) -> EventRequest {
    EventRequest::new(
        session_id,
        EventContext::empty(),
        crate::events::ReasonStartedData {
            harness_id: HarnessId::new(),
            agent_id: None,
            metadata: None,
        },
    )
}

#[tokio::test]
async fn progress_markers_commit_behind_and_before_the_next_barrier() {
    let session_id = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let sink = Arc::new(SeenSink::default());
    let emitter = HostEventEmitter::new(log.clone(), sink.clone());
    emitter.enable_committed_batches();

    emitter.emit(held_lifecycle(session_id)).await.unwrap();
    let marker = emitter.emit(reason_started(session_id)).await.unwrap();
    assert_eq!(
        marker.sequence, None,
        "a marker committing behind is provisional"
    );

    let barrier = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(RuntimeMessage::user("hi")),
        ))
        .await
        .unwrap();
    assert_eq!(
        barrier.sequence,
        Some(3),
        "the barrier waits for the marker"
    );
    assert_eq!(
        sink.types(),
        ["session.activated", "reason.started", "input.message"],
        "listeners see committed events in emission order"
    );
}

/// A log whose batches running behind fail, as with a lost lease.
struct RefusesBehind(InMemoryEventLog);

#[async_trait::async_trait]
impl EventReader for RefusesBehind {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.0.read_page(request).await
    }
}

#[async_trait::async_trait]
impl EventLog for RefusesBehind {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        self.0.append(request).await
    }
    async fn append_conditional(
        &self,
        request: EventRequest,
        expected: Option<i32>,
    ) -> Result<Event, EventLogError> {
        self.0.append_conditional(request, expected).await
    }
    async fn append_batch_behind(&self, _: Vec<EventRequest>) -> Result<Vec<Event>, EventLogError> {
        Err(EventLogError::Backend {
            detail: "refused".into(),
        })
    }
    fn durability(&self) -> EventDurability {
        self.0.durability()
    }
}

#[tokio::test]
async fn a_failed_commit_behind_fails_the_next_commit_and_reaches_no_listener() {
    let session_id = SessionId::new();
    let sink = Arc::new(SeenSink::default());
    let emitter = HostEventEmitter::new(
        Arc::new(RefusesBehind(InMemoryEventLog::new())),
        sink.clone(),
    );
    emitter.enable_committed_batches();

    emitter.emit(reason_started(session_id)).await.unwrap();
    let next = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(RuntimeMessage::user("hi")),
        ))
        .await;
    assert!(next.is_err(), "the failure surfaces at the next commit");
    assert!(sink.types().is_empty());
}

#[tokio::test]
async fn abandoned_held_events_are_never_committed() {
    let session_id = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let sink = Arc::new(SeenSink::default());
    let emitter = HostEventEmitter::new(log.clone(), sink.clone());
    emitter.enable_committed_batches();

    emitter.emit(held_lifecycle(session_id)).await.unwrap();
    emitter.abandon();
    emitter.flush().await.unwrap();
    assert_eq!(committed(&log, session_id).await, 0);
    assert!(sink.types().is_empty());
}

#[tokio::test]
async fn a_final_answer_rides_on_turn_completed_and_a_tool_call_does_not_wait() {
    let session_id = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let sink = Arc::new(SeenSink::default());
    let emitter = HostEventEmitter::new(log.clone(), sink.clone());
    emitter.enable_committed_batches();

    let calls = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant_with_tools(
                "",
                vec![everruns_contracts::tool_types::ToolCall {
                    id: "call_1".into(),
                    name: "search".into(),
                    arguments: serde_json::json!({}),
                }],
            )),
        ))
        .await
        .unwrap();
    assert_eq!(
        calls.sequence,
        Some(1),
        "no tool runs on an uncommitted call"
    );

    let answer = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            OutputMessageCompletedData::new(RuntimeMessage::assistant("done")),
        ))
        .await
        .unwrap();
    assert_eq!(answer.sequence, None, "the final answer is held");
    assert_eq!(committed(&log, session_id).await, 1);

    let turn_id = TurnId::new();
    let terminal = emitter
        .emit(EventRequest::new(
            session_id,
            EventContext::empty(),
            crate::events::TurnCompletedData {
                turn_id,
                ..Default::default()
            },
        ))
        .await
        .unwrap();
    assert_eq!(terminal.sequence, Some(3), "a terminal event is never held");
    assert_eq!(
        sink.types(),
        [
            "output.message.completed",
            "output.message.completed",
            "turn.completed"
        ]
    );
}
