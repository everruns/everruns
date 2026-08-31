use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use everruns_core::event_emitter::EventEmitter;
use everruns_core::events::{
    Event, EventContext, EventRequest, InputMessageData, OutputMessageCompletedData,
    OutputMessageDeltaData, OutputMessageReplacedData, ToolCompletedData, TurnStartedData,
};
use everruns_core::message::{ContentPart, Message};
use everruns_core::message_retriever::MessageRetriever;
use everruns_host::{
    EventCursor, EventDurability, EventHistory, EventHistoryReadLimit, EventHistoryReadRequest,
    EventLog, EventLogError, EventPage, EventReadLimit, EventReadRequest, EventReader, EventSink,
    EventSinkError, HostEventEmitter, InMemoryEventLog, JsonlEventLog,
};
use everruns_provider::typed_id::{EventId, MessageId, SessionId, TurnId};
use std::sync::Mutex;

fn input(session_id: SessionId, text: &str) -> EventRequest {
    EventRequest::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(Message::user(text)),
    )
}

async fn read_all(reader: &dyn EventReader, session_id: SessionId) -> Vec<Event> {
    let limit = EventReadLimit::new(2).unwrap();
    let mut request = EventReadRequest::new(session_id, limit);
    let mut events = Vec::new();
    loop {
        let page = reader.read_page(request).await.unwrap();
        events.extend(page.events);
        let Some(cursor) = page.next_cursor else {
            return events;
        };
        request = EventReadRequest::from_cursor(cursor, limit);
    }
}

#[tokio::test]
async fn snapshot_pages_do_not_skip_duplicate_or_include_concurrent_appends() {
    let session = SessionId::new();
    let log = InMemoryEventLog::new();
    for text in ["one", "two", "three"] {
        log.append(input(session, text)).await.unwrap();
    }

    let limit = EventReadLimit::new(2).unwrap();
    let first = log
        .read_page(EventReadRequest::new(session, limit))
        .await
        .unwrap();
    assert_eq!(first.snapshot_high_watermark(), 3);
    assert_eq!(first.events.len(), 2);

    log.append(input(session, "four")).await.unwrap();
    let second = log
        .read_page(EventReadRequest::from_cursor(
            first.next_cursor.unwrap(),
            limit,
        ))
        .await
        .unwrap();
    assert_eq!(second.snapshot_high_watermark(), 3);
    assert_eq!(second.events.len(), 1);
    assert!(second.next_cursor.is_none());

    let poll = EventCursor::after(session, second.snapshot_high_watermark()).unwrap();
    let newer = log
        .read_page(EventReadRequest::from_cursor(poll, limit))
        .await
        .unwrap();
    assert_eq!(newer.snapshot_high_watermark(), 4);
    assert_eq!(newer.events.len(), 1);
    assert_eq!(newer.events[0].sequence, Some(4));
}

#[tokio::test]
async fn a_public_continuation_cursor_reads_the_same_pinned_snapshot() {
    let session = SessionId::new();
    let log = InMemoryEventLog::new();
    for text in ["one", "two", "three"] {
        log.append(input(session, text)).await.unwrap();
    }
    let limit = EventReadLimit::new(2).unwrap();
    let first = log
        .read_page(EventReadRequest::new(session, limit))
        .await
        .unwrap();
    log.append(input(session, "four")).await.unwrap();

    // A cursor rebuilt from the page's public parts behaves exactly like the
    // one the reader returned: it stays pinned to the original snapshot.
    let rebuilt = EventCursor::continuation(
        session,
        first.events.last().unwrap().sequence.unwrap(),
        first.snapshot_high_watermark(),
    )
    .unwrap();
    assert_eq!(Some(&rebuilt), first.next_cursor.as_ref());

    let page = log
        .read_page(EventReadRequest::from_cursor(rebuilt, limit))
        .await
        .unwrap();
    assert_eq!(page.snapshot_high_watermark(), 3);
    assert_eq!(page.events.len(), 1);
    assert!(page.next_cursor.is_none());
}

