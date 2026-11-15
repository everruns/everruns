use super::*;
use std::{collections::HashMap, sync::Mutex};
/// Application result sink. Called on the Engine listener queue, never inside execution.
#[async_trait::async_trait]
pub trait ScoreSink: Send + Sync {
    /// Persist or export an online grade. `Err` records bounded-evidence or grader failure.
    async fn record(&self, session: &str, turn: &str, result: Result<Vec<Score>, Error>);
}
/// Bounded online scoring listener. Install with `Engine::builder().listener(...)`.
///
/// At most 64 turns and 1 MiB per turn are retained. Overflows produce an error;
/// interrupted turns are discarded during flush. Engine listener drop statistics
/// must be monitored: online grades are observational, never an audit ledger.
#[derive(Clone)]
pub struct OnlineScoring {
    evaluator: Evaluator,
    sink: Arc<dyn ScoreSink>,
    turns: Arc<Mutex<HashMap<(String, String), Evidence>>>,
}
impl OnlineScoring {
    /// Construct a listener using the same evaluator as offline cases.
    pub fn new(evaluator: Evaluator, sink: impl ScoreSink + 'static) -> Self {
        Self {
            evaluator,
            sink: Arc::new(sink),
            turns: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}
#[async_trait::async_trait]
impl crate::EventListener for OnlineScoring {
    fn name(&self) -> &'static str {
        "OnlineScoring"
    }
    fn filter(&self) -> crate::EventFilter {
        crate::EventFilter::event_types([
            "turn.started",
            "input.message",
            "output.message.completed",
            "llm.generation",
            "tool.started",
            "tool.completed",
            "turn.completed",
            "turn.failed",
            "turn.cancelled",
        ])
    }
    async fn on_event(&self, event: &SessionEvent) {
        let Some(turn) = event.turn_id.clone() else {
            return;
        };
        let key = (event.session_id.clone(), turn.clone());
        let (complete, overflow) = {
            let Ok(mut turns) = self.turns.lock() else {
                return;
            };
            if event.event_type() == "turn.started" {
                if turns.len() >= 64 {
                    (None, true)
                } else {
                    let evidence = turns.entry(key.clone()).or_default();
                    evidence.apply(event);
                    (None, false)
                }
            } else if let Some(evidence) = turns.get_mut(&key) {
                evidence.apply(event);
                if evidence.retained_bytes() > 1024 * 1024 {
                    turns.remove(&key);
                    (None, true)
                } else if matches!(
                    event.event_type(),
                    "turn.completed" | "turn.failed" | "turn.cancelled"
                ) {
                    (turns.remove(&key), false)
                } else {
                    (None, false)
                }
            } else {
                (None, false)
            }
        };
        if overflow {
            self.sink
                .record(
                    &event.session_id,
                    &turn,
                    Err(Error("Online evidence retention limit exceeded".into())),
                )
                .await;
        }
        if let Some(evidence) = complete {
            // THREAT[TM-FW-006]: never grade a partial stream as complete evidence.
            let result = if event.event_type() == "turn.completed" && evidence.complete(event) {
                self.evaluator.score(&evidence).await
            } else {
                Err(Error(
                    "Turn did not complete successfully or its online evidence is incomplete"
                        .into(),
                ))
            };
            self.sink.record(&event.session_id, &turn, result).await;
        }
    }
    async fn flush(&self) {
        if let Ok(mut turns) = self.turns.lock() {
            turns.clear();
        }
    }
}
