// Observer matching listener: taps turn.completed, tool.completed and
// session.idled, and decides what to score.
//
// Design: the listener does only the cheap part on the event path: look up
// active observers for the org, match, sample and enqueue pending
// trace_scores (matching.rs, shared with backfill). It never runs a judge or
// blocks the turn. The background worker (worker.rs) drains the queue. A
// shared Notify wakes the worker immediately after enqueue; the worker also
// polls for catch-up, which is what picks up delayed session scores.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use everruns_core::EventListener;
use everruns_core::events::{Event, EventData, SESSION_IDLED, TOOL_COMPLETED, TURN_COMPLETED};
use tracing::{error, instrument};

use crate::domains::observers::matching::{SessionFacts, Slice, plan_scores};
use crate::storage::StorageBackend;

pub struct ObserverMatchListener {
    db: Arc<StorageBackend>,
    /// Wakes the scoring worker after enqueue for low-latency scoring.
    wake: Arc<tokio::sync::Notify>,
}

impl ObserverMatchListener {
    pub fn new(db: Arc<StorageBackend>, wake: Arc<tokio::sync::Notify>) -> Self {
        Self { db, wake }
    }

    async fn handle_slice(&self, event: &Event, slice: Slice) -> anyhow::Result<()> {
        // The org and its active observers come from memory; most orgs have
        // none, so the session row (tags, harness) is read only to match.
        let Some(lineage) = self.db.session_lineage(event.session_id).await? else {
            return Ok(());
        };
        let observers = self.db.cached_active_observers(lineage.org_id).await?;
        if observers.is_empty() {
            return Ok(());
        }
        let Some(session) = self.db.get_session_unscoped(event.session_id).await? else {
            return Ok(());
        };
        let facts = SessionFacts {
            session_id: event.session_id.uuid(),
            agent_id: session.agent_id.map(|a| a.uuid()),
            harness_id: session.harness_id.map(|h| h.uuid()),
            tags: &session.tags,
        };
        let enqueue = plan_scores(&observers, &facts, &slice, true, Utc::now());
        if enqueue.is_empty() {
            return Ok(());
        }
        let inserted = self
            .db
            .enqueue_trace_scores(session.org_id, &enqueue)
            .await?;
        // Session scores wait out their settle delay; the worker's poll
        // claims them, so only immediate scores wake it.
        if inserted > 0 && !matches!(slice, Slice::Session) {
            self.wake.notify_one();
        }
        Ok(())
    }
}

#[async_trait]
impl EventListener for ObserverMatchListener {
    #[instrument(skip(self, event), fields(event_id = %event.id, session_id = %event.session_id))]
    async fn on_event(&self, event: &Event) {
        let slice = match &event.data {
            EventData::TurnCompleted(data) => Slice::Turn {
                turn_id: data.turn_id.to_string(),
            },
            EventData::ToolCompleted(data) => Slice::Tool {
                call_id: data.tool_call_id.clone(),
                tool: data.tool_name.clone(),
            },
            EventData::SessionIdled(_) => Slice::Session,
            _ => return,
        };
        if let Err(e) = self.handle_slice(event, slice).await {
            error!(error = %e, "Observer matching failed");
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![TURN_COMPLETED, TOOL_COMPLETED, SESSION_IDLED])
    }

    fn name(&self) -> &'static str {
        "ObserverMatchListener"
    }
}