#[test]
fn public_cursor_and_page_constructors_validate_their_invariants() {
    let session = SessionId::new();

    assert!(matches!(
        EventCursor::continuation(session, -1, 3).unwrap_err(),
        EventLogError::InvalidRead { .. }
    ));
    assert!(matches!(
        EventCursor::continuation(session, 1, -1).unwrap_err(),
        EventLogError::InvalidRead { .. }
    ));
    assert!(matches!(
        EventCursor::continuation(session, 4, 3).unwrap_err(),
        EventLogError::IncompatibleCursor { .. }
    ));
    assert_eq!(
        EventCursor::continuation(session, 3, 3)
            .unwrap()
            .snapshot_high_watermark(),
        Some(3)
    );
    assert_eq!(
        EventCursor::after(session, 3)
            .unwrap()
            .snapshot_high_watermark(),
        None
    );

    // Gaps are legitimate: a reader may project a filtered logical sequence.
    let sparse = vec![
        fixed_event(session, EventId::new(), 1, "one"),
        fixed_event(session, EventId::new(), 4, "four"),
    ];
    let page = EventPage::new(sparse.clone(), None, 4).unwrap();
    assert_eq!(page.snapshot_high_watermark(), 4);

    // Out-of-order events, foreign sessions, and unpinned or exhausted
    // continuations are rejected with typed errors.
    let mut reversed = sparse.clone();
    reversed.reverse();
    assert!(matches!(
        EventPage::new(reversed, None, 4).unwrap_err(),
        EventLogError::Corruption { .. }
    ));
    assert!(matches!(
        EventPage::new(
            vec![
                sparse[0].clone(),
                fixed_event(SessionId::new(), EventId::new(), 2, "other session"),
            ],
            None,
            4,
        )
        .unwrap_err(),
        EventLogError::Corruption { .. }
    ));
    assert!(matches!(
        EventPage::new(sparse.clone(), None, 3).unwrap_err(),
        EventLogError::InvalidRead { .. }
    ));
    assert!(matches!(
        EventPage::new(
            sparse.clone(),
            Some(EventCursor::after(session, 4).unwrap()),
            8,
        )
        .unwrap_err(),
        EventLogError::IncompatibleCursor { .. }
    ));
    assert!(matches!(
        EventPage::new(
            sparse.clone(),
            Some(EventCursor::continuation(SessionId::new(), 4, 8).unwrap()),
            8,
        )
        .unwrap_err(),
        EventLogError::CrossSessionCursor { .. }
    ));
    assert!(matches!(
        EventPage::new(
            sparse.clone(),
            Some(EventCursor::continuation(session, 2, 8).unwrap()),
            8,
        )
        .unwrap_err(),
        EventLogError::IncompatibleCursor { .. }
    ));
    assert!(matches!(
        EventPage::new(
            sparse,
            Some(EventCursor::continuation(session, 4, 4).unwrap()),
            4,
        )
        .unwrap_err(),
        EventLogError::IncompatibleCursor { .. }
    ));
}

#[tokio::test]
async fn empty_snapshot_is_pollable_and_cursor_is_session_bound() {
    let first_session = SessionId::new();
    let second_session = SessionId::new();
    let log = InMemoryEventLog::new();
    let limit = EventReadLimit::default();
    let empty = log
        .read_page(EventReadRequest::new(first_session, limit))
        .await
        .unwrap();
    assert_eq!(empty.snapshot_high_watermark(), 0);

    log.append(input(first_session, "later")).await.unwrap();
    let poll = EventCursor::after(first_session, 0).unwrap();
    let page = log
        .read_page(EventReadRequest::from_cursor(poll.clone(), limit))
        .await
        .unwrap();
    assert_eq!(page.events.len(), 1);

    let error = log
        .read_page(EventReadRequest::new(second_session, limit).with_cursor(poll))
        .await
        .unwrap_err();
    assert!(matches!(error, EventLogError::CrossSessionCursor { .. }));
}

