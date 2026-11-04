// Write-behind for the lifecycle events a phase emits before its real work.
//
// Why: a reason phase stored `reason.started`, `capability.usage`, and
// `output.message.started` one after another before it sent the model request,
// and an act phase stored `act.started` and each `tool.started` before running
// the tool. Every store is a control-plane round trip (tens of milliseconds in
// production), and nothing on that path reads what the store returns.
//
// The input step queues `session.activated`, then waits for `turn.started`.
// Its authority response carries the real trace anchor needed before tools
// can create children or tasks; this also drains the preceding activation.
//
// Decision: those events are queued and stored in the background, in order,
// and the caller gets the event back at once (the same synthetic event the
// ephemeral path returns). Any other event waits for the queue to drain before
// it is sent, so the session's event order is unchanged, and the blocking
// events that end every phase (`reason.completed`, `tool.completed`) flush the
// queue before the phase completes.
//
// Decision: a queued event is never lost. A background store that fails is
// retried with backoff for a bounded window (`STORE_RETRY_WINDOW`), while it
// is retryable: not a refused fenced append (the actor lease is lost, another
// holder owns the log) and not a deleted session. Past that, the queue fails:
// the failed batch and everything queued after it are dropped unsent, and every
// later emit and `stored` return the failure, so the turn stops and nothing is
// appended past the gap. On a session actor lease the turn then lets the lease
// expire and the next holder resumes from the log, as after a crash just
// before the store. Logging and dropping a failed batch let a turn carry on
// past a hole, and the journal got `turn.completed` without `turn.started`
// (chaos harness: a control plane pool timeout on the batch holding
// `session.activated` and `turn.started`).
//
// A retried store whose earlier attempt actually landed (a transport error
// after the control plane committed) stores its events twice. The control
// plane's own failures roll back the insert, so that takes a lost response,
// and a repeated lifecycle start event is recoverable where a missing one is
// not.
//
// Decision: queued events that pile up while an earlier store is in flight go
// out together in one batched store (one RPC, one insert on the control plane)
// instead of one round trip each. A store takes every event queued before it
// ran, so later stores in the chain may find nothing left and do nothing.
//
// Decision: streamed `output.message.delta` and `llm.generation` are queued
// too: nothing reads what their stores return, and a failed store was only
// logged. A reason phase used to end with three blocking stores in a row
// (final delta, generation, `output.message.completed`).
//
// Decision: a blocking emit takes the events still waiting in the queue and
// stores them together with its own event, in one round trip and one insert,
// and gets its own event back as stored. So the final delta and the
// generation ride along with `output.message.completed`. Events whose store
// has already started stay with that store; the blocking emit waits for it
// first, so order is unchanged.
//
// Decision: `output.message.started` carrying reasoning state is not queued. It
// is persisted before the provider call so an interrupted worker can resume,
// and its caller fails the phase when that store fails.
//
// Decision: a queued store waits `COALESCE_WINDOW` before taking the queue,
// so the events a step emits a few tens of milliseconds apart share one
// insert and one commit instead of one each. A 2026-10-29 statement log of a
// four-tool turn showed ~34 event commits, every one holding a single event:
// `reason.started` and `output.message.started` land ~25 ms apart, `act.started`
// and `tool.started` ~20 ms apart, and `llm.generation` ~100 ms before the
// blocking `output.message.completed` that could carry it. At hundreds of
// concurrent turns those commits queue on the database's WAL flush. The cost
// is that a live viewer sees a queued event up to the window later. Waiting
// never delays the turn: a blocking emit takes what waits (see above), and
// `flush` cuts every waiting window short.
//
// Decision: the `tool.started` of an `AtMostOnce` call is not queued. It is
// stored before the tool runs, so a log without it proves the call never ran
// and resume may run it instead of settling it as interrupted; the act does
// not run the call when that store fails. Safe calls (pure, idempotent) keep
// the queued start: resume runs them again either way.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::actor_lease_store::{ActorLeaseLost, actor_lease_lost, is_actor_lease_lost};
use crate::core::{Event, EventData, EventRequest};
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::tool_types::SideEffectClass;
use everruns_contracts::typed_id::{EventId, SessionId};
use std::future::Future;
use std::pin::Pin;
use tokio::sync::watch;

