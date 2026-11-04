//! Runs event listeners in order per session, off the caller's path when it
//! asks.
//!
//! Decisions:
//! - Every listener notification goes through one queue per shard, and a
//!   session always maps to the same shard. A shard handles one event at a
//!   time and runs its listeners one after another, as `notify_listeners`
//!   did, so a session's listeners see its events in the order they were
//!   handed over, whichever caller handed them over.
//! - A session actor's holder (a fenced append) does not wait for its
//!   events' listeners: its call is answered once the events are stored and
//!   published for live delivery. Before, every blocking event of a turn
//!   (`tool.completed`, `turn.started`, ...) waited for the observer matcher,
//!   approval audit, MCP events, tracing and metrics in turn, several of them
//!   database queries, before the worker could continue. Every other caller
//!   still waits until its events' listeners ran, so API paths that read a
//!   listener's effect right after emitting keep working.
//! - Listeners still run in memory: a server that stops between storing an
//!   event and running its listeners loses that listener work, as before. The
//!   listener feed from the database replaces this
//!   (`knowledge/execution/event-listeners.md`).
//! - Queues are bounded. A full shard makes the next caller wait for room,
//!   so a slow listener slows its callers down instead of growing memory.
//! - A draining server waits (bounded) for queued listener work after its
//!   worker link stops (`app_builder::serve`), so a rolling deploy does not
//!   drop what fenced appends left queued.
//! - Each listener runs in its own task so a panic is contained, as before.
//!   One slow listener holds up the sessions of its shard; [`SHARDS`] keeps
//!   that share small.

use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use everruns_core::{Event, EventListener};
use tokio::sync::{mpsc, oneshot};

/// Queues per service. A session's events always land on the same one.
pub(super) const SHARDS: usize = 32;
/// Events one shard holds before callers wait for room.
const SHARD_CAPACITY: usize = 1024;

/// Listener jobs queued or running in this process, across services.
static PENDING: AtomicUsize = AtomicUsize::new(0);

