//! A turn whose answer committed before its holder died: resume records its
//! end from the log and does not ask the model again.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use async_trait::async_trait;
use everruns_contracts::driver_registry::{
    ChatDriver, LlmCallConfig, LlmResponseStream, LlmStreamEvent, Message,
};
use everruns_contracts::runtime_provider::{Provider, ProviderEndpoint};

use crate::events::{Event, EventRequest};
use crate::host::{
    EventDurability, EventLog, EventLogError, EventPage, EventReadRequest, EventReader,
    HostBackends, InMemoryEventLog, InProcessRuntime, TurnSteering,
};

/// Answers with text and counts its calls.
struct CountingModel(Arc<AtomicUsize>);

#[async_trait]
impl ChatDriver for CountingModel {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        _config: &LlmCallConfig,
    ) -> everruns_contracts::error::Result<LlmResponseStream> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let event = LlmStreamEvent::TextDelta("Done.".into());
        Ok(Box::pin(futures::stream::iter([Ok(event)])))
    }
}

/// While `dying`, an append of `turn.completed` never returns: the holder
/// dies after its answer is durable and before the turn's end.
struct DiesBeforeTurnEnd {
    inner: Arc<InMemoryEventLog>,
    dying: AtomicBool,
}

impl DiesBeforeTurnEnd {
    async fn gate(&self, request: &EventRequest) {
        if self.dying.load(Ordering::SeqCst) && request.event_type == "turn.completed" {
            std::future::pending::<()>().await;
        }
    }
}

#[async_trait]
impl EventReader for DiesBeforeTurnEnd {
    async fn read_page(&self, request: EventReadRequest) -> Result<EventPage, EventLogError> {
        self.inner.read_page(request).await
    }
}

#[async_trait]
impl EventLog for DiesBeforeTurnEnd {
    async fn append(&self, request: EventRequest) -> Result<Event, EventLogError> {
        self.gate(&request).await;
        self.inner.append(request).await
    }

    async fn append_conditional(
        &self,
        request: EventRequest,
        expected_last_sequence: Option<i32>,
    ) -> Result<Event, EventLogError> {
        self.gate(&request).await;
        self.inner
            .append_conditional(request, expected_last_sequence)
            .await
    }

    fn durability(&self) -> EventDurability {
        self.inner.durability()
    }
}

async fn event_types(runtime: &InProcessRuntime) -> Vec<String> {
    runtime
        .events()
        .await
        .expect("events")
        .iter()
        .map(|event| event.event_type.clone())
        .collect()
}

#[tokio::test]
async fn an_answered_turn_cut_off_before_its_end_completes_without_the_model() {
    let calls = Arc::new(AtomicUsize::new(0));
    let log = Arc::new(DiesBeforeTurnEnd {
        inner: Arc::new(InMemoryEventLog::new()),
        dying: AtomicBool::new(true),
    });
    let runtime = InProcessRuntime::builder()
        .backends(HostBackends::in_memory().with_event_log(log.clone()))
        .provider_with_default_model(
            Provider::new("counting", CountingModel(calls.clone())),
            "counting-model",
        )
        .single_session(|session| {
            session
                .harness("resume", "You are concise.")
                .agent("resume-agent", "Answer.")
        })
        .build()
        .await
        .expect("runtime builds");
    let session_id = runtime.default_session_id().expect("default session");

    let cut = tokio::time::timeout(
        Duration::from_millis(500),
        runtime.run_text_turn(session_id, "hi"),
    )
    .await;
    assert!(cut.is_err(), "the turn hangs at its end");
    let before = event_types(&runtime).await;
    assert!(
        before.iter().any(|t| t == "output.message.completed"),
        "{before:?}"
    );
    assert!(!before.iter().any(|t| t == "turn.completed"), "{before:?}");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    log.dying.store(false, Ordering::SeqCst);
    let turn = runtime
        .resume_interrupted_turn(session_id, TurnSteering::new())
        .await
        .expect("resume");

    assert!(turn.success, "{:?}", turn.error);
    assert_eq!(turn.response, "Done.");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "the model is not asked again"
    );
    let after = event_types(&runtime).await;
    assert_eq!(
        after.iter().filter(|t| *t == "turn.completed").count(),
        1,
        "{after:?}"
    );
    assert!(after.iter().any(|t| t == "session.idled"), "{after:?}");
}