type Task = Pin<Box<dyn Future<Output = ()> + Send>>;
/// One attempt to store a batch of queued events.
pub type Store = Pin<Box<dyn Future<Output = Result<()>> + Send>>;

/// How long a failing background store is retried before the queue fails.
const STORE_RETRY_WINDOW: Duration = Duration::from_secs(5);
const STORE_RETRY_FIRST_BACKOFF: Duration = Duration::from_millis(100);
const STORE_RETRY_MAX_BACKOFF: Duration = Duration::from_secs(1);
/// How long a queued store waits for more events before taking the queue.
const COALESCE_WINDOW: Duration = Duration::from_millis(150);

/// Why the queue stopped storing; see the module notes.
#[derive(Debug, Clone)]
enum Failure {
    LeaseLost(SessionId, String),
    SessionDeleted(SessionId),
    Store(String),
}

impl Failure {
    fn of(error: &AgentLoopError) -> Self {
        if let AgentLoopError::SessionNotFound(session_id) = error {
            return Self::SessionDeleted(*session_id);
        }
        if let AgentLoopError::Internal(inner) = error
            && let Some(lost) = inner.downcast_ref::<ActorLeaseLost>()
        {
            return Self::LeaseLost(lost.session_id, lost.detail.clone());
        }
        Self::Store(error.to_string())
    }

    fn error(&self) -> AgentLoopError {
        match self {
            Self::LeaseLost(session_id, detail) => actor_lease_lost(*session_id, detail.clone()),
            Self::SessionDeleted(session_id) => AgentLoopError::session_not_found(*session_id),
            Self::Store(error) => {
                AgentLoopError::store(format!("queued events were not stored: {error}"))
            }
        }
    }
}

/// Whether a failed store may be tried again.
fn retryable(error: &AgentLoopError) -> bool {
    !is_actor_lease_lost(error) && !matches!(error, AgentLoopError::SessionNotFound(_))
}

const QUEUED: &[&str] = &[
    crate::core::events::SESSION_ACTIVATED,
    crate::core::events::REASON_STARTED,
    crate::core::events::CAPABILITY_USAGE,
    crate::core::events::OUTPUT_MESSAGE_STARTED,
    crate::core::events::OUTPUT_MESSAGE_DELTA,
    crate::core::events::LLM_GENERATION,
    crate::core::events::ACT_STARTED,
    crate::core::events::TOOL_STARTED,
];

/// Ordered background queue for one phase's lifecycle events.
#[derive(Clone)]
pub struct WriteBehind {
    /// Turns true once every store queued so far has finished.
    tail: Arc<Mutex<watch::Receiver<bool>>>,
    /// Events queued by [`WriteBehind::enqueue_event`] and not yet taken by a
    /// store.
    pending: Arc<Mutex<Vec<EventRequest>>>,
    /// Set once a store gave up; nothing is stored after it.
    failed: Arc<Mutex<Option<Failure>>>,
    /// Signalled by [`WriteBehind::flush`]: waiting stores stop waiting.
    hurry: Arc<watch::Sender<()>>,
    retry_window: Duration,
    coalesce_window: Duration,
}

impl Default for WriteBehind {
    fn default() -> Self {
        Self::new()
    }
}

impl WriteBehind {
    pub fn new() -> Self {
        Self {
            tail: Arc::new(Mutex::new(watch::channel(true).1)),
            pending: Arc::new(Mutex::new(Vec::new())),
            failed: Arc::new(Mutex::new(None)),
            hurry: Arc::new(watch::channel(()).0),
            retry_window: STORE_RETRY_WINDOW,
            coalesce_window: COALESCE_WINDOW,
        }
    }

    #[cfg(test)]
    fn with_coalesce_window(mut self, window: Duration) -> Self {
        self.coalesce_window = window;
        self
    }

    #[cfg(test)]
    fn with_retry_window(mut self, window: Duration) -> Self {
        self.retry_window = window;
        self
    }