#[tokio::test]
async fn history_pages_count_messages_and_skip_lifecycle_envelopes() {
    let session = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let history = EventHistory::new(log.clone());
    log.append(input(session, "one")).await.unwrap();
    let turn_id = TurnId::new();
    log.append(EventRequest::new(
        session,
        EventContext::empty(),
        TurnStartedData {
            turn_id,
            input_message_id: MessageId::new(),
            input_content: Some("one".into()),
        },
    ))
    .await
    .unwrap();
    log.append(input(session, "two")).await.unwrap();

    let limit = EventHistoryReadLimit::new(1).unwrap();
    let first = history
        .read_page(EventHistoryReadRequest::new(session, limit))
        .await
        .unwrap();
    assert_eq!(first.messages.len(), 1);
    assert_eq!(first.messages[0].text(), Some("one"));
    assert_eq!(first.snapshot_high_watermark(), 3);

    let second = history
        .read_page(
            EventHistoryReadRequest::new(session, limit).with_cursor(first.next_cursor.unwrap()),
        )
        .await
        .unwrap();
    assert_eq!(second.messages.len(), 1);
    assert_eq!(second.messages[0].text(), Some("two"));
    assert_eq!(second.snapshot_high_watermark(), 3);
    assert!(second.next_cursor.is_none());
}

#[tokio::test]
async fn history_presence_does_not_claim_session_identity() {
    let empty_session = SessionId::new();
    let orphan_session = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let history = EventHistory::new(log.clone());

    assert!(!history.has_history(empty_session).await.unwrap());

    // History can outlive its session catalog record. The projection reports
    // that factual event presence without claiming the session still exists.
    log.append(input(orphan_session, "retained history"))
        .await
        .unwrap();
    assert!(history.has_history(orphan_session).await.unwrap());
    assert!(!history.has_history(empty_session).await.unwrap());
}

#[derive(Default)]
struct CollectingSink(Mutex<Vec<Event>>);

