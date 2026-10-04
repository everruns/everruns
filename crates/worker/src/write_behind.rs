// Write-behind for the lifecycle events a phase emits before its real work.
//
// Why: a reason phase stored `reason.started`, `capability.usage`, and
// `output.message.started` one after another before it sent the model request,
// and an act phase stored `act.started` and each `tool.started` before running
// the tool. Every store is a control-plane round trip (tens of milliseconds in
// production), and nothing on that path reads what the store returns.
//
// Decision: those events are queued and stored in the background, in order,
// and the caller gets the event back at once (the same synthetic event the
// ephemeral path returns). Any other event waits for the queue to drain before
// it is sent, so the session's event order is unchanged, and the blocking
// events that end every phase (`reason.completed`, `tool.completed`) flush the
// queue before the phase completes. A failed background store is logged, as
// these emits already were on failure.
//
// Decision: `output.message.started` carrying reasoning state is not queued. It
// is persisted before the provider call so an interrupted worker can resume,
// and its caller fails the phase when that store fails.

use std::sync::{Arc, Mutex};

use crate::core::{Event, EventData, EventRequest};
use everruns_contracts::typed_id::EventId;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::watch;

type Store = Pin<Box<dyn Future<Output = ()> + Send>>;

const QUEUED: &[&str] = &[
    crate::core::events::REASON_STARTED,
    crate::core::events::CAPABILITY_USAGE,
    crate::core::events::OUTPUT_MESSAGE_STARTED,
    crate::core::events::ACT_STARTED,
    crate::core::events::TOOL_STARTED,
];

/// Ordered background queue for one phase's lifecycle events.
#[derive(Clone)]
pub struct WriteBehind {
    /// Turns true once every store queued so far has finished.
    tail: Arc<Mutex<watch::Receiver<bool>>>,
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
        }
    }

    /// Whether `request` may be stored in the background.
    pub fn queues(request: &EventRequest) -> bool {
        if let EventData::OutputMessageStarted(data) = &request.data
            && data.reasoning_state.is_some()
        {
            return false;
        }
        QUEUED.contains(&request.event_type.as_str())
    }

    /// Run `store` after every store queued before it.
    pub fn enqueue(&self, store: Store) {
        let (done, next) = watch::channel(false);
        let previous = std::mem::replace(&mut *self.lock(), next);
        tokio::spawn(async move {
            finished(previous).await;
            store.await;
            let _ = done.send(true);
        });
    }

    /// Wait until every queued store has finished.
    pub async fn flush(&self) {
        let tail = self.lock().clone();
        finished(tail).await;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, watch::Receiver<bool>> {
        // The guarded receiver is replaced whole, so a poisoned lock still
        // holds a valid one.
        self.tail
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
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

        let mut completed = started(None);
        completed.event_type = crate::core::events::OUTPUT_MESSAGE_COMPLETED.to_string();
        assert!(!WriteBehind::queues(&completed));
    }
}
