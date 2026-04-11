//! DuckDuckGo Integration (Experimental)
//!
//! Instant answers via DuckDuckGo Instant Answer API.
//! Provides `duckduckgo_search` tool for agents to get instant answers,
//! abstracts, related topics, and definitions.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: No API key required — DuckDuckGo Instant Answer API is free
//! Decision: Stateless — no per-resource state management needed

pub mod client;
mod tools;

use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin};
use everruns_core::tools::Tool;

use tools::DuckDuckGoSearchTool;

// ============================================================================
// Integration Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        feature_flag: None,
        factory: || Box::new(DuckDuckGoCapability),
    }
}

// ============================================================================
// Constants
// ============================================================================

const DUCKDUCKGO_API_BASE: &str = "https://api.duckduckgo.com";

// ============================================================================
// DuckDuckGoCapability
// ============================================================================

pub struct DuckDuckGoCapability;

impl Capability for DuckDuckGoCapability {
    fn id(&self) -> &str {
        "duckduckgo"
    }

    fn name(&self) -> &str {
        "[Experimental] DuckDuckGo"
    }

    fn description(&self) -> &str {
        "Search for instant answers using DuckDuckGo Instant Answer API. \
         Agents can get abstracts, definitions, related topics, and direct answers. \
         EXPERIMENTAL: This capability may change."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("search")
    }

    fn category(&self) -> Option<&str> {
        Some("Network")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## DuckDuckGo (Experimental)

Instant answers via DuckDuckGo Instant Answer API. Use this to get quick facts, definitions, abstracts (from Wikipedia and other sources), and related topics.

No API key required — the DuckDuckGo Instant Answer API is free.

Tools:
- `duckduckgo_search` - Get instant answers, abstracts, and related topics for a query

Note: This returns curated instant answers, not full web search results. For comprehensive web search, use Brave Search if available."#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(DuckDuckGoSearchTool)]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_core::capabilities::CapabilityStatus;

    #[test]
    fn test_capability_metadata() {
        let cap = DuckDuckGoCapability;
        assert_eq!(cap.id(), "duckduckgo");
        assert_eq!(cap.name(), "[Experimental] DuckDuckGo");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("search"));
        assert_eq!(cap.category(), Some("Network"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = DuckDuckGoCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 1);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"duckduckgo_search"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DuckDuckGoCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("duckduckgo_search"));
        assert!(prompt.contains("DuckDuckGo"));
        assert!(prompt.contains("Experimental"));
    }

    #[test]
    fn test_capability_no_dependencies() {
        let cap = DuckDuckGoCapability;
        assert!(cap.dependencies().is_empty());
    }
}
