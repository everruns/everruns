//! An epoch's live deltas (the token stream of a running turn) to the
//! cell's streams. Split from `hosting.rs` for size.
//!
//! Decisions:
//! - A node's epoch publishes straight into the node's live channel, which
//!   its SSE streams merge with committed events; the gateway merges the
//!   streams of every node of the cell.
//! - An executor's epoch sends its deltas to the cell in batches
//!   (`/cell/exec/live`); the node takes them only while the epoch is the
//!   session's current one, so a superseded executor's stream stops.
//! - Publishing never waits: one bounded queue per epoch; a full queue
//!   drops the delta, and the committed event that ends its stream still
//!   carries the whole text.

use super::*;

use everruns::hosting::{LiveDelta, LiveDeltaSink};

/// Deltas one epoch queues before it drops new ones.
const QUEUE: usize = 1024;
/// Deltas per request to the cell.
const BATCH: usize = 64;

pub(super) fn sink(inner: Arc<Inner>, session: SessionId, fence: u64) -> Arc<dyn LiveDeltaSink> {
    match &inner.source {
        Source::Node(node) => Arc::new(NodeDeltas {
            node: node.clone(),
            session,
            fence,
        }),
        Source::Remote(remote) => RemoteDeltas::start(remote.clone(), session, fence),
    }
}

struct NodeDeltas {
    node: Weak<Node>,
    session: SessionId,
    fence: u64,
}

impl LiveDeltaSink for NodeDeltas {
    fn publish(&self, delta: LiveDelta) {
        if delta.session != self.session || delta.fence != self.fence {
            return;
        }
        if let Some(node) = self.node.upgrade() {
            node.publish_live(self.session, self.fence, vec![delta.event]);
        }
    }
}

struct RemoteDeltas {
    queue: tokio::sync::mpsc::Sender<everruns_core::events::Event>,
    session: SessionId,
    fence: u64,
}

impl RemoteDeltas {
    fn start(remote: Arc<crate::remote::Remote>, session: SessionId, fence: u64) -> Arc<Self> {
        let (queue, mut deltas) = tokio::sync::mpsc::channel(QUEUE);
        tokio::spawn(async move {
            while let Some(first) = deltas.recv().await {
                let mut events = vec![first];
                while events.len() < BATCH {
                    match deltas.try_recv() {
                        Ok(event) => events.push(event),
                        Err(_) => break,
                    }
                }
                let batch = crate::remote::LiveBatch {
                    session,
                    fence,
                    events,
                };
                remote.live(&batch).await;
            }
        });
        Arc::new(Self {
            queue,
            session,
            fence,
        })
    }
}

impl LiveDeltaSink for RemoteDeltas {
    fn publish(&self, delta: LiveDelta) {
        if delta.session != self.session || delta.fence != self.fence {
            return;
        }
        let _ = self.queue.try_send(delta.event);
    }
}
