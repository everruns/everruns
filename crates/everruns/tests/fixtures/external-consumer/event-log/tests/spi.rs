//! Acceptance tests for the canonical event-log SPI, executed from outside the
//! Everruns workspace. Everything here uses published public paths only.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_provider::error::Result as CoreResult;
use everruns_core::events::{Event, EventContext, EventRequest, InputMessageData};
use everruns_core::harness_definition::HarnessDefinition;
use everruns_core::message::Message;
use everruns_core::{
    execution_loading::AgentStore, execution_loading::HarnessStore, session_services::KeyInfo, provider_resolution::ProviderStore, session_services::SecretInfo, session_services::SessionStorageStore, execution_loading::SessionStore,
};
use everruns_provider::typed_id::{AgentId, HarnessId, ModelId, SessionId};
use everruns_core::{
    AgentDefinition, CompactionCheckpoint, CompactionCheckpointStore, ExecutionSession,
    ProactiveCompactionAttempt,
};
use everruns_provider::{model_spec::ModelSpec, provider::DriverId};
use everruns_host::{
    EventCursor, EventDurability, EventHistory, EventLog, EventLogError, EventPage, EventReadLimit,
    EventReadRequest, EventReader, EventSink, EventSinkError, HostBackends,
    InProcessRuntimeBuilder, RuntimeAgentStore, RuntimeHarnessStore, RuntimeProviderStore,
    RuntimeSessionStore,
};
use everruns_host::SessionMutator;
use everruns_llmsim::{LlmSimConfig, LlmSimRuntimeExt};
use external_event_log::ExternalEventLog;

struct ExternalAgentStore(Arc<dyn RuntimeAgentStore>);

#[async_trait]
impl AgentStore for ExternalAgentStore {
    async fn get_agent(&self, agent_id: AgentId) -> CoreResult<Option<AgentDefinition>> {
        self.0.get_agent(agent_id).await
    }
}

#[async_trait]
impl RuntimeAgentStore for ExternalAgentStore {
    async fn add_agent(&self, agent: AgentDefinition) -> CoreResult<()> {
        self.0.add_agent(agent).await
    }
}

struct ExternalHarnessStore(Arc<dyn RuntimeHarnessStore>);

#[async_trait]
impl HarnessStore for ExternalHarnessStore {
    async fn get_harness(&self, harness_id: HarnessId) -> CoreResult<Option<HarnessDefinition>> {
        self.0.get_harness(harness_id).await
    }
}

#[async_trait]
impl RuntimeHarnessStore for ExternalHarnessStore {
    async fn add_harness(
        &self,
        harness_id: HarnessId,
        harness: HarnessDefinition,
    ) -> CoreResult<()> {
        self.0.add_harness(harness_id, harness).await
    }
}

struct ExternalSessionStore(Arc<dyn RuntimeSessionStore>);

#[async_trait]
impl SessionStore for ExternalSessionStore {
    async fn get_session(&self, session_id: SessionId) -> CoreResult<Option<ExecutionSession>> {
        self.0.get_session(session_id).await
    }
}

#[async_trait]
impl SessionMutator for ExternalSessionStore {
    async fn update_session_title(
        &self,
        session_id: SessionId,
        title: String,
    ) -> CoreResult<ExecutionSession> {
        self.0.update_session_title(session_id, title).await
    }

    async fn upsert_session_capability(
        &self,
        session_id: SessionId,
        capability: everruns_capability::CapabilityRef,
    ) -> CoreResult<ExecutionSession> {
        self.0
            .upsert_session_capability(session_id, capability)
            .await
    }

    async fn remove_session_capability(
        &self,
        session_id: SessionId,
        capability_id: &str,
    ) -> CoreResult<ExecutionSession> {
        self.0
            .remove_session_capability(session_id, capability_id)
            .await
    }
}

#[async_trait]
impl RuntimeSessionStore for ExternalSessionStore {
    async fn add_session(&self, session: ExecutionSession) -> CoreResult<()> {
        self.0.add_session(session).await
    }
}

struct ExternalProviderStore(Arc<dyn RuntimeProviderStore>);

#[async_trait]
impl ProviderStore for ExternalProviderStore {
    async fn get_model_spec(&self, model_id: ModelId) -> CoreResult<Option<ModelSpec>> {
        self.0.get_model_spec(model_id).await
    }

    async fn get_default_model_spec(&self) -> CoreResult<Option<ModelSpec>> {
        self.0.get_default_model_spec().await
    }

