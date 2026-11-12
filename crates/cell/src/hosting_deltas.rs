//! An epoch's live deltas (the token stream of a running turn) to the
//! session's stream node (`stream.rs`). Split from `hosting.rs` for size.
//!
//! Decisions:
//! - Every epoch, on a node or an executor, sends its deltas to the
//!   session's stream node, which the sender finds by hashing the session
//!   over the cell's view (no lookup). A node that is the stream node
//!   itself takes them in memory.
//! - Batching: one request in flight per epoch. Deltas produced while one
//!   is in flight go together in the next, so a quiet stream sends at once
//!   (first-delta latency) and a busy one batches itself.
//! - The stream node takes them only while the epoch is the session's
//!   current one, so a superseded executor's stream stops.
//! - Publishing never waits: one bounded queue per epoch; a full queue
//!   drops the delta, and the committed event that ends its stream still
//!   carries the whole text.

use super::*;

use everruns::hosting::{LiveDelta, LiveDeltaSink};

use crate::stream::{LiveBatch, LiveItem};

/// Deltas one epoch queues before it drops new ones.
const QUEUE: usize = 1024;
/// Deltas per request to the stream node.
const BATCH: usize = 256;

pub(super) fn sink(inner: Arc<Inner>, session: SessionId, fence: u64) -> Arc<dyn LiveDeltaSink> {
    let (queue, mut deltas) = tokio::sync::mpsc::channel::<LiveItem>(QUEUE);
    let source = match &inner.source {
        Source::Node(node) => Target::Node(node.clone()),
        Source::Remote(remote) => Target::Remote(remote.clone()),
    };
    tokio::spawn(async move {
        while let Some(first) = deltas.recv().await {
            let mut batch = vec![first];
            while batch.len() < BATCH {
                match deltas.try_recv() {
                    Ok(item) => batch.push(item),
                    Err(_) => break,
                }
            }
            let batch = LiveBatch {
                session,
                fence,
                view: 0,
                deltas: batch,
            };
            match &source {
                Target::Node(node) => match node.upgrade() {
                    Some(node) => node.deliver_live(batch).await,
                    None => return,
                },
                Target::Remote(remote) => remote.live(batch).await,
            }
        }
    });
    Arc::new(Deltas {
        queue,
        session,
        fence,
    })
}

enum Target {
    Node(Weak<Node>),
    Remote(Arc<crate::remote::Remote>),
}

struct Deltas {
    queue: tokio::sync::mpsc::Sender<LiveItem>,
    session: SessionId,
    fence: u64,
}

impl LiveDeltaSink for Deltas {
    fn publish(&self, delta: LiveDelta) {
        if delta.session != self.session || delta.fence != self.fence {
            return;
        }
        let _ = self.queue.try_send(LiveItem {
            stream: delta.attempt.stream,
            seq: delta.seq,
            event: delta.event,
        });
    }
}

impl Node {
    /// Deltas of an epoch this node runs: taken here when this node is the
    /// session's stream node, sent to it otherwise.
    pub async fn deliver_live(&self, batch: LiveBatch) {
        if self.check_stream_owner(batch.session, None).is_ok() {
            self.publish_live(batch.session, batch.fence, batch.deltas)
                .await;
            return;
        }
        let seeds: Vec<String> = self.members().into_values().collect();
        self.deliver
            .send(&self.http, self.cfg.cell, &seeds, batch)
            .await;
    }
}
