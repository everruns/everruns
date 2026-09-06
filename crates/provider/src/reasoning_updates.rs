//! Conversation-scoped Astra configuration, distinct from the request baseline.

use crate::model::ReasoningEffort;
use serde::{Deserialize, Serialize};

/// Persisted with assistant messages and native compaction checkpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReasoningState {
    /// A new epoch starts when switching model/provider or entering this mode.
    #[cfg_attr(
        feature = "openapi",
        schema(example = "01933b5a-0000-7000-8000-000000000001")
    )]
    pub epoch: String,
    /// Original request-level effort preserved throughout this epoch.
    #[cfg_attr(feature = "openapi", schema(example = "low"))]
    pub baseline: Option<ReasoningEffort>,
    /// Effort currently active after ordered configuration updates.
    #[cfg_attr(feature = "openapi", schema(example = "high"))]
    pub effective: Option<ReasoningEffort>,
    /// Pending transition for this request; completed-message snapshots omit it.
    #[serde(skip)]
    pub pending: Option<ReasoningEffort>,
}

impl ReasoningState {
    pub fn is_supported(&self) -> bool {
        [self.baseline, self.effective, self.pending]
            .into_iter()
            .flatten()
            .all(supported_update_effort)
    }
}

/// Configuration updates are an Astra standard single-agent Responses feature.
/// Everruns does not request native pro or multi-agent mode on this path.
pub fn supports_configuration_updates(model: &str) -> bool {
    model == "gpt-6-astra"
}

pub fn supported_update_effort(effort: ReasoningEffort) -> bool {
    matches!(
        effort,
        ReasoningEffort::Low
            | ReasoningEffort::Medium
            | ReasoningEffort::High
            | ReasoningEffort::Xhigh
            | ReasoningEffort::Max
    )
}
