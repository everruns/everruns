//! Engine-level event observation.
//!
//! Stability: alpha — may change without a major bump; see [`crate::stability`].

use std::future::Future;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
#[cfg(any(feature = "otel", feature = "braintrust"))]
use everruns_core::EventListener as CoreEventListener;
use everruns_core::events::Event as CoreEvent;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

use crate::SessionEvent;

/// Default number of pending events retained for each Engine listener.
// THREAT[TM-DOS-040]: fixed retention and prefix-free deltas bound observer memory growth.
pub const OBSERVER_QUEUE_CAPACITY: usize = 8192;

/// Maximum serialized event bytes retained by one Engine listener.
pub const OBSERVER_QUEUE_BYTE_CAPACITY: usize = 8 * 1024 * 1024;

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

pub(crate) enum ListenerRegistration {
    App(Arc<dyn EventListener>),
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    Host(Arc<dyn CoreEventListener>),
}

/// One dispatched event, shared by every listener that matched it.
///
/// The `core` half is `None` unless a host listener actually matched, so an
/// app-only build never clones a `CoreEvent` it has nobody to hand it to
/// (TM-DOS-037). When both listener kinds match, the two halves still travel in
/// one `Arc`: sharing is what keeps a dispatch O(1) in the number of listeners
/// rather than O(n), and the per-listener byte budget below is what bounds what
/// a slow listener can pin.
struct DispatchEvent {
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    core: Option<CoreEvent>,
    facade: SessionEvent,
}

impl DispatchEvent {
    fn retained_bytes(&self) -> usize {
        let facade = self.facade.as_json().to_string().len()
            + self.facade.canonical_json().to_string().len();
        #[cfg(any(feature = "otel", feature = "braintrust"))]
        let core = self.core.as_ref().map_or(0, |core| {
            serde_json::to_vec(core).map_or(0, |value| value.len())
        });
        #[cfg(not(any(feature = "otel", feature = "braintrust")))]
        let core = 0;
        facade.saturating_add(core)
    }
}

struct QueuedEvent {
    event: Arc<DispatchEvent>,
    retained_bytes: Arc<AtomicUsize>,
    bytes: usize,
}

impl Drop for QueuedEvent {
    fn drop(&mut self) {
        self.retained_bytes.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

fn reserve_retained_bytes(retained: &AtomicUsize, bytes: usize, capacity: usize) -> bool {
    retained
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(bytes)
                .filter(|total| *total <= capacity)
        })
        .is_ok()
}

enum ListenerTarget {
    App(Arc<dyn EventListener>),
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    Host(Arc<dyn CoreEventListener>),
}

impl ListenerTarget {
    async fn on_event(&self, event: &DispatchEvent) {
        match self {
            Self::App(listener) => listener.on_event(&event.facade).await,
            // `dispatch` clones the core half whenever a host listener matched,
            // so a host listener can only be handed an event that carries one.
            #[cfg(any(feature = "otel", feature = "braintrust"))]
            Self::Host(listener) => {
                if let Some(core) = event.core.as_ref() {
                    listener.on_event(core).await;
                }
            }
        }
    }

    async fn flush(&self) {
        match self {
            Self::App(listener) => listener.flush().await,
            #[cfg(any(feature = "otel", feature = "braintrust"))]
            Self::Host(listener) => listener.flush().await,
        }
    }
}

enum ListenerFilter {
    App(EventFilter),
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    Host(Option<Vec<&'static str>>),
}

impl ListenerFilter {
    fn matches(&self, _core: &CoreEvent, facade: &SessionEvent) -> bool {
        match self {
            Self::App(filter) => filter.matches(facade),
            #[cfg(any(feature = "otel", feature = "braintrust"))]
            Self::Host(None) => true,
            #[cfg(any(feature = "otel", feature = "braintrust"))]
            Self::Host(Some(types)) => types.contains(&_core.event_type.as_str()),
        }
    }

