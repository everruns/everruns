//! Engine-level event observation.
//!
//! Stability: alpha — may change without a major bump; see [`crate::stability`].

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::SessionEvent;

/// Default number of pending events retained for each Engine listener.
// THREAT[TM-DOS-037]: fixed retention and prefix-free deltas bound observer memory growth.
pub const OBSERVER_QUEUE_CAPACITY: usize = 8192;

/// Selects the event types delivered to an [`EventListener`].
///
/// Stability: alpha.
#[derive(Clone, Debug, Default)]
pub struct EventFilter {
    event_types: Option<Arc<[String]>>,
}

impl EventFilter {
    /// Receive every event.
    pub fn all() -> Self {
        Self::default()
    }

    /// Receive only events whose canonical type matches one of `event_types`.
    pub fn event_types<I, S>(event_types: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            event_types: Some(
                event_types
                    .into_iter()
                    .map(Into::into)
                    .collect::<Vec<_>>()
                    .into(),
            ),
        }
    }

    pub(crate) fn matches(&self, event: &SessionEvent) -> bool {
        self.event_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|kind| kind == event.event_type()))
    }
}

/// Receives projected events from every session owned by an Engine.
///
/// Listener work runs on a dedicated bounded queue. A slow or failing listener
/// cannot delay a turn or another listener.
///
/// Stability: alpha.
#[async_trait]
pub trait EventListener: Send + Sync + 'static {
    /// Process one post-commit event.
    async fn on_event(&self, event: &SessionEvent);

    /// Select the event types this listener receives.
    fn filter(&self) -> EventFilter {
        EventFilter::all()
    }

    /// Flush buffered listener state during [`crate::Engine::shutdown`].
    async fn flush(&self) {}

    /// Human-readable identity used in observer statistics.
    fn name(&self) -> &'static str {
        "EventListener"
    }
}

pub(crate) struct ClosureEventListener<F>(F);

impl<F> ClosureEventListener<F> {
    pub(crate) fn new(handler: F) -> Self {
        Self(handler)
    }
}

#[async_trait]
impl<F, Fut> EventListener for ClosureEventListener<F>
where
    F: Fn(SessionEvent) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = ()> + Send + 'static,
{
    async fn on_event(&self, event: &SessionEvent) {
        (self.0)(event.clone()).await;
    }

    fn name(&self) -> &'static str {
        "on_event closure"
    }
}

/// Delivery counters for one Engine listener.
///
/// Stability: alpha.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListenerStats {
    /// Listener registration order within the Engine.
    pub index: usize,
    /// Listener-provided diagnostic name.
    pub name: String,
    /// Events whose listener future completed successfully.
    pub delivered: u64,
    /// Events rejected because this listener's queue was full.
    pub dropped: u64,
    /// Panics isolated while invoking this listener.
    pub panics: u64,
    /// Whether the listener completed its most recent shutdown flush.
    pub flushed: bool,
}

/// Snapshot of Engine observer delivery counters.
///
/// Stability: alpha.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObserverStats {
    /// Counters in listener registration order.
    pub listeners: Vec<ListenerStats>,
}

impl ObserverStats {
    /// Total number of events dropped across all listeners.
    pub fn dropped(&self) -> u64 {
        self.listeners.iter().map(|listener| listener.dropped).sum()
    }

    /// Total number of isolated listener panics.
    pub fn panics(&self) -> u64 {
        self.listeners.iter().map(|listener| listener.panics).sum()
    }
}

/// Result of a deadline-bounded Engine observer shutdown.
///
/// Stability: alpha.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ObserverReport {
    /// Final counters observed before shutdown returned.
    pub stats: ObserverStats,
    /// Whether at least one listener exceeded the shutdown deadline.
    pub timed_out: bool,
}

struct ListenerCounters {
    delivered: AtomicU64,
    dropped: AtomicU64,
    panics: AtomicU64,
    flushed: AtomicBool,
    next_warning_ms: AtomicU64,
}

impl ListenerCounters {
    fn new() -> Self {
        Self {
            delivered: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            panics: AtomicU64::new(0),
            flushed: AtomicBool::new(false),
            next_warning_ms: AtomicU64::new(0),
        }
    }
}

