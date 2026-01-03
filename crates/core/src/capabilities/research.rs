//! Research Capability - for deep research with organized findings (coming soon)

use super::{Capability, CapabilityId, CapabilityStatus};

/// Research capability - for deep research with organized findings (coming soon)
pub struct ResearchCapability;

impl Capability for ResearchCapability {
    fn id(&self) -> &str {
        CapabilityId::RESEARCH
    }

    fn name(&self) -> &str {
        "Deep Research"
    }

    fn description(&self) -> &str {
        "Enables deep research capabilities with a scratchpad for notes, web search tools, and structured thinking."
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
        Some("You have access to a research scratchpad. Use it to organize your thoughts and findings.")
    }
}