    async fn get_provider_config(
        &self,
        provider: &everruns_provider::runtime_provider::ProviderKey,
    ) -> CoreResult<Option<everruns_provider::driver_registry::ProviderConfig>> {
        self.0.get_provider_config(provider).await
    }
}

#[async_trait]
impl RuntimeProviderStore for ExternalProviderStore {
    async fn set_default_model_spec(&self, model: ModelSpec) -> CoreResult<()> {
        self.0.set_default_model_spec(model).await
    }
}

struct ExternalCheckpointStore(Arc<dyn CompactionCheckpointStore>);

#[async_trait]
impl CompactionCheckpointStore for ExternalCheckpointStore {
    async fn get_latest(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> CoreResult<Option<CompactionCheckpoint>> {
        self.0.get_latest(session_id, provider_type, model).await
    }

    async fn install(&self, checkpoint: CompactionCheckpoint) -> CoreResult<bool> {
        self.0.install(checkpoint).await
    }

    async fn get_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
    ) -> CoreResult<Option<ProactiveCompactionAttempt>> {
        self.0
            .get_proactive_attempt(session_id, provider_type, model)
            .await
    }

    async fn record_proactive_attempt(
        &self,
        session_id: SessionId,
        provider_type: &str,
        model: &str,
        attempt: ProactiveCompactionAttempt,
    ) -> CoreResult<()> {
        self.0
            .record_proactive_attempt(session_id, provider_type, model, attempt)
            .await
    }
}

struct ExternalStorageStore(Arc<dyn SessionStorageStore>);

#[async_trait]
impl SessionStorageStore for ExternalStorageStore {
    async fn set_value(&self, session_id: SessionId, key: &str, value: &str) -> CoreResult<()> {
        self.0.set_value(session_id, key, value).await
    }

    async fn get_value(&self, session_id: SessionId, key: &str) -> CoreResult<Option<String>> {
        self.0.get_value(session_id, key).await
    }

    async fn delete_value(&self, session_id: SessionId, key: &str) -> CoreResult<bool> {
        self.0.delete_value(session_id, key).await
    }

    async fn list_keys(&self, session_id: SessionId) -> CoreResult<Vec<KeyInfo>> {
        self.0.list_keys(session_id).await
    }

    async fn set_secret(&self, session_id: SessionId, name: &str, value: &str) -> CoreResult<()> {
        self.0.set_secret(session_id, name, value).await
    }

    async fn get_secret(&self, session_id: SessionId, name: &str) -> CoreResult<Option<String>> {
        self.0.get_secret(session_id, name).await
    }

    async fn delete_secret(&self, session_id: SessionId, name: &str) -> CoreResult<bool> {
        self.0.delete_secret(session_id, name).await
    }

    async fn list_secrets(&self, session_id: SessionId) -> CoreResult<Vec<SecretInfo>> {
        self.0.list_secrets(session_id).await
    }
}

struct ExternalEventSink;

impl EventSink for ExternalEventSink {
    fn try_send(&self, _event: Event) -> std::result::Result<(), EventSinkError> {
        Ok(())
    }
}

fn external_backends(log: Arc<ExternalEventLog>) -> HostBackends {
    let defaults = HostBackends::in_memory();
    HostBackends {
        harness_store: Arc::new(ExternalHarnessStore(defaults.harness_store)),
        agent_store: Arc::new(ExternalAgentStore(defaults.agent_store)),
        session_store: Arc::new(ExternalSessionStore(defaults.session_store)),
        event_log: log,
        compaction_checkpoint_store: Arc::new(ExternalCheckpointStore(
            defaults.compaction_checkpoint_store,
        )),
        provider_store: Arc::new(ExternalProviderStore(defaults.provider_store)),
        event_sink: Arc::new(ExternalEventSink),
        storage_store: Arc::new(ExternalStorageStore(defaults.storage_store)),
        connection_resolver: defaults.connection_resolver,
        session_task_registry: defaults.session_task_registry,
        schedule_store_factory: defaults.schedule_store_factory,
        tool_context_extensions_factory: defaults.tool_context_extensions_factory,
        subagent_delegate_factory: defaults.subagent_delegate_factory,
        tool_augmentor: defaults.tool_augmentor,
    }
}

fn input(session_id: SessionId, text: &str) -> EventRequest {
    EventRequest::new(
        session_id,
        EventContext::empty(),
        InputMessageData::new(Message::user(text)),
    )
}

fn limit(value: usize) -> EventReadLimit {
    EventReadLimit::new(value).expect("valid read limit")
}

