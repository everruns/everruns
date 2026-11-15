//! Portable turn/step traces over canonical session events. Stability: alpha.
use crate::SessionEvent;
/// A model, tool, approval or delegation step.
pub use everruns_contracts::trace::TraceStepRow as Step;
/// A turn in the trace.
pub use everruns_contracts::trace::TraceTurnRow as Turn;
/// A projected snapshot. Full prompts and full tool results are never retained here.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Trace {
    /// Turns in execution order.
    pub turns: Vec<Turn>,
    /// Steps in turn/step order.
    pub steps: Vec<Step>,
}
/// Invalid or incomplete canonical input; callers must supply the complete prefix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TraceError(pub String);
impl std::fmt::Display for TraceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for TraceError {}
impl Trace {
    /// Project a complete sequence-ordered session prefix using the Server's shared fold.
    /// Rejects duplicate/out-of-order events and mixed sessions. Events irrelevant to traces are ignored.
    pub fn project<'a>(
        events: impl IntoIterator<Item = &'a SessionEvent>,
    ) -> Result<Self, TraceError> {
        let mut set = everruns_core::session_trace::TraceWorkingSet::default();
        let mut sequence = 0i32;
        let mut session = None;
        for event in events {
            let raw: everruns_core::Event = serde_json::from_value(event.canonical_json().clone())
                .map_err(|_| TraceError("Malformed canonical event".into()))?;
            let current = raw
                .sequence
                .ok_or_else(|| TraceError("Trace events require stored sequences".into()))?;
            if Some(current) != sequence.checked_add(1) {
                return Err(TraceError(
                    "Trace events must be a complete contiguous prefix in sequence order".into(),
                ));
            }
            sequence = current;
            if session.is_some_and(|id| id != raw.session_id) {
                return Err(TraceError(
                    "Trace events belong to different sessions".into(),
                ));
            }
            session = Some(raw.session_id);
            if let Some(event) = everruns_core::session_trace::trace_event(&raw) {
                set.apply(&event)
            }
        }
        let (_, mut turns, mut steps) = set.into_changes();
        turns.sort_by_key(|t| t.turn_no);
        steps.sort_by_key(|s| (s.turn_no, s.step_no));
        Ok(Self { turns, steps })
    }
}
