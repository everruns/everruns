// Observer matching listener — taps turn.completed, decides what to score.
//
// Design: the listener does only the cheap part on the event path — look up
// active observers for the org, evaluate match predicates, sample, and
// enqueue pending trace_scores. It never runs a judge or blocks the turn.
// The background worker (worker.rs) drains the queue. A shared Notify wakes
// the worker immediately after enqueue; the worker also polls for catch-up.

use std::sync::Arc;

use async_trait::async_trait;
use everruns_core::EventListener;
use everruns_core::events::{Event, EventData, TURN_COMPLETED};
use everruns_core::typed_id::TraceScoreId;
use everruns_platform::observer::{ObserverMatch, ObserverScope, ObserverScorerConfig};
use tracing::{error, instrument};
use uuid::Uuid;

use crate::storage::StorageBackend;
use crate::storage::models::CreateTraceScoreRow;

/// Tag marking synthetic eval-run sessions, which observers must never score.
const EVAL_SESSION_TAG: &str = "eval";

pub struct ObserverMatchListener {
    db: Arc<StorageBackend>,
    /// Wakes the scoring worker after enqueue for low-latency scoring.
    wake: Arc<tokio::sync::Notify>,
}

impl ObserverMatchListener {
    pub fn new(db: Arc<StorageBackend>, wake: Arc<tokio::sync::Notify>) -> Self {
        Self { db, wake }
    }

    async fn handle_turn_completed(&self, event: &Event, turn_id: String) -> anyhow::Result<()> {
        let session_id = event.session_id;

        // Need org + session attributes to match. One unscoped lookup.
        let Some(session) = self.db.get_session_unscoped(session_id).await? else {
            return Ok(());
        };

        // Never score synthetic eval sessions.
        if session.tags.iter().any(|t| t == EVAL_SESSION_TAG) {
            return Ok(());
        }

        let observers = self.db.list_active_observers(session.org_id).await?;
        if observers.is_empty() {
            return Ok(());
        }

        let mut enqueue = Vec::new();
        for observer in observers {
            let match_config: ObserverMatch = match serde_json::from_value(observer.match_config) {
                Ok(m) => m,
                Err(e) => {
                    error!(observer_id = %observer.public_id, error = %e, "Invalid observer match config");
                    continue;
                }
            };
            if !match_config.matches(session.agent_id, session.harness_id, &session.tags) {
                continue;
            }
            // Sample per (turn, observer) after matching.
            if !sample(observer.sampling_rate) {
                continue;
            }
            let scorers: Vec<ObserverScorerConfig> = match serde_json::from_value(observer.scorers)
            {
                Ok(s) => s,
                Err(e) => {
                    error!(observer_id = %observer.public_id, error = %e, "Invalid observer scorers");
                    continue;
                }
            };
            for scorer in scorers {
                // Phase 1 implements turn scope only.
                if scorer.scope != ObserverScope::Turn {
                    continue;
                }
                enqueue.push(CreateTraceScoreRow {
                    public_id: TraceScoreId::from_uuid(Uuid::now_v7()).to_string(),
                    observer_id: observer.id,
                    scorer_key: scorer.key,
                    session_id: session_id.uuid(),
                    turn_id: turn_id.clone(),
                    agent_id: session.agent_id.map(|a| a.uuid()),
                    agent_version_id: session.agent_version_id.map(|a| a.uuid()),
                    harness_id: session.harness_id.map(|h| h.uuid()),
                });
            }
        }

        if enqueue.is_empty() {
            return Ok(());
        }

        let inserted = self
            .db
            .enqueue_trace_scores(session.org_id, &enqueue)
            .await?;
        if inserted > 0 {
            self.wake.notify_one();
        }
        Ok(())
    }
}

#[async_trait]
impl EventListener for ObserverMatchListener {
    #[instrument(skip(self, event), fields(event_id = %event.id, session_id = %event.session_id))]
    async fn on_event(&self, event: &Event) {
        let EventData::TurnCompleted(data) = &event.data else {
            return;
        };
        let turn_id = data.turn_id.to_string();
        if let Err(e) = self.handle_turn_completed(event, turn_id).await {
            error!(error = %e, "Observer matching failed");
        }
    }

    fn event_types(&self) -> Option<Vec<&'static str>> {
        Some(vec![TURN_COMPLETED])
    }

    fn name(&self) -> &'static str {
        "ObserverMatchListener"
    }
}

/// Bernoulli sample. `rate >= 1.0` always scores; `rate <= 0.0` never does.
fn sample(rate: f64) -> bool {
    if rate >= 1.0 {
        return true;
    }
    if rate <= 0.0 {
        return false;
    }
    rand::random::<f64>() < rate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sample_boundaries() {
        assert!(sample(1.0));
        assert!(sample(2.0));
        assert!(!sample(0.0));
        assert!(!sample(-1.0));
    }

    #[test]
    fn sample_full_rate_always_true() {
        for _ in 0..100 {
            assert!(sample(1.0));
        }
    }
}
