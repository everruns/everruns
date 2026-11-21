use crate::{CompactionObservation, SessionEvent};

/// Assess a terminal compaction event without model calls or side effects.
///
/// A pass reports the operation's metadata, not independent durability or
/// semantic fidelity. Hosts own consent and budgets for content assessment.
pub fn observe_compaction(event: &SessionEvent) -> Option<CompactionObservation> {
    if !matches!(
        event.event_type(),
        "context.compacted" | "context.compaction.failed" | "context.compaction.skipped"
    ) {
        return None;
    }
    let data = everruns_core::events::deserialize_event_data(
        event.event_type(),
        event.canonical_json().get("data")?.clone(),
    );
    everruns_core::compaction_observation::observe_compaction(&data)
}