fn sequences(page: &EventPage) -> Vec<i32> {
    page.events
        .iter()
        .map(|event| event.sequence.expect("durable event"))
        .collect()
}

#[tokio::test]
async fn append_assigns_a_sequence_and_returns_the_finalized_event() {
    let session = SessionId::new();
    let log = ExternalEventLog::new();

    let first = log.append(input(session, "one")).await.expect("append");
    let second = log.append(input(session, "two")).await.expect("append");

    assert_eq!(first.sequence, Some(1));
    assert_eq!(second.sequence, Some(2));
    assert_eq!(first.session_id, session);
    assert_ne!(first.id, second.id);
    assert_eq!(first.event_type, "input.message");
    assert_eq!(log.durability(), EventDurability::Volatile);

    // Read-your-accepted-writes: the append is visible to the next read.
    let page = log
        .read_page(EventReadRequest::new(session, limit(8)))
        .await
        .expect("read page");
    assert_eq!(sequences(&page), vec![1, 2]);
    assert_eq!(page.events[0].id, first.id);
}

#[tokio::test]
async fn snapshot_pagination_excludes_concurrent_appends_and_polling_observes_them() {
    let session = SessionId::new();
    let log = ExternalEventLog::new();
    for text in ["one", "two", "three"] {
        log.append(input(session, text)).await.expect("append");
    }

    let first = log
        .read_page(EventReadRequest::new(session, limit(2)))
        .await
        .expect("first page");
    assert_eq!(first.snapshot_high_watermark(), 3);
    assert_eq!(sequences(&first), vec![1, 2]);
    let continuation = first.next_cursor.clone().expect("continuation");
    assert_eq!(continuation.session_id(), session);
    assert_eq!(continuation.after_sequence(), 2);
    assert_eq!(continuation.snapshot_high_watermark(), Some(3));

    // An append committed mid-pagination must stay outside the pinned snapshot.
    log.append(input(session, "four")).await.expect("append");

    let second = log
        .read_page(EventReadRequest::from_cursor(continuation, limit(2)))
        .await
        .expect("second page");
    assert_eq!(second.snapshot_high_watermark(), 3);
    assert_eq!(sequences(&second), vec![3]);
    assert!(second.next_cursor.is_none());

    // Polling starts a new snapshot and does observe the later append.
    let poll = EventCursor::after(session, second.snapshot_high_watermark()).expect("poll cursor");
    assert_eq!(poll.snapshot_high_watermark(), None);
    let polled = log
        .read_page(EventReadRequest::from_cursor(poll, limit(2)))
        .await
        .expect("poll page");
    assert_eq!(polled.snapshot_high_watermark(), 4);
    assert_eq!(sequences(&polled), vec![4]);
}

#[tokio::test]
async fn sequence_gaps_are_accepted_and_stay_ordered() {
    let session = SessionId::new();
    let log = ExternalEventLog::new();
    log.append(input(session, "visible one"))
        .await
        .expect("append");
    log.append_hidden(input(session, "internal record"));
    log.append_hidden(input(session, "internal record"));
    log.append(input(session, "visible two"))
        .await
        .expect("append");
    log.append(input(session, "visible three"))
        .await
        .expect("append");

    assert_eq!(log.physical_len(session), 5);

    let first = log
        .read_page(EventReadRequest::new(session, limit(2)))
        .await
        .expect("first page");
    assert_eq!(first.snapshot_high_watermark(), 5);
    assert_eq!(sequences(&first), vec![1, 4]);

    let second = log
        .read_page(EventReadRequest::from_cursor(
            first.next_cursor.clone().expect("continuation"),
            limit(2),
        ))
        .await
        .expect("second page");
    assert_eq!(sequences(&second), vec![5]);
    assert!(second.next_cursor.is_none());
}

#[tokio::test]
async fn cross_session_and_unavailable_snapshot_cursors_fail() {
    let session = SessionId::new();
    let other_session = SessionId::new();
    let log = ExternalEventLog::new();
    log.append(input(session, "one")).await.expect("append");

    let foreign = EventCursor::continuation(other_session, 0, 1).expect("cursor");
    let error = log
        .read_page(EventReadRequest::new(session, limit(4)).with_cursor(foreign))
        .await
        .expect_err("cross-session cursor must fail");
    assert!(matches!(error, EventLogError::CrossSessionCursor { .. }));

    let unavailable = EventCursor::continuation(session, 0, 99).expect("cursor");
    let error = log
        .read_page(EventReadRequest::from_cursor(unavailable, limit(4)))
        .await
        .expect_err("unavailable snapshot must fail");
    assert!(matches!(error, EventLogError::ExpiredCursor { .. }));
}