    /// Whether this listener consumes the raw `CoreEvent` half of a dispatch.
    /// `dispatch` uses it to decide whether cloning that half is worth anything.
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    fn is_host(&self) -> bool {
        matches!(self, Self::Host(_))
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

async fn finish_shutdown(
    slots: Vec<Arc<ListenerSlot>>,
    handles: Vec<(Arc<ListenerSlot>, JoinHandle<()>)>,
    timeout: Duration,
    completion: watch::Sender<Option<ObserverReport>>,
) {
    let deadline = tokio::time::Instant::now().checked_add(timeout);
    let mut timed_out = false;
    for (slot, mut handle) in handles {
        let outcome = match deadline {
            Some(deadline) => tokio::time::timeout_at(deadline, &mut handle).await,
            None => Ok((&mut handle).await),
        };
        match outcome {
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
    let report = ObserverReport {
        stats: ObserverStats {
            listeners: slots.iter().map(|slot| slot.stats()).collect(),
        },
        timed_out,
    };
    let _ = completion.send(Some(report));
}

struct WorkerState {
    receiver: Option<mpsc::Receiver<QueuedEvent>>,
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
    listener: Arc<ListenerTarget>,
    filter: ListenerFilter,
    sender: Mutex<Option<mpsc::Sender<QueuedEvent>>>,
    worker: Mutex<WorkerState>,
    counters: Arc<ListenerCounters>,
    retained_bytes: Arc<AtomicUsize>,
}

impl ListenerSlot {
    fn new(index: usize, registration: ListenerRegistration, capacity: usize) -> Arc<Self> {
        let (sender, receiver) = mpsc::channel(capacity);
        let (name, filter, listener) = match registration {
            ListenerRegistration::App(listener) => (
                listener.name().to_string(),
                ListenerFilter::App(listener.filter()),
                ListenerTarget::App(listener),
            ),
            #[cfg(any(feature = "otel", feature = "braintrust"))]
            ListenerRegistration::Host(listener) => (
                listener.name().to_string(),
                ListenerFilter::Host(listener.event_types()),
                ListenerTarget::Host(listener),
            ),
        };
        Arc::new(Self {
            index,
            name,
            filter,
            listener: Arc::new(listener),
            sender: Mutex::new(Some(sender)),
            worker: Mutex::new(WorkerState {
                receiver: Some(receiver),
                handle: None,
            }),
            counters: Arc::new(ListenerCounters::new()),
            retained_bytes: Arc::new(AtomicUsize::new(0)),
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

    fn try_send(&self, event: Arc<DispatchEvent>) {
        self.ensure_worker();
        let bytes = event.retained_bytes();
        if !reserve_retained_bytes(&self.retained_bytes, bytes, OBSERVER_QUEUE_BYTE_CAPACITY) {
            self.counters.dropped.fetch_add(1, Ordering::Relaxed);
            self.warn_overflow();
            return;
        }
        let event = QueuedEvent {
            event,
            retained_bytes: self.retained_bytes.clone(),
            bytes,
        };
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
    listener: Arc<ListenerTarget>,
    mut receiver: mpsc::Receiver<QueuedEvent>,
    counters: Arc<ListenerCounters>,
) {
    while let Some(event) = receiver.recv().await {
        let listener = listener.clone();
        let mut invocation = AbortOnDrop(tokio::spawn(async move {
            listener.on_event(&event.event).await
        }));
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
    shutdown: Mutex<Option<watch::Receiver<Option<ObserverReport>>>>,
}

impl ObserverDispatcher {
    pub(crate) fn new(listeners: Vec<ListenerRegistration>, queue_capacity: usize) -> Arc<Self> {
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
            shutdown: Mutex::new(None),
        })
    }

    /// Build the one payload every matching listener will share, or `None` when
    /// nothing matched.
    ///
    /// Split out of `dispatch` so the cloning decision — the part TM-DOS-037
    /// turns on — can be asserted directly, without standing up listener queues
    /// and worker tasks to observe it second-hand.
    fn build_event(&self, core: &CoreEvent, facade: &SessionEvent) -> Option<Arc<DispatchEvent>> {
        // Match first, clone second. Filtering is a string compare, while the
        // payload halves are full event clones, so deciding who wants this event
        // before building it means an event nobody matched costs nothing and the
        // `CoreEvent` half is only cloned when a host listener is there to read
        // it (TM-DOS-037). Two filter passes rather than collecting the matches:
        // an allocation per dispatched event would cost more than re-running the
        // compare.
        let mut any_match = false;
        #[cfg(any(feature = "otel", feature = "braintrust"))]
        let mut wants_core = false;
        for slot in &self.slots {
            if slot.filter.matches(core, facade) {
                any_match = true;
                #[cfg(any(feature = "otel", feature = "braintrust"))]
                if slot.filter.is_host() {
                    wants_core = true;
                }
            }
        }
        if !any_match {
            return None;
        }

        let _ = core;
        Some(Arc::new(DispatchEvent {
            #[cfg(any(feature = "otel", feature = "braintrust"))]
            core: wants_core.then(|| core.clone()),
            facade: facade.clone(),
        }))
    }

    pub(crate) fn dispatch(&self, core: &CoreEvent, facade: &SessionEvent) {
        if !self.accepting.load(Ordering::Acquire) {
            return;
        }
        let Some(event) = self.build_event(core, facade) else {
            return;
        };
        for slot in &self.slots {
            if slot.filter.matches(core, facade) {
                slot.try_send(Arc::clone(&event));
            }
        }
    }

    pub(crate) fn stats(&self) -> ObserverStats {
        ObserverStats {
            listeners: self.slots.iter().map(|slot| slot.stats()).collect(),
        }
    }

    pub(crate) async fn shutdown(&self, timeout: Duration) -> ObserverReport {
        let mut completion = {
            let mut shutdown = self
                .shutdown
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if let Some(completion) = shutdown.as_ref() {
                completion.clone()
            } else {
                self.accepting.store(false, Ordering::Release);
                for slot in &self.slots {
                    slot.ensure_worker();
                    slot.close();
                }
                let handles = self
                    .slots
                    .iter()
                    .filter_map(|slot| slot.take_handle().map(|handle| (slot.clone(), handle)))
                    .collect();
                let slots = self.slots.clone();
                let (sender, completion) = watch::channel(None);
                tokio::spawn(finish_shutdown(slots, handles, timeout, sender));
                *shutdown = Some(completion.clone());
                completion
            }
        };

        loop {
            if let Some(report) = completion.borrow().clone() {
                return report;
            }
            completion
                .changed()
                .await
                .expect("observer shutdown task retains its completion sender");
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
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use async_trait::async_trait;
    use everruns_core::event_emitter::EventEmitter;
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    use everruns_core::events::EventData;
    use everruns_core::events::OutputMessageDeltaData;
    use everruns_core::events::{EventContext, EventRequest, TurnStartedData};
    use everruns_host::{HostBackends, HostEventEmitter, InMemoryEventLog};
    use everruns_provider::tool_types::ToolCall;
    use everruns_provider::typed_id::EventId;
    use everruns_provider::typed_id::{MessageId, SessionId, TurnId};
    use serde_json::json;
    use tokio::sync::Notify;

    #[cfg(any(feature = "otel", feature = "braintrust"))]
    use super::{CoreEvent, CoreEventListener};
    use super::{
        EventFilter, EventListener, ListenerRegistration, OBSERVER_QUEUE_CAPACITY,
        ObserverDispatcher, reserve_retained_bytes,
    };
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

        /// A recorder whose filter matches no event this suite dispatches.
        fn none() -> Self {
            Self {
                events: Arc::new(Mutex::new(Vec::new())),
                flushed: Arc::new(AtomicBool::new(false)),
                filter: EventFilter::event_types(["never.dispatched"]),
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

    /// An unfiltered host listener, for asserting that the core half is still
    /// cloned when something consumes it.
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    struct CoreRecorder;

    #[cfg(any(feature = "otel", feature = "braintrust"))]
    #[async_trait]
    impl CoreEventListener for CoreRecorder {
        async fn on_event(&self, _event: &CoreEvent) {}

        fn name(&self) -> &'static str {
            "core-recorder"
        }
    }

    /// Build a core `output.message.delta` carrying a large `accumulated` prefix.
    fn output_delta_with_accumulated(accumulated: &str) -> everruns_core::events::Event {
        let turn_id = TurnId::new();
        let message_id = MessageId::new();
        EventRequest::new(
            SessionId::new(),
            EventContext::turn(turn_id, message_id),
            OutputMessageDeltaData {
                turn_id,
                message_id,
                delta: "x".to_string(),
                accumulated: accumulated.to_string(),
                phase: None,
            },
        )
        .into_event(EventId::new(), 1)
    }

    /// An app-only dispatcher must not clone the `CoreEvent` at all: nobody is
    /// there to read it, and it is the half that carries the growing
    /// `accumulated` prefix (TM-DOS-037).
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    #[test]
    fn app_only_dispatch_does_not_clone_the_core_event() {
        let accumulated = "x".repeat(1024 * 1024);
        let core = output_delta_with_accumulated(&accumulated);
        let facade = SessionEvent::from_core_event(&core);

        let dispatcher = ObserverDispatcher::new(
            vec![ListenerRegistration::App(Arc::new(Recorder::all()))],
            OBSERVER_QUEUE_CAPACITY,
        );
        let event = dispatcher
            .build_event(&core, &facade)
            .expect("app listener accepts output deltas");

        assert!(
            event.core.is_none(),
            "an app-only dispatch retained the raw core event"
        );

        // The source event still carries the prefix; only the shared payload drops it.
        let EventData::OutputMessageDelta(core_delta) = &core.data else {
            panic!("core event is not an output delta")
        };
        assert_eq!(core_delta.accumulated, accumulated);
        assert!(event.facade.data().get("accumulated").is_none());
        assert!(
            event.facade.canonical_json()["data"]
                .get("accumulated")
                .is_none()
        );
    }

    /// A host listener still gets its core half, so the laziness above cannot be
    /// implemented by simply never cloning.
    #[cfg(any(feature = "otel", feature = "braintrust"))]
    #[test]
    fn a_host_listener_still_receives_the_core_event() {
        let core = output_delta_with_accumulated("prefix");
        let facade = SessionEvent::from_core_event(&core);

        let dispatcher = ObserverDispatcher::new(
            vec![ListenerRegistration::Host(Arc::new(CoreRecorder))],
            OBSERVER_QUEUE_CAPACITY,
        );
        let event = dispatcher
            .build_event(&core, &facade)
            .expect("an unfiltered host listener accepts every event");

        assert!(
            event.core.is_some(),
            "a host listener was handed a payload with no core event"
        );
    }

    /// An event no listener matched is never built, so a filtered-out event costs
    /// no clone of either half.
    #[test]
    fn an_unmatched_event_builds_no_payload() {
        let core = output_delta_with_accumulated("prefix");
        let facade = SessionEvent::from_core_event(&core);

        let dispatcher = ObserverDispatcher::new(
            vec![ListenerRegistration::App(Arc::new(Recorder::none()))],
            OBSERVER_QUEUE_CAPACITY,
        );

        assert!(
            dispatcher.build_event(&core, &facade).is_none(),
            "a dispatch nothing matched still built a payload"
        );
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

    #[test]
    fn growing_prefixes_cannot_exceed_listener_byte_budget() {
        let retained = AtomicUsize::new(0);
        let capacity = 1024;
        let mut dropped = 0;

        for prefix_bytes in (64..=2048).step_by(64) {
            if !reserve_retained_bytes(&retained, prefix_bytes, capacity) {
                dropped += 1;
            }
        }

        assert!(dropped > 0);
        assert!(retained.load(Ordering::Acquire) <= capacity);
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

    #[tokio::test]
    async fn fresh_engine_attach_and_resume_delivers_only_new_events() {
        let backends = HostBackends::in_memory();
        let build_agent = || {
            Agent::builder()
                .instructions("Reply deterministically.")
                .model(Model::simulated("done"))
                .backends(backends.clone())
                .build()
                .expect("valid agent")
        };
        let original = Engine::new();
        let original_session = original.create(build_agent());
        let session_id = original_session.session_id();
        original_session
            .run("before attach")
            .await
            .expect("original turn runs");
        drop(original_session);
        drop(original);

        let recorder = Recorder::all();
        let engine = Engine::builder().listener(recorder.clone()).build();
        engine
            .attach(session_id, build_agent())
            .await
            .expect("persisted session attaches");
        engine
            .resume(session_id)
            .await
            .expect("attached session resumes")
            .run("after attach")
            .await
            .expect("resumed turn runs");
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
            1,
            "attaching and resuming must not replay the original turn"
        );
        assert!(
            events
                .iter()
                .all(|event| event.session_id == session_id.to_string())
        );
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

    #[tokio::test]
    async fn concurrent_shutdown_callers_wait_for_the_same_result() {
        let entered = Arc::new(Notify::new());
        let release = Arc::new(Notify::new());
        let engine = Engine::builder()
            .listener(SlowListener {
                blocked: AtomicBool::new(false),
                entered: entered.clone(),
                release: release.clone(),
            })
            .build();
        let session = engine.create(simple_agent());
        let run = tokio::spawn(async move { session.run("hello").await });
        entered.notified().await;
        run.await
            .expect("turn task remains healthy")
            .expect("turn runs");

        let first_engine = engine.clone();
        let first =
            tokio::spawn(async move { first_engine.shutdown(Duration::from_secs(5)).await });
        let second = tokio::spawn(async move { engine.shutdown(Duration::from_secs(5)).await });
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert!(!first.is_finished());
        assert!(!second.is_finished());

        release.notify_waiters();
        let first_report = first.await.expect("first shutdown task remains healthy");
        let second_report = second.await.expect("second shutdown task remains healthy");
        assert_eq!(first_report, second_report);
        assert!(first_report.stats.listeners[0].flushed);
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
            vec![ListenerRegistration::App(Arc::new(PendingListener {
                entered: entered.clone(),
                cancelled: cancelled.clone(),
            }))],
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
    async fn tool_spawned_session_reaches_engine_listener_not_parent_stream() {
        let recorder = Recorder::all();
        let engine = Engine::builder().listener(recorder.clone()).build();
        let spawned_ids = Arc::new(Mutex::new(Vec::new()));
        let tool_engine = engine.clone();
        let tool_spawned_ids = spawned_ids.clone();
        let spawn = FunctionTool::new(
            "spawn_child",
            "Run a child session.",
            json!({"type": "object", "properties": {}}),
            move |_arguments: serde_json::Value| {
                let engine = tool_engine.clone();
                let spawned_ids = tool_spawned_ids.clone();
                async move {
                    let child = engine.create(simple_agent());
                    let child_id = child.session_id();
                    child
                        .run("child turn")
                        .await
                        .map_err(|error| format!("child turn failed: {error:?}"))?;
                    spawned_ids
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(child_id);
                    Ok::<_, String>(json!({"session_id": child_id.to_string()}))
                }
            },
        );
        let parent = Agent::builder()
            .instructions("Spawn one child.")
            .model(Model::simulated_scripted(
                "parent done",
                vec![
                    vec![ToolCall {
                        id: "call_spawn_child".to_string(),
                        name: "spawn_child".to_string(),
                        arguments: json!({}),
                    }],
                    vec![],
                ],
            ))
            .tool(spawn)
            .build()
            .expect("valid parent agent");
        let parent_session = engine.create(parent);
        let parent_id = parent_session.session_id();
        let mut parent_stream = parent_session.events();
        parent_session.run("spawn").await.expect("parent turn runs");
        engine.shutdown(Duration::from_secs(5)).await;

        let child_id = spawned_ids.lock().unwrap()[0];
        let events = recorder.events.lock().unwrap();
        assert!(events.iter().any(|event| {
            event.session_id == child_id.to_string() && event.event_type() == "turn.started"
        }));
        while let Some(event) = parent_stream
            .try_recv()
            .expect("parent stream remains healthy")
        {
            assert_eq!(event.session_id, parent_id.to_string());
        }
    }
}