    /// The failure that stopped the queue, as the error every later emit
    /// returns; `None` while every store succeeded.
    pub fn failure(&self) -> Option<AgentLoopError> {
        lock(&self.failed).as_ref().map(Failure::error)
    }

    /// Stop the queue on `error`: drop what waits unsent, store nothing more.
    fn fail(&self, error: &AgentLoopError) {
        lock(&self.failed).get_or_insert_with(|| Failure::of(error));
        drop(self.take_pending());
    }

    /// Whether `request` may be stored in the background.
    pub fn queues(request: &EventRequest) -> bool {
        if let EventData::OutputMessageStarted(data) = &request.data
            && data.reasoning_state.is_some()
        {
            return false;
        }
        if let EventData::ToolStarted(data) = &request.data
            && data.side_effect_class == Some(SideEffectClass::AtMostOnce)
        {
            return false;
        }
        QUEUED.contains(&request.event_type.as_str())
    }

    /// Run `store` after every store queued before it.
    pub fn enqueue(&self, store: Task) {
        let (done, next) = watch::channel(false);
        let previous = std::mem::replace(&mut *self.lock(), next);
        tokio::spawn(async move {
            finished(previous).await;
            store.await;
            let _ = done.send(true);
        });
    }

    /// Queue `request` and store it, together with every event queued after
    /// it before the store starts, by calling `store` with the batch (again,
    /// while a failure is retryable; see the module notes). Refused once the
    /// queue failed: the event would land past the gap.
    pub fn enqueue_event<F>(&self, request: EventRequest, store: F) -> Result<()>
    where
        F: Fn(Vec<EventRequest>) -> Store + Send + 'static,
    {
        if let Some(error) = self.failure() {
            return Err(error);
        }
        lock(&self.pending).push(request);
        let queue = self.clone();
        // Subscribed now, so a flush after this point ends the wait below.
        let mut hurry = self.hurry.subscribe();
        self.enqueue(Box::pin(async move {
            // Let the step's next events join this store (see the module
            // notes), unless an earlier store already took them all.
            if lock(&queue.pending).is_empty() {
                return;
            }
            if queue.coalesce_window.is_zero() {
                tokio::task::yield_now().await;
            } else {
                tokio::select! {
                    () = tokio::time::sleep(queue.coalesce_window) => {}
                    _ = hurry.changed() => {}
                }
            }
            // An earlier store gave up: these would land past its gap.
            if queue.failure().is_some() {
                drop(queue.take_pending());
                return;
            }
            let batch = queue.take_pending();
            if batch.is_empty() {
                return;
            }
            let deadline = tokio::time::Instant::now() + queue.retry_window;
            let mut backoff = STORE_RETRY_FIRST_BACKOFF;
            loop {
                let error = match store(batch.clone()).await {
                    Ok(()) => return,
                    Err(error) => error,
                };
                if !retryable(&error) || tokio::time::Instant::now() + backoff > deadline {
                    tracing::warn!(count = batch.len(), error = %error, "queued event store failed; the turn stops");
                    queue.fail(&error);
                    return;
                }
                tracing::warn!(count = batch.len(), error = %error, "queued event store failed; retrying");
                tokio::time::sleep(backoff).await;
                backoff = (backoff * 2).min(STORE_RETRY_MAX_BACKOFF);
            }
        }));
        Ok(())
    }

    /// Take the queued events no store has taken yet. The caller stores them
    /// itself, after [`WriteBehind::flush`], ahead of its own event.
    pub fn take_pending(&self) -> Vec<EventRequest> {
        std::mem::take(&mut *lock(&self.pending))
    }

    /// Wait until every queued store has finished.
    pub async fn flush(&self) {
        self.hurry.send_replace(());
        let tail = self.lock().clone();
        finished(tail).await;
    }