/// Wait until no listener work is queued or running in this process, at most
/// `limit`. `false` when work was still pending at the limit.
pub async fn wait_for_queued_listeners(limit: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + limit;
    while PENDING.load(Ordering::SeqCst) > 0 {
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    true
}

enum Job {
    Notify {
        event: Box<Event>,
        listeners: Vec<Arc<dyn EventListener>>,
        done: Option<oneshot::Sender<()>>,
    },
    /// Answered once every job queued before it on the shard ran.
    Barrier(oneshot::Sender<()>),
}

/// Ordered, per-session listener queues. Cheap to clone; clones share queues.
#[derive(Clone)]
pub(super) struct ListenerDispatch {
    listeners: Arc<Vec<Arc<dyn EventListener>>>,
    shards: Arc<[OnceLock<mpsc::Sender<Job>>]>,
}

impl ListenerDispatch {
    pub(super) fn new(listeners: Arc<Vec<Arc<dyn EventListener>>>) -> Self {
        Self {
            listeners,
            shards: (0..SHARDS).map(|_| OnceLock::new()).collect(),
        }
    }

    /// Queue `event` for the listeners that want its type. The returned
    /// receiver resolves once they all ran; `None` when no listener wants it.
    pub(super) async fn dispatch(&self, event: &Event) -> Option<oneshot::Receiver<()>> {
        let wanted: Vec<Arc<dyn EventListener>> = self
            .listeners
            .iter()
            .filter(|listener| {
                listener
                    .event_types()
                    .is_none_or(|types| types.contains(&event.event_type.as_str()))
            })
            .cloned()
            .collect();
        if wanted.is_empty() {
            return None;
        }
        let (done, ran) = oneshot::channel();
        let job = Job::Notify {
            event: Box::new(event.clone()),
            listeners: wanted,
            done: Some(done),
        };
        PENDING.fetch_add(1, Ordering::SeqCst);
        if self.shard(shard_of(event)).send(job).await.is_err() {
            PENDING.fetch_sub(1, Ordering::SeqCst);
            tracing::error!(event_type = %event.event_type, "listener queue closed");
            return None;
        }
        Some(ran)
    }

    /// Wait until every event queued so far ran its listeners.
    pub(super) async fn settled(&self) {
        let mut pending = Vec::new();
        for (index, shard) in self.shards.iter().enumerate() {
            if shard.get().is_none() {
                continue;
            }
            let (tx, rx) = oneshot::channel();
            if self.shard(index).send(Job::Barrier(tx)).await.is_ok() {
                pending.push(rx);
            }
        }
        for rx in pending {
            let _ = rx.await;
        }
    }

    fn shard(&self, index: usize) -> &mpsc::Sender<Job> {
        self.shards[index].get_or_init(|| {
            let (tx, rx) = mpsc::channel(SHARD_CAPACITY);
            tokio::spawn(run_shard(rx));
            tx
        })
    }
}

fn shard_of(event: &Event) -> usize {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    event.session_id.uuid().hash(&mut hasher);
    (hasher.finish() % SHARDS as u64) as usize
}

async fn run_shard(mut jobs: mpsc::Receiver<Job>) {
    while let Some(job) = jobs.recv().await {
        match job {
            Job::Notify {
                event,
                listeners,
                mut done,
            } => {
                let event: Arc<Event> = Arc::from(event);
                for listener in listeners {
                    let name = listener.name();
                    let event = event.clone();
                    let handle = tokio::spawn(async move { listener.on_event(&event).await });
                    if let Err(e) = handle.await {
                        tracing::error!(
                            listener = name,
                            error = %e,
                            "EventListener panicked or was cancelled"
                        );
                    }
                }
                PENDING.fetch_sub(1, Ordering::SeqCst);
                if let Some(done) = done.take() {
                    let _ = done.send(());
                }
            }
            Job::Barrier(done) => {
                let _ = done.send(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::typed_id::SessionId;
    use everruns_core::RuntimeMessage;
    use everruns_core::events::{EventContext, EventData, OutputMessageCompletedData};
    use std::sync::Mutex;
    use std::time::Duration;
    use tokio::sync::Notify;

    fn event(session: SessionId, label: &str) -> Event {
        let data = EventData::OutputMessageCompleted(OutputMessageCompletedData::new(
            RuntimeMessage::assistant(label),
        ));
        Event {
            id: everruns_contracts::typed_id::EventId::new(),
            event_type: data.event_type().to_string(),
            ts: chrono::Utc::now(),
            session_id: session,
            context: EventContext::empty(),
            data,
            metadata: None,
            tags: Some(vec![label.to_string()]),
            sequence: None,
        }
    }

    fn label(event: &Event) -> String {
        event
            .tags
            .as_ref()
            .and_then(|t| t.first())
            .cloned()
            .unwrap_or_default()
    }

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<String>>,
    }

    #[async_trait::async_trait]
    impl EventListener for Recorder {
        async fn on_event(&self, event: &Event) {
            // Later events finish faster, so out-of-order handling would show.
            let delay = 20u64.saturating_sub(self.seen.lock().unwrap().len() as u64 * 5);
            tokio::time::sleep(Duration::from_millis(delay)).await;
            self.seen.lock().unwrap().push(label(event));
        }
    }

    #[tokio::test]
    async fn a_sessions_events_reach_listeners_in_order() {
        let recorder = Arc::new(Recorder::default());
        let dispatch = ListenerDispatch::new(Arc::new(vec![recorder.clone() as _]));
        let session = SessionId::new();
        for n in 0..4 {
            dispatch.dispatch(&event(session, &n.to_string())).await;
        }
        dispatch.settled().await;
        assert_eq!(*recorder.seen.lock().unwrap(), vec!["0", "1", "2", "3"]);
    }

    struct Gate {
        open: Notify,
    }

    #[async_trait::async_trait]
    impl EventListener for Gate {
        async fn on_event(&self, _event: &Event) {
            self.open.notified().await;
        }
    }

    #[tokio::test]
    async fn queueing_does_not_wait_for_the_listener() {
        let gate = Arc::new(Gate {
            open: Notify::new(),
        });
        let dispatch = ListenerDispatch::new(Arc::new(vec![gate.clone() as _]));
        let mut ran = tokio::time::timeout(
            Duration::from_secs(1),
            dispatch.dispatch(&event(SessionId::new(), "x")),
        )
        .await
        .expect("queueing returns while the listener is still running")
        .expect("the listener wants the event");
        assert!(ran.try_recv().is_err(), "the listener has not run yet");
        gate.open.notify_one();
        tokio::time::timeout(Duration::from_secs(1), ran)
            .await
            .unwrap()
            .unwrap();
    }

    struct Panicking;

    #[async_trait::async_trait]
    impl EventListener for Panicking {
        async fn on_event(&self, _event: &Event) {
            panic!("listener bug");
        }
    }

    #[tokio::test]
    async fn a_panicking_listener_does_not_stop_the_others_or_the_queue() {
        let recorder = Arc::new(Recorder::default());
        let dispatch = ListenerDispatch::new(Arc::new(vec![
            Arc::new(Panicking) as _,
            recorder.clone() as _,
        ]));
        let session = SessionId::new();
        dispatch.dispatch(&event(session, "a")).await;
        dispatch.dispatch(&event(session, "b")).await;
        dispatch.settled().await;
        assert_eq!(*recorder.seen.lock().unwrap(), vec!["a", "b"]);
    }

    struct OnlyTurns;

    #[async_trait::async_trait]
    impl EventListener for OnlyTurns {
        async fn on_event(&self, _event: &Event) {}
        fn event_types(&self) -> Option<Vec<&'static str>> {
            Some(vec!["turn.completed"])
        }
    }

    #[tokio::test]
    async fn events_no_listener_wants_are_not_queued() {
        let dispatch = ListenerDispatch::new(Arc::new(vec![Arc::new(OnlyTurns) as _]));
        assert!(
            dispatch
                .dispatch(&event(SessionId::new(), "x"))
                .await
                .is_none()
        );
    }
}
