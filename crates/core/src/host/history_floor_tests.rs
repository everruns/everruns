//! A long session compacts natively, restores its checkpoint on later turns,
//! and loads only the history after it ([`crate::HistoryFloor`]).

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns_contracts::compact::{CompactOutputItem, CompactRequest, CompactResponse};
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmResponseStream, LlmStreamEvent, Message,
};
use everruns_contracts::error::Result;
use everruns_contracts::runtime_provider::{Provider, ProviderEndpoint};

use crate::events::{Event, EventData, EventRequest};
use crate::host::events::{
    EventDurability, EventLog, EventLogError, EventPage, EventReadRequest, EventReader,
    InMemoryEventLog,
};
use crate::host::{HostBackends, InProcessRuntime};

/// Answers at length, compacts natively (one opaque item for the whole
/// history, as OpenAI's Responses API does), and has a small window, so a
/// session compacts every few turns.
#[derive(Clone, Default)]
struct Compactor {
    /// Messages in each model request.
    requests: Arc<Mutex<Vec<usize>>>,
}

#[async_trait]
impl ChatDriver for Compactor {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        self.requests.lock().unwrap().push(messages.len());
        let answer = "Noted, and here is what follows from it. ".repeat(10);
        Ok(Box::pin(futures::stream::iter([Ok(
            LlmStreamEvent::TextDelta(answer),
        )])))
    }

    fn supports_compact(&self) -> bool {
        true
    }

    fn effective_context_window(&self, _model: &str) -> Option<usize> {
        Some(4_000)
    }

    async fn compact(
        &self,
        _endpoint: &ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        Ok(Some(CompactResponse {
            output: vec![CompactOutputItem::Compaction {
                encrypted_content: format!("{} items", request.input.len()),
            }],
            usage: None,
        }))
    }
}

/// The in-memory log, noting where each read starts.
#[derive(Clone)]
struct ReadLog {
    inner: Arc<InMemoryEventLog>,
    starts: Arc<Mutex<Vec<i32>>>,
}

#[async_trait]
impl EventReader for ReadLog {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        if let Some(cursor) = request.cursor() {
            self.starts.lock().unwrap().push(cursor.after_sequence());
        } else if request.limit().get() == 1 {
            // A probe for the log's end (tail reads), not a scan.
            self.starts.lock().unwrap().push(-1);
        } else {
            self.starts.lock().unwrap().push(0);
        }
        self.inner.read_page(request).await
    }
}

#[async_trait]
impl EventLog for ReadLog {
    async fn append(&self, request: EventRequest) -> std::result::Result<Event, EventLogError> {
        self.inner.append(request).await
    }

    async fn append_conditional(
        &self,
        request: EventRequest,
        expected_last_sequence: Option<i32>,
    ) -> std::result::Result<Event, EventLogError> {
        self.inner
            .append_conditional(request, expected_last_sequence)
            .await
    }

    fn durability(&self) -> EventDurability {
        self.inner.durability()
    }
}

// The compaction capability is a builtin.
#[cfg(feature = "builtins")]
#[tokio::test]
async fn later_turns_load_only_the_history_after_the_checkpoint() {
    let model = Compactor::default();
    let log = ReadLog {
        inner: Arc::new(InMemoryEventLog::new()),
        starts: Arc::default(),
    };
    let runtime = InProcessRuntime::builder()
        .backends(HostBackends::in_memory().with_event_log(Arc::new(log.clone())))
        .provider_with_default_model(Provider::new("compactor", model.clone()), "compactor-model")
        .capability(crate::builtins::CompactionCapability)
        .single_session(|session| {
            session
                .harness("long", "You keep long conversations useful.")
                .agent("long-agent", "Answer.")
                .agent_capability(everruns_contracts::CapabilityRef::with_config(
                    "compaction",
                    serde_json::json!({ "strategy": "native", "budget_percent": 0.1 }),
                ))
        })
        .build()
        .await
        .expect("runtime builds");
    let session = runtime.default_session_id().expect("default session");

    for turn in 0..30 {
        let result = runtime
            .run_text_turn(session, &format!("message {turn}"))
            .await
            .expect("turn runs");
        assert!(result.success, "turn {turn}: {:?}", result.error);
    }

    let events = runtime.events().await.expect("events");
    let checkpoints: Vec<i64> = events
        .iter()
        .filter_map(|event| match &event.data {
            EventData::ContextCompacted(data) if data.checkpoint_id.is_some() => {
                data.source_sequence
            }
            _ => None,
        })
        .collect();
    assert!(
        checkpoints.len() >= 2,
        "the session compacted more than once: {checkpoints:?}"
    );
    // Later turns restored a checkpoint: no request carries the whole
    // conversation (two messages a turn).
    let largest = *model.requests.lock().unwrap().iter().max().unwrap();
    assert!(largest < 30, "largest request had {largest} messages");
    // And read the history from the newest checkpoint, not from the start.
    let newest = *checkpoints.last().unwrap() as i32;
    assert!(
        log.starts
            .lock()
            .unwrap()
            .iter()
            .any(|start| *start >= newest),
        "no read started at the checkpoint (sequence {newest})"
    );
    // Once a checkpoint exists, a turn reads nothing from the log's start.
    log.starts.lock().unwrap().clear();
    let result = runtime
        .run_text_turn(session, "one more")
        .await
        .expect("turn runs");
    assert!(result.success, "{:?}", result.error);
    let starts = log.starts.lock().unwrap().clone();
    assert!(
        !starts.contains(&0),
        "a turn after the checkpoint scanned the whole log: {starts:?}"
    );
}

#[tokio::test]
async fn the_floor_holds_only_when_the_turn_input_is_after_it() {
    use crate::events::{EventContext, InputMessageData};
    use crate::message::RuntimeMessage;
    use crate::message_filter::MessageQuery;
    use crate::{HistoryFloor, host::events::EventHistory};

    let session = everruns_contracts::typed_id::SessionId::new();
    let log = Arc::new(InMemoryEventLog::new());
    let mut ids = Vec::new();
    for text in ["one", "two", "three", "four"] {
        let message = RuntimeMessage::user(text);
        ids.push(message.id);
        log.append(EventRequest::new(
            session,
            EventContext::empty(),
            InputMessageData::new(message),
        ))
        .await
        .expect("append");
    }
    let history = EventHistory::new(log);
    let floor = |input| HistoryFloor {
        after_sequence: 2,
        input_message_id: Some(input),
    };

    let (loaded, used) =
        super::load_history(&history, MessageQuery::new(session), Some(floor(ids[3])))
            .await
            .expect("load");
    assert_eq!(used, Some(2));
    assert_eq!(loaded.messages.len(), 2);

    // A checkpoint taken mid-turn covers the turn's input: load everything.
    let (loaded, used) =
        super::load_history(&history, MessageQuery::new(session), Some(floor(ids[1])))
            .await
            .expect("load");
    assert_eq!(used, None);
    assert_eq!(loaded.messages.len(), 4);
}