#[test]
fn cursor_and_page_construction_reject_inconsistent_positions() {
    let session = SessionId::new();

    assert!(matches!(
        EventCursor::continuation(session, -1, 4).expect_err("negative position"),
        EventLogError::InvalidRead { .. }
    ));
    assert!(matches!(
        EventCursor::continuation(session, 5, 4).expect_err("position beyond snapshot"),
        EventLogError::IncompatibleCursor { .. }
    ));
    assert!(matches!(
        EventPage::new(Vec::new(), None, -1).expect_err("negative watermark"),
        EventLogError::InvalidRead { .. }
    ));

    // A continuation on an exhausted snapshot promises events that cannot exist.
    let exhausted = EventCursor::continuation(session, 4, 4).expect("cursor");
    assert!(matches!(
        EventPage::new(Vec::new(), Some(exhausted), 4).expect_err("exhausted continuation"),
        EventLogError::IncompatibleCursor { .. }
    ));

    // A page may not return events beyond the snapshot it claims.
    let event: Event = input(session, "one").into_event(everruns_provider::typed_id::EventId::new(), 5);
    assert!(matches!(
        EventPage::new(vec![event], None, 4).expect_err("sequence beyond snapshot"),
        EventLogError::InvalidRead { .. }
    ));
}

/// Reader that records the request shapes it is asked to serve, proving an
/// external implementation can tell an initial read from a continuation.
#[derive(Default)]
struct RecordingReader {
    seen: std::sync::Mutex<Vec<Option<(i32, Option<i32>)>>>,
}

#[async_trait]
impl EventReader for RecordingReader {
    async fn read_page(
        &self,
        request: EventReadRequest,
    ) -> std::result::Result<EventPage, EventLogError> {
        self.seen.lock().expect("recorder mutex").push(
            request
                .cursor()
                .map(|cursor| (cursor.after_sequence(), cursor.snapshot_high_watermark())),
        );
        EventPage::new(Vec::new(), None, 0)
    }
}

#[tokio::test]
async fn a_reader_can_inspect_the_request_cursor() {
    let session = SessionId::new();
    let reader = RecordingReader::default();

    reader
        .read_page(EventReadRequest::new(session, limit(4)))
        .await
        .expect("initial read");
    reader
        .read_page(EventReadRequest::from_cursor(
            EventCursor::continuation(session, 2, 7).expect("cursor"),
            limit(4),
        ))
        .await
        .expect("continuation read");
    reader
        .read_page(EventReadRequest::from_cursor(
            EventCursor::after(session, 7).expect("cursor"),
            limit(4),
        ))
        .await
        .expect("poll read");

    let seen = reader.seen.lock().expect("recorder mutex").clone();
    assert_eq!(seen, vec![None, Some((2, Some(7))), Some((7, None))]);
}

#[tokio::test]
async fn the_external_log_serves_host_composition_and_message_projection() {
    let log = Arc::new(ExternalEventLog::new());
    let runtime = InProcessRuntimeBuilder::new()
        .llm_sim_as_default(LlmSimConfig::fixed("Four."))
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .backends(external_backends(log.clone()))
        .single_session(|session| {
            session
                .harness("chat", "You are concise.")
                .agent("chat-agent", "Answer directly.")
                .session_title("External Event Log Session")
        })
        .build()
        .await
        .expect("build runtime");

    let session_id = runtime.default_session_id().expect("single session id");
    let turn = runtime
        .run_text_turn(session_id, "What is 2 + 2?")
        .await
        .expect("run turn");
    assert!(turn.success);
    assert_eq!(turn.response, "Four.");

    // The turn's canonical events landed in the external log ...
    let page = log
        .read_page(EventReadRequest::new(session_id, limit(64)))
        .await
        .expect("read external log");
    assert!(page.snapshot_high_watermark() > 0);
    assert!(
        page.events
            .iter()
            .any(|event| event.event_type == "input.message")
    );

    // ... and the host's read-only projection rebuilds from it.
    let messages = EventHistory::new(log.clone())
        .read_page(everruns_host::EventHistoryReadRequest::new(
            session_id,
            everruns_host::EventHistoryReadLimit::new(16).expect("valid history limit"),
        ))
        .await
        .expect("project history");
    assert_eq!(
        messages
            .messages
            .iter()
            .filter_map(|message| message.text())
            .collect::<Vec<_>>(),
        vec!["What is 2 + 2?", "Four."]
    );
}
