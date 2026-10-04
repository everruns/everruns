//! Research Capability - for deep research with organized findings (coming soon)

use super::{Capability, CapabilityLocalization, CapabilityStatus};

pub const RESEARCH_CAPABILITY_ID: &str = "research";

/// Research capability - for deep research with organized findings (coming soon)
pub struct ResearchCapability;

impl Capability for ResearchCapability {
    fn id(&self) -> &str {
        RESEARCH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Deep Research"
    }

    fn description(&self) -> &str {
        "Enables deep research capabilities with a scratchpad for notes, web search tools, and structured thinking."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Глибоке дослідження",
            "Вмикає можливості глибокого дослідження з нотатником для записів, інструментами пошуку в інтернеті та структурованим мисленням.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::ComingSoon
    }

    fn icon(&self) -> Option<&str> {
        Some("search")
    }

    fn category(&self) -> Option<&str> {
        Some("AI")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            "You have access to a research scratchpad. Use it to organize your thoughts and findings.",
        )
    }
}