impl EventSink for CollectingSink {
    fn try_send(&self, event: Event) -> Result<(), EventSinkError> {
        self.0.lock().unwrap().push(event);
        Ok(())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn durable_and_ephemeral_interleave_but_only_durable_replays() {
    let session = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let sink = Arc::new(CollectingSink::default());
    let emitter = HostEventEmitter::new(log.clone(), sink.clone());

    let durable = emitter.emit(input(session, "hello")).await.unwrap();
    let ephemeral = emitter
        .emit(EventRequest::new(
            session,
            EventContext::empty(),
            OutputMessageDeltaData {
                turn_id: TurnId::new(),
                message_id: MessageId::new(),
                delta: "x".into(),
                accumulated: "x".into(),
                phase: None,
            },
        ))
        .await
        .unwrap();

    assert_eq!(durable.sequence, Some(1));
    assert_eq!(ephemeral.sequence, None);
    {
        let delivered = sink.0.lock().unwrap();
        assert_eq!(delivered.len(), 2);
        assert_eq!(delivered[0].id, durable.id);
        assert_eq!(delivered[1].id, ephemeral.id);
    }
    let replay = read_all(log.as_ref(), session).await;
    assert_eq!(replay.len(), 1);
    assert_eq!(replay[0].id, durable.id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_appends_notify_in_per_session_acceptance_order() {
    let session = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let sink = Arc::new(CollectingSink::default());
    let emitter = HostEventEmitter::new(log, sink.clone());
    let mut tasks = tokio::task::JoinSet::new();
    for number in 0..64 {
        let emitter = emitter.clone();
        tasks.spawn(async move {
            emitter
                .emit(input(session, &format!("message {number}")))
                .await
                .unwrap()
        });
    }
    while let Some(result) = tasks.join_next().await {
        result.unwrap();
    }

    let sequences = sink
        .0
        .lock()
        .unwrap()
        .iter()
        .map(|event| event.sequence.expect("durable delivery"))
        .collect::<Vec<_>>();
    assert_eq!(sequences, (1..=64).collect::<Vec<_>>());
}

#[derive(Default)]
struct RejectingSink(AtomicUsize);

impl EventSink for RejectingSink {
    fn try_send(&self, _event: Event) -> Result<(), EventSinkError> {
        match self.0.fetch_add(1, Ordering::SeqCst) {
            0 => Err(EventSinkError::Full { dropped: 3 }),
            _ => Err(EventSinkError::Closed),
        }
    }
}

#[tokio::test]
async fn sink_failure_is_observational_after_successful_commit() {
    let session = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let emitter = HostEventEmitter::new(log.clone(), Arc::new(RejectingSink::default()));
    emitter.emit(input(session, "one")).await.unwrap();
    emitter.emit(input(session, "two")).await.unwrap();

    assert_eq!(read_all(log.as_ref(), session).await.len(), 2);
    assert_eq!(emitter.delivery_stats().full, 3);
    assert_eq!(emitter.delivery_stats().closed, 1);
}

struct FailingLog;

#[async_trait]
impl EventReader for FailingLog {
    async fn read_page(&self, _request: EventReadRequest) -> Result<EventPage, EventLogError> {
        Err(EventLogError::Backend {
            detail: "injected read failure".into(),
        })
    }
}

#[async_trait]
impl EventLog for FailingLog {
    async fn append(&self, _request: EventRequest) -> Result<Event, EventLogError> {
        Err(EventLogError::Backend {
            detail: "injected append failure".into(),
        })
    }

    fn durability(&self) -> EventDurability {
        EventDurability::Volatile
    }
}

#[derive(Default)]
struct CountingSink(AtomicUsize);

impl EventSink for CountingSink {
    fn try_send(&self, _event: Event) -> Result<(), EventSinkError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[tokio::test]
async fn failed_append_has_no_projection_or_sink_side_effect() {
    let sink = Arc::new(CountingSink::default());
    let emitter = HostEventEmitter::new(Arc::new(FailingLog), sink.clone());
    assert!(emitter.emit(input(SessionId::new(), "lost")).await.is_err());
    assert_eq!(sink.0.load(Ordering::SeqCst), 0);

    let empty = Arc::new(InMemoryEventLog::new());
    let history = EventHistory::new(empty);
    assert!(history.load(SessionId::new()).await.unwrap().is_empty());
}

struct CommitFlagLog {
    inner: InMemoryEventLog,
    committed: Arc<AtomicBool>,
}

#[async_trait]
impl EventReader for CommitFlagLog {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.inner.read_page(request).await
    }
}

#[async_trait]
impl EventLog for CommitFlagLog {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        let event = self.inner.append(request).await?;
        self.committed.store(true, Ordering::SeqCst);
        Ok(event)
    }

    fn durability(&self) -> EventDurability {
        EventDurability::Volatile
    }
}

struct PostCommitSink(Arc<AtomicBool>);

impl EventSink for PostCommitSink {
    fn try_send(&self, _event: Event) -> Result<(), EventSinkError> {
        assert!(self.0.load(Ordering::SeqCst), "sink ran before commit");
        Ok(())
    }
}

#[tokio::test]
async fn durable_notification_is_strictly_post_commit() {
    let committed = Arc::new(AtomicBool::new(false));
    let log = Arc::new(CommitFlagLog {
        inner: InMemoryEventLog::new(),
        committed: committed.clone(),
    });
    let emitter = HostEventEmitter::new(log, Arc::new(PostCommitSink(committed)));
    emitter
        .emit(input(SessionId::new(), "hello"))
        .await
        .unwrap();
}

#[tokio::test]
async fn replay_is_identical_with_replacement_and_tool_completion() {
    let session = SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let history = EventHistory::new(log.clone());
    log.append(input(session, "question")).await.unwrap();
    let turn_id = TurnId::new();
    let message_id = MessageId::new();
    log.append(EventRequest::new(
        session,
        EventContext::empty(),
        OutputMessageReplacedData {
            turn_id,
            message_id,
            guardrail_capability_id: "guard".into(),
            guardrail_id: "safe".into(),
            reason_code: "blocked".into(),
            replacement: "safe answer".into(),
        },
    ))
    .await
    .unwrap();
    assert_eq!(history.load(session).await.unwrap().len(), 1);

    log.append(EventRequest::new(
        session,
        EventContext::empty(),
        OutputMessageCompletedData::new(Message::assistant("safe answer").with_id(message_id)),
    ))
    .await
    .unwrap();
    log.append(EventRequest::new(
        session,
        EventContext::empty(),
        ToolCompletedData::success(
            "call_1".into(),
            "read_file".into(),
            vec![ContentPart::text(r#"{"ok":true}"#)],
            Some(1),
        ),
    ))
    .await
    .unwrap();

    let first = serde_json::to_value(history.load(session).await.unwrap()).unwrap();
    let second = serde_json::to_value(history.load(session).await.unwrap()).unwrap();
    assert_eq!(first, second);
    assert_eq!(first.as_array().unwrap().len(), 3);
}

fn fixed_event(session: SessionId, id: EventId, sequence: i32, text: &str) -> Event {
    input(session, text).into_event(id, sequence)
}

#[tokio::test]
async fn jsonl_recovers_only_torn_tail_and_rejects_malformed_complete_record() {
    let temp = tempfile::tempdir().unwrap();
    let recoverable = temp.path().join("recoverable.jsonl");
    let session = SessionId::new();
    {
        let log = JsonlEventLog::open(&recoverable).await.unwrap();
        log.append(input(session, "committed")).await.unwrap();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&recoverable)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
    use std::io::Write;
    std::fs::OpenOptions::new()
        .append(true)
        .open(&recoverable)
        .unwrap()
        .write_all(br#"{"partial""#)
        .unwrap();
    let reopened = JsonlEventLog::open(&recoverable).await.unwrap();
    assert_eq!(read_all(&reopened, session).await.len(), 1);
    assert!(std::fs::read(&recoverable).unwrap().ends_with(b"\n"));

    let corrupt = temp.path().join("corrupt.jsonl");
    std::fs::write(&corrupt, b"not-json\n").unwrap();
    assert!(matches!(
        JsonlEventLog::open(&corrupt).await,
        Err(EventLogError::Corruption { .. })
    ));
}

#[tokio::test]
async fn jsonl_collapses_exact_duplicates_and_rejects_id_or_sequence_conflicts() {
    let temp = tempfile::tempdir().unwrap();
    let session = SessionId::new();
    let event = fixed_event(session, EventId::new(), 1, "one");
    let line = serde_json::to_string(&event).unwrap();

    let duplicate = temp.path().join("duplicate.jsonl");
    std::fs::write(&duplicate, format!("{line}\n{line}\n")).unwrap();
    let log = JsonlEventLog::open(&duplicate).await.unwrap();
    assert_eq!(read_all(&log, session).await.len(), 1);

    let mut conflicting_id = event.clone();
    conflicting_id.tags = Some(vec!["changed".into()]);
    let id_conflict = temp.path().join("id-conflict.jsonl");
    std::fs::write(
        &id_conflict,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&event).unwrap(),
            serde_json::to_string(&conflicting_id).unwrap()
        ),
    )
    .unwrap();
    assert!(matches!(
        JsonlEventLog::open(&id_conflict).await,
        Err(EventLogError::Corruption { .. })
    ));

    let sequence_conflict = fixed_event(session, EventId::new(), 1, "other");
    let seq_path = temp.path().join("sequence-conflict.jsonl");
    std::fs::write(
        &seq_path,
        format!(
            "{}\n{}\n",
            serde_json::to_string(&event).unwrap(),
            serde_json::to_string(&sequence_conflict).unwrap()
        ),
    )
    .unwrap();
    assert!(matches!(
        JsonlEventLog::open(&seq_path).await,
        Err(EventLogError::Corruption { .. })
    ));
}

#[tokio::test]
async fn jsonl_rejects_append_from_stale_second_writer_without_corrupting_log() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("events.jsonl");
    let session = SessionId::new();
    let first = JsonlEventLog::open(&path).await.unwrap();
    let stale = JsonlEventLog::open(&path).await.unwrap();

    let accepted = first.append(input(session, "first")).await.unwrap();
    assert_eq!(accepted.sequence, Some(1));
    assert!(matches!(
        stale.append(input(session, "conflict")).await,
        Err(EventLogError::Backend { .. })
    ));

    drop(first);
    drop(stale);
    let reopened = JsonlEventLog::open(&path).await.unwrap();
    let events = read_all(&reopened, session).await;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].id, accepted.id);
}