struct WorkerState {
    receiver: Option<mpsc::Receiver<SessionEvent>>,
    handle: Option<JoinHandle<()>>,
}
struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct ListenerSlot {
    index: usize,
    name: String,
    listener: Arc<dyn EventListener>,
    filter: EventFilter,
    sender: Mutex<Option<mpsc::Sender<SessionEvent>>>,
    worker: Mutex<WorkerState>,
    counters: Arc<ListenerCounters>,
}

impl ListenerSlot {
    fn new(index: usize, listener: Arc<dyn EventListener>, capacity: usize) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel(capacity);
        Arc::new(Self {
            index,
            name: listener.name().to_string(),
            filter: listener.filter(),
            listener,
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(WorkerState {
                receiver: Some(receiver),
                handle: None,
            }),
            counters: Arc::new(ListenerCounters::new()),
        })
    }

    fn ensure_worker(&self) {
        let mut worker = self
            .worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if worker.handle.is_some() {
            return;
        }
        let Some(receiver) = worker.receiver.take() else {
            return;
        };
        worker.handle = Some(tokio::spawn(drain_listener(
            self.listener.clone(),
            receiver,
            self.counters.clone(),
        )));
    }

    fn try_send(&self, event: SessionEvent) {
        self.ensure_worker();
        let sender = self
            .sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(sender) = sender.as_ref() else {
            return;
        };
        match sender.try_send(event) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.counters.dropped.fetch_add(1, Ordering::Relaxed);
                self.warn_overflow();
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }

    fn warn_overflow(&self) {
        const WARNING_INTERVAL_MS: u64 = 60_000;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .min(u128::from(u64::MAX)) as u64;
        let next = self.counters.next_warning_ms.load(Ordering::Relaxed);
        if now >= next
            && self
                .counters
                .next_warning_ms
                .compare_exchange(
                    next,
                    now.saturating_add(WARNING_INTERVAL_MS),
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                )
                .is_ok()
        {
            tracing::warn!(
                listener = %self.name,
                dropped = self.counters.dropped.load(Ordering::Relaxed),
                "Engine event listener queue is full; dropping events"
            );
        }
    }

    fn close(&self) {
        self.sender
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    fn take_handle(&self) -> Option<JoinHandle<()>> {
        self.worker
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .handle
            .take()
    }

    fn stats(&self) -> ListenerStats {
        ListenerStats {
            index: self.index,
            name: self.name.clone(),
            delivered: self.counters.delivered.load(Ordering::Relaxed),
            dropped: self.counters.dropped.load(Ordering::Relaxed),
            panics: self.counters.panics.load(Ordering::Relaxed),
            flushed: self.counters.flushed.load(Ordering::Relaxed),
        }
    }
}

