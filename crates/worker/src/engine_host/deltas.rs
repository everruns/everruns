//! A platform session's live deltas on the Engine: the token stream of a
//! running turn (`output.message.delta`, `tool.output.delta`, thinking
//! deltas), which the Engine never stores and hands to its epoch instead.
//!
//! Decisions:
//! - Deltas reach the control plane the way a worker turn's do, as fenced
//!   event writes, so the platform's live streams show text as the model
//!   writes it. Taking them off the control plane's store is the cell's
//!   stream node (data plane v2 "Live streaming"), not this step.
//! - Publishing never waits on the control plane: one bounded queue per
//!   epoch, drained in order by one task, in batches. A full queue drops the
//!   delta; the committed event that ends the stream carries the whole text.
//! - A refused write (the lease was lost) drops its batch: the next holder
//!   streams its own attempt.

use std::sync::{Arc, Mutex};

use everruns::hosting::{LiveDelta, LiveDeltaSink};
use tokio::sync::mpsc;
use tracing::debug;

use super::backends::EventStamp;
use crate::actor_lease_store::ActorFence;
use crate::core::events::EventRequest;
use crate::worker_adapters::WorkerAdapters;

/// Deltas one epoch holds before it drops new ones.
const QUEUE: usize = 1024;
/// Deltas per control plane write.
const BATCH: usize = 64;

/// Forwards an epoch's deltas to the control plane.
pub struct PlatformDeltas {
    queue: mpsc::Sender<EventRequest>,
    stamp: EventStamp,
}

impl PlatformDeltas {
    /// Start forwarding under the epoch's `fence`. The forwarding task ends
    /// once the sink is dropped and its queue drained.
    pub fn start<A: WorkerAdapters>(
        adapters: A,
        fence: Arc<Mutex<ActorFence>>,
        stamp: EventStamp,
    ) -> Arc<Self> {
        let (queue, mut deltas) = mpsc::channel::<EventRequest>(QUEUE);
        tokio::spawn(async move {
            while let Some(first) = deltas.recv().await {
                let mut batch = vec![first];
                while batch.len() < BATCH {
                    match deltas.try_recv() {
                        Ok(delta) => batch.push(delta),
                        Err(_) => break,
                    }
                }
                let fence = fence.lock().unwrap_or_else(|p| p.into_inner()).clone();
                if let Err(error) = adapters.emit_events_fenced(batch, &fence).await {
                    debug!(session_id = %fence.session_id, error = %error,
                        "Live deltas not stored; the stream's committed event still is");
                }
            }
        });
        Arc::new(Self { queue, stamp })
    }
}

impl LiveDeltaSink for PlatformDeltas {
    fn publish(&self, delta: LiveDelta) {
        let event = delta.event;
        let mut request = EventRequest {
            event_type: event.event_type,
            ts: event.ts,
            session_id: event.session_id,
            context: event.context,
            data: event.data,
            metadata: event.metadata,
            tags: event.tags,
        };
        self.stamp.apply(&mut request);
        let _ = self.queue.try_send(request);
    }
}