    /// Wait until every queued store has finished, and fail when one gave up.
    pub async fn stored(&self) -> Result<()> {
        self.flush().await;
        self.failure().map_or(Ok(()), Err)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, watch::Receiver<bool>> {
        lock(&self.tail)
    }
}

/// The guarded values are replaced or drained whole, so a poisoned lock still
/// holds a valid one.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Wait for a store to finish. A store whose task ended without reporting
/// (it panicked) counts as finished, so the queue never wedges.
async fn finished(mut store: watch::Receiver<bool>) {
    let _ = store.wait_for(|done| *done).await;
}

/// The event a queued emit returns. The stored copy gets its id and sequence
/// from the control plane.
pub fn provisional_event(request: &EventRequest) -> Event {
    Event {
        id: EventId::new(),
        event_type: request.event_type.clone(),
        ts: request.ts,
        session_id: request.session_id,
        context: request.context.clone(),
        data: request.data.clone(),
        metadata: request.metadata.clone(),
        tags: request.tags.clone(),
        sequence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test]
    async fn queued_stores_run_in_order_and_flush_waits_for_them() {
        let queue = WriteBehind::new();
        let order = Arc::new(Mutex::new(Vec::new()));
        let (open, opened) = tokio::sync::oneshot::channel::<()>();
        let first = order.clone();
        queue.enqueue(Box::pin(async move {
            opened.await.ok();
            first.lock().unwrap().push(1);
        }));
        let second = order.clone();
        queue.enqueue(Box::pin(async move { second.lock().unwrap().push(2) }));

        // The second store waits behind the first, which is still blocked.
        tokio::task::yield_now().await;
        assert!(order.lock().unwrap().is_empty());

        let flushed = Arc::new(AtomicBool::new(false));
        let done = flushed.clone();
        let flush = queue.clone();
        let waiter = tokio::spawn(async move {
            flush.flush().await;
            done.store(true, Ordering::SeqCst);
        });
        tokio::task::yield_now().await;
        assert!(!flushed.load(Ordering::SeqCst), "flush waits for the queue");

        open.send(()).ok();
        waiter.await.unwrap();
        assert_eq!(*order.lock().unwrap(), vec![1, 2]);
    }

    fn request(event_type: &str) -> EventRequest {
        use crate::core::{EventContext, OutputMessageStartedData};
        use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};
        let mut request = EventRequest::new(
            SessionId::new(),
            EventContext::empty(),
            OutputMessageStartedData {
                reasoning_state: None,
                turn_id: TurnId::new(),
                message_id: MessageId::new(),
                model: None,
                iteration: None,
                phase: None,
            },
        );
        request.event_type = event_type.to_string();
        request
    }

    type Batches = Arc<Mutex<Vec<Vec<String>>>>;