async fn drain_listener(
    listener: Arc<dyn EventListener>,
    mut receiver: mpsc::Receiver<SessionEvent>,
    counters: Arc<ListenerCounters>,
) {
    while let Some(event) = receiver.recv().await {
        let listener = listener.clone();
        let mut invocation =
            AbortOnDrop(tokio::spawn(async move { listener.on_event(&event).await }));
        match (&mut invocation.0).await {
            Ok(()) => {
                counters.delivered.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) if error.is_panic() => {
                counters.panics.fetch_add(1, Ordering::Relaxed);
            }
            Err(_) => {}
        }
    }

    let flush_listener = listener.clone();
    let mut flush = AbortOnDrop(tokio::spawn(async move { flush_listener.flush().await }));
    match (&mut flush.0).await {
        Ok(()) => counters.flushed.store(true, Ordering::Relaxed),
        Err(error) if error.is_panic() => {
            counters.panics.fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {}
    }
}

pub(crate) struct ObserverDispatcher {
    accepting: AtomicBool,
    slots: Vec<Arc<ListenerSlot>>,
}

impl ObserverDispatcher {
    pub(crate) fn new(listeners: Vec<Arc<dyn EventListener>>, queue_capacity: usize) -> Arc<Self> {
        assert!(
            queue_capacity > 0,
            "observer queue capacity must be non-zero"
        );
        Arc::new(Self {
            accepting: AtomicBool::new(true),
            slots: listeners
                .into_iter()
                .enumerate()
                .map(|(index, listener)| ListenerSlot::new(index, listener, queue_capacity))
                .collect(),
        })
    }

    pub(crate) fn dispatch(&self, event: SessionEvent) {
        if !self.accepting.load(Ordering::Acquire) {
            return;
        }
        for slot in &self.slots {
            if slot.filter.matches(&event) {
                slot.try_send(event.clone());
            }
        }
    }

    pub(crate) fn stats(&self) -> ObserverStats {
        ObserverStats {
            listeners: self.slots.iter().map(|slot| slot.stats()).collect(),
        }
    }

    pub(crate) async fn shutdown(&self, timeout: Duration) -> ObserverReport {
        self.accepting.store(false, Ordering::Release);
        for slot in &self.slots {
            slot.ensure_worker();
            slot.close();
        }

        let deadline = tokio::time::Instant::now() + timeout;
        let mut timed_out = false;
        for slot in &self.slots {
            let Some(mut handle) = slot.take_handle() else {
                continue;
            };
            match tokio::time::timeout_at(deadline, &mut handle).await {
                Ok(Ok(())) => {}
                Ok(Err(error)) if error.is_panic() => {
                    slot.counters.panics.fetch_add(1, Ordering::Relaxed);
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    timed_out = true;
                    handle.abort();
                    let _ = handle.await;
                }
            }
        }
        ObserverReport {
            stats: self.stats(),
            timed_out,
        }
    }
}

impl Drop for ObserverDispatcher {
    fn drop(&mut self) {
        self.accepting.store(false, Ordering::Release);
        for slot in &self.slots {
            slot.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::future::pending;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use everruns_core::event_emitter::EventEmitter;
    use everruns_core::events::{EventContext, EventRequest, TurnStartedData};
    use everruns_host::{HostEventEmitter, InMemoryEventLog};
    use everruns_provider::tool_types::ToolCall;
    use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
    use serde_json::json;
    use tokio::sync::Notify;

    use super::{EventFilter, EventListener, OBSERVER_QUEUE_CAPACITY, ObserverDispatcher};
    use crate::events::FacadeEventBus;
    use crate::{Agent, Engine, FunctionTool, Model, SessionEvent};

    #[derive(Clone)]
    struct Recorder {
        events: Arc<Mutex<Vec<SessionEvent>>>,
        flushed: Arc<AtomicBool>,
        filter: EventFilter,
    }

    impl Recorder {
        fn all() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
                flushed: Arc::new(AtomicBool::new(false)),
                filter: EventFilter::all(),
            }
        }

        fn event_types(&self) -> Vec<String> {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .map(|event| event.event_type().to_string())
                .collect()
        }
    }

    #[async_trait]
    impl EventListener for Recorder {
        async fn on_event(&self, event: &SessionEvent) {
            self.events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .push(event.clone());
        }

        fn filter(&self) -> EventFilter {
            self.filter.clone()
        }

        async fn flush(&self) {
            self.flushed.store(true, Ordering::Release);
        }

        fn name(&self) -> &'static str {
            "recorder"
        }
    }

    fn tool_agent() -> Agent {
        let tool = FunctionTool::new(
            "lookup",
            "Look up a value.",
            json!({
                "type": "object",
                "properties": { "key": { "type": "string" } },
                "required": ["key"]
            }),
            |arguments: serde_json::Value| async move {
                Ok::<_, String>(json!({ "value": arguments["key"] }))
            },
        );
        Agent::builder()
            .instructions("Use the lookup tool.")
            .model(Model::simulated_scripted(
                "done",
                vec![
                    vec![ToolCall {
                        id: "call_lookup_1".to_string(),
                        name: "lookup".to_string(),
                        arguments: json!({ "key": "answer" }),
                    }],
                    vec![],
                ],
            ))
            .tool(tool)
            .build()
            .expect("valid agent")
    }

    fn simple_agent() -> Agent {
        Agent::builder()
            .instructions("Reply deterministically.")
            .model(Model::simulated("done"))
            .build()
            .expect("valid agent")
    }

    #[tokio::test]
    async fn listener_observes_simulated_turn_tool_and_output_events() {
        let recorder = Recorder::all();
        let engine = Engine::builder().listener(recorder.clone()).build();

        let turn = engine
            .create(tool_agent())
            .run("look it up")
            .await
            .expect("tool turn runs");
        assert!(turn.success);

        let report = engine.shutdown(Duration::from_secs(5)).await;
        assert!(!report.timed_out);
        assert!(recorder.flushed.load(Ordering::Acquire));
        let event_types = recorder.event_types();
        for expected in [
            "turn.started",
            "tool.started",
            "tool.completed",
            "output.message.completed",
            "turn.completed",
        ] {
            assert!(
                event_types.iter().any(|event_type| event_type == expected),
                "missing {expected}: {event_types:?}"
            );
        }
    }

    #[tokio::test]
    async fn resumed_session_delivers_only_new_events() {
        let recorder = Recorder::all();
        let engine = Engine::builder().listener(recorder.clone()).build();
        let session = engine.create(simple_agent());
        let session_id = session.session_id();

        session.run("first").await.expect("first turn runs");
        drop(session);
        engine
            .resume(session_id)
            .await
            .expect("session resumes")
            .run("second")
            .await
            .expect("second turn runs");

        engine.shutdown(Duration::from_secs(5)).await;
        let events = recorder
            .events
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type() == "turn.started")
                .count(),
            2
        );
        let mut ids = events
            .iter()
            .map(|event| event.event_id.as_str())
            .collect::<Vec<_>>();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), events.len(), "resume must not replay old events");
    }

    struct SlowListener {
        blocked: AtomicBool,
        entered: Arc<Notify>,
        release: Arc<Notify>,
    }

    #[async_trait]
    impl EventListener for SlowListener {
        async fn on_event(&self, _event: &SessionEvent) {
            if !self.blocked.swap(true, Ordering::AcqRel) {
                self.entered.notify_one();
                self.release.notified().await;
            }
        }

        fn name(&self) -> &'static str {
            "slow"
        }
    }

    #[tokio::test]
    async fn slow_listener_never_blocks_turn_and_overflow_is_counted() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let engine = Engine::builder()
            .observer_queue_capacity(1)
            .listener(SlowListener {
                blocked: AtomicBool::new(false),
                entered: entered.clone(),
                release: release.clone(),
            })
            .build();
        let session = engine.create(simple_agent());
        let run = tokio::spawn(async move { session.run("hello").await });

        tokio::time::timeout(Duration::from_secs(5), entered.notified())
            .await
            .expect("listener receives an event");
        let turn = tokio::time::timeout(Duration::from_secs(5), run)
            .await
            .expect("slow listener does not delay the turn")
            .expect("turn task remains healthy")
            .expect("turn runs");
        assert!(turn.success);
        assert!(engine.observer_stats().dropped() > 0);

        release.notify_waiters();
        let report = engine.shutdown(Duration::from_secs(5)).await;
        assert!(!report.timed_out);
    }

    struct PanickingListener;

    #[async_trait]
    impl EventListener for PanickingListener {
        async fn on_event(&self, _event: &SessionEvent) {
            panic!("listener panic");
        }

        fn name(&self) -> &'static str {
            "panicking"
        }
    }

    #[tokio::test]
    async fn panicking_listener_isolated_from_turn_and_other_listeners() {
        let recorder = Recorder::all();
        let engine = Engine::builder()
            .listener(PanickingListener)
            .listener(recorder.clone())
            .build();

        let turn = engine
            .create(simple_agent())
            .run("hello")
            .await
            .expect("turn runs");
        assert!(turn.success);

        let report = engine.shutdown(Duration::from_secs(5)).await;
        assert!(report.stats.panics() > 0);
        assert!(!recorder.event_types().is_empty());
    }

    #[tokio::test]
    async fn shutdown_drains_every_queued_event_and_flushes() {
        let recorder = Recorder::all();
        let engine = Engine::builder().listener(recorder.clone()).build();
        let session = engine.create(simple_agent());
        let mut stream = session.events();

        session.run("hello").await.expect("turn runs");

        let report = engine.shutdown(Duration::from_secs(5)).await;
        assert!(!report.timed_out);
        let mut observed = 0;
        while stream
            .try_recv()
            .expect("session event stream stays lossless")
            .is_some()
        {
            observed += 1;
        }
        assert_eq!(report.stats.listeners[0].delivered as usize, observed);
        assert!(report.stats.listeners[0].flushed);
    }
    struct PendingListener {
        entered: Arc<Notify>,
        cancelled: Arc<Notify>,
    }

    struct CancellationGuard(Arc<Notify>);

    impl Drop for CancellationGuard {
        fn drop(&mut self) {
            self.0.notify_one();
        }
    }

    #[async_trait]
    impl EventListener for PendingListener {
        async fn on_event(&self, _event: &SessionEvent) {
            let _guard = CancellationGuard(self.cancelled.clone());
            self.entered.notify_one();
            pending::<()>().await;
        }
    }

    #[tokio::test]
    async fn shutdown_deadline_cancels_in_flight_listener() {
        let entered = Arc::new(Notify::new());
        let cancelled = Arc::new(Notify::new());
        let dispatcher = ObserverDispatcher::new(
            vec![Arc::new(PendingListener {
                entered: entered.clone(),
                cancelled: cancelled.clone(),
            })],
            OBSERVER_QUEUE_CAPACITY,
        );
        let session_id = SessionId::new();
        let bus = Arc::new(FacadeEventBus::new(session_id, dispatcher.clone()));
        let emitter = HostEventEmitter::new(Arc::new(InMemoryEventLog::new()), bus);
        let turn_id = TurnId::new();
        emitter
            .emit(EventRequest::new(
                session_id,
                EventContext::turn(turn_id, MessageId::new()),
                TurnStartedData {
                    turn_id,
                    input_message_id: MessageId::new(),
                    input_content: None,
                    agent_id: None,
                    agent_name: None,
                    agent_description: None,
                },
            ))
            .await
            .expect("event emits");
        entered.notified().await;

        let report = dispatcher.shutdown(Duration::ZERO).await;

        assert!(report.timed_out);
        tokio::time::timeout(Duration::from_secs(1), cancelled.notified())
            .await
            .expect("deadline cancels the active listener callback");
    }

    #[tokio::test]
    async fn filter_and_on_event_closure_limit_and_deliver_events() {
        let filtered = Recorder {
            filter: EventFilter::event_types(["turn.completed"]),
            ..Recorder::all()
        };
        let closure_events = Arc::new(Mutex::new(Vec::new()));
        let closure_output = closure_events.clone();
        let engine = Engine::builder()
            .listener(filtered.clone())
            .on_event(move |event| {
                let output = closure_output.clone();
                async move {
                    output
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(event.event_type().to_string());
                }
            })
            .build();

        engine
            .create(simple_agent())
            .run("hello")
            .await
            .expect("turn runs");
        engine.shutdown(Duration::from_secs(5)).await;

        assert_eq!(filtered.event_types(), ["turn.completed"]);
        assert!(
            closure_events
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .iter()
                .any(|event_type| event_type == "turn.completed")
        );
    }

    #[tokio::test]
    async fn child_session_events_reach_engine_listener_not_parent_stream() {
        let recorder = Recorder::all();
        let dispatcher =
            ObserverDispatcher::new(vec![Arc::new(recorder.clone())], OBSERVER_QUEUE_CAPACITY);
        let parent_session_id = SessionId::new();
        let child_session_id = SessionId::new();
        let bus = Arc::new(FacadeEventBus::new(parent_session_id, dispatcher.clone()));
        let mut parent_stream = bus.subscribe();
        let emitter = HostEventEmitter::new(Arc::new(InMemoryEventLog::new()), bus);
        let turn_id = TurnId::new();

        emitter
            .emit(EventRequest::new(
                child_session_id,
                EventContext::turn(turn_id, MessageId::new()),
                TurnStartedData {
                    turn_id,
                    input_message_id: MessageId::new(),
                    input_content: None,
                    agent_id: None,
                    agent_name: None,
                    agent_description: None,
                },
            ))
            .await
            .expect("child event emits");

        dispatcher.shutdown(Duration::from_secs(5)).await;
        assert_eq!(
            recorder.events.lock().unwrap()[0].session_id,
            child_session_id.to_string()
        );
        assert!(
            parent_stream
                .try_recv()
                .expect("parent stream remains healthy")
                .is_none()
        );
    }
}
