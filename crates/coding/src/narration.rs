//! Transcript lines for the navigation tools.

use everruns_contracts::runtime::tool_narration::{ToolNarrationPhase, labeled_phrase};

/// Present-tense label for started and completed; explicit failure wording.
pub(crate) fn stable_labeled(
    verb: &str,
    detail: Option<String>,
    phase: ToolNarrationPhase,
) -> String {
    let failed = format!("Could not {}", verb.to_lowercase());
    labeled_phrase(verb, verb, &failed, detail, phase)
}