    /// A store that records each batch's event types, first waiting for
    /// `gate` when given one.
    fn recording(
        batches: &Batches,
        gate: Option<tokio::sync::oneshot::Receiver<()>>,
    ) -> impl Fn(Vec<EventRequest>) -> Store + Send + 'static {
        let batches = batches.clone();
        let gate = Arc::new(Mutex::new(gate));
        move |batch| {
            let batches = batches.clone();
            let gate = gate.lock().unwrap().take();
            Box::pin(async move {
                if let Some(gate) = gate {
                    gate.await.ok();
                }
                let types = batch.into_iter().map(|r| r.event_type).collect();
                batches.lock().unwrap().push(types);
                Ok(())
            })
        }
    }

    /// A store that fails its first `failures` attempts with `error`, then
    /// records like [`recording`].
    fn failing(
        batches: &Batches,
        failures: usize,
        error: impl Fn() -> AgentLoopError + Send + Sync + 'static,
    ) -> impl Fn(Vec<EventRequest>) -> Store + Send + 'static {
        let batches = batches.clone();
        let error = Arc::new(error);
        let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        move |batch| {
            let batches = batches.clone();
            let attempt = attempts.fetch_add(1, Ordering::SeqCst);
            let failure = (attempt < failures).then(|| error());
            Box::pin(async move {
                if let Some(error) = failure {
                    return Err(error);
                }
                let types = batch.into_iter().map(|r| r.event_type).collect();
                batches.lock().unwrap().push(types);
                Ok(())
            })
        }
    }

    fn pool_timeout() -> AgentLoopError {
        AgentLoopError::store("Failed to store events: pool timed out")
    }

    // Chaos harness: a pool timeout on the batch holding `session.activated`
    // and `turn.started` was logged and dropped, and the turn went on to
    // store `turn.completed` without them.
    #[tokio::test(start_paused = true)]
    async fn a_transient_store_failure_is_retried_until_it_lands() {
        let queue = WriteBehind::new();
        let batches = Batches::default();
        queue
            .enqueue_event(request("turn.started"), failing(&batches, 2, pool_timeout))
            .unwrap();
        queue.stored().await.expect("the retried store landed");
        assert_eq!(
            *batches.lock().unwrap(),
            vec![vec!["turn.started".to_string()]]
        );
        assert!(queue.failure().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn a_store_that_keeps_failing_fails_the_queue_and_nothing_lands_after_it() {
        let queue = WriteBehind::new().with_retry_window(Duration::from_secs(2));
        let batches = Batches::default();
        queue
            .enqueue_event(
                request("turn.started"),
                failing(&batches, usize::MAX, pool_timeout),
            )
            .unwrap();
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        // Queued while the first store is still retrying: behind the gap.
        queue
            .enqueue_event(request("reason.started"), recording(&batches, None))
            .unwrap();
        let error = queue.stored().await.expect_err("the queue failed");
        assert!(error.to_string().contains("pool timed out"), "{error}");
        assert!(
            !error.is_non_retryable(),
            "the next holder resumes the turn"
        );
        assert!(batches.lock().unwrap().is_empty(), "nothing past the gap");
        assert!(
            queue
                .enqueue_event(request("act.started"), recording(&batches, None))
                .is_err(),
            "later events are refused"
        );
        assert!(queue.take_pending().is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn a_refused_fenced_append_fails_the_queue_at_once_as_a_lost_lease() {
        let queue = WriteBehind::new();
        let batches = Batches::default();
        let session_id = SessionId::new();
        queue
            .enqueue_event(
                request("turn.started"),
                failing(&batches, 1, move || {
                    actor_lease_lost(session_id, "fence moved")
                }),
            )
            .unwrap();
        let started = tokio::time::Instant::now();
        let error = queue.stored().await.expect_err("the lease is lost");
        assert!(is_actor_lease_lost(&error), "{error}");
        assert_eq!(started.elapsed(), Duration::ZERO, "no retry");
        assert!(batches.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn events_queued_behind_a_store_in_flight_go_out_as_one_batch() {
        let queue = WriteBehind::new().with_coalesce_window(Duration::ZERO);
        let batches = Batches::default();
        let (open, opened) = tokio::sync::oneshot::channel::<()>();
        let mut opened = Some(opened);
        for event_type in ["a", "b", "c"] {
            queue
                .enqueue_event(request(event_type), recording(&batches, opened.take()))
                .unwrap();
            // The first store has started and holds its batch.
            tokio::task::yield_now().await;
            tokio::task::yield_now().await;
        }
        open.send(()).ok();
        queue.flush().await;
        assert_eq!(
            *batches.lock().unwrap(),
            vec![
                vec!["a".to_string()],
                vec!["b".to_string(), "c".to_string()]
            ],
            "events queued while the first store ran share the next store"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn events_a_step_emits_within_the_window_share_one_store() {
        let queue = WriteBehind::new();
        let batches = Batches::default();
        queue
            .enqueue_event(request("act.started"), recording(&batches, None))
            .unwrap();
        tokio::time::sleep(Duration::from_millis(20)).await;
        queue
            .enqueue_event(request("tool.started"), recording(&batches, None))
            .unwrap();
        tokio::time::sleep(COALESCE_WINDOW).await;
        queue
            .enqueue_event(request("tool.started"), recording(&batches, None))
            .unwrap();
        queue.flush().await;
        assert_eq!(
            *batches.lock().unwrap(),
            vec![
                vec!["act.started".to_string(), "tool.started".to_string()],
                vec!["tool.started".to_string()]
            ],
            "events inside the window share a store; a later one starts the next"
        );
    }

    #[tokio::test(start_paused = true)]
    async fn flush_cuts_the_window_short() {
        let queue = WriteBehind::new();
        let batches = Batches::default();
        queue
            .enqueue_event(request("reason.started"), recording(&batches, None))
            .unwrap();
        let started = tokio::time::Instant::now();
        queue.flush().await;
        assert!(
            started.elapsed() < COALESCE_WINDOW,
            "the end of a phase does not wait out the window: {:?}",
            started.elapsed()
        );
        assert_eq!(
            *batches.lock().unwrap(),
            vec![vec!["reason.started".to_string()]]
        );
    }

    #[tokio::test]
    async fn a_blocking_emit_takes_waiting_events_and_leaves_started_stores_alone() {
        let queue = WriteBehind::new().with_coalesce_window(Duration::ZERO);
        let batches = Arc::new(Mutex::new(Vec::<Vec<String>>::new()));
        let (open, opened) = tokio::sync::oneshot::channel::<()>();
        // "a" is taken by a store that is still running.
        queue
            .enqueue_event(request("a"), recording(&batches, Some(opened)))
            .unwrap();
        tokio::task::yield_now().await;
        tokio::task::yield_now().await;
        // "b" and "c" wait; the blocking emit takes them.
        queue
            .enqueue_event(request("b"), recording(&batches, None))
            .unwrap();
        queue
            .enqueue_event(request("c"), recording(&batches, None))
            .unwrap();
        let taken: Vec<String> = queue
            .take_pending()
            .into_iter()
            .map(|r| r.event_type)
            .collect();
        assert_eq!(taken, vec!["b".to_string(), "c".to_string()]);

        open.send(()).ok();
        queue.flush().await;
        assert_eq!(
            *batches.lock().unwrap(),
            vec![vec!["a".to_string()]],
            "the started store keeps its event and the later stores find nothing"
        );
    }

    #[test]
    fn only_lifecycle_starts_without_reasoning_state_are_queued() {
        use crate::core::{EventContext, OutputMessageStartedData};
        use everruns_contracts::typed_id::{MessageId, SessionId, TurnId};

        let started = |reasoning_state| {
            EventRequest::new(
                SessionId::new(),
                EventContext::empty(),
                OutputMessageStartedData {
                    reasoning_state,
                    turn_id: TurnId::new(),
                    message_id: MessageId::new(),
                    model: None,
                    iteration: None,
                    phase: None,
                },
            )
        };
        assert!(WriteBehind::queues(&started(None)));
        let state = everruns_contracts::reasoning_updates::ReasoningState {
            epoch: "epoch".to_string(),
            baseline: None,
            effective: None,
            pending: None,
        };
        assert!(!WriteBehind::queues(&started(Some(state))));

        // A turn's start events ride the same queue, ahead of its reason.
        let mut turn_started = started(None);
        turn_started.event_type = crate::core::events::TURN_STARTED.to_string();
        assert!(!WriteBehind::queues(&turn_started));
        turn_started.event_type = crate::core::events::SESSION_ACTIVATED.to_string();
        assert!(WriteBehind::queues(&turn_started));

        let mut completed = started(None);
        completed.event_type = crate::core::events::OUTPUT_MESSAGE_COMPLETED.to_string();
        assert!(!WriteBehind::queues(&completed));
    }

    #[test]
    fn at_most_once_tool_starts_are_stored_before_the_tool_runs() {
        use crate::core::{EventContext, ToolStartedData};
        use everruns_contracts::tool_types::ToolCall;
        use everruns_contracts::typed_id::SessionId;

        let started = |side_effect_class| {
            EventRequest::new(
                SessionId::new(),
                EventContext::empty(),
                ToolStartedData {
                    tool_call: ToolCall {
                        id: "call_1".to_string(),
                        name: "send_email".to_string(),
                        arguments: serde_json::json!({}),
                    },
                    tool_call_fingerprint: None,
                    display_name: None,
                    narration: None,
                    side_effect_class,
                },
            )
        };
        assert!(!WriteBehind::queues(&started(Some(
            SideEffectClass::AtMostOnce
        ))));
        assert!(WriteBehind::queues(&started(Some(SideEffectClass::Pure))));
        assert!(WriteBehind::queues(&started(Some(
            SideEffectClass::Idempotent
        ))));
        // An unknown tool runs nothing; its start may be queued.
        assert!(WriteBehind::queues(&started(None)));
    }
}
