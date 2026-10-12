use super::*;
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
