//! Brave Search Integration (Experimental)
//!
//! Web search via Brave Search REST API.
//! Provides `brave_web_search` tool for agents to search the web.
//!
//! Decision: External integration crate, auto-registered via inventory plugin system
//! Decision: API key resolved via user connections (provider: "brave_search"), fallback to session secrets
//! Decision: Stateless — no per-resource state management needed

pub mod client;
mod tools;

use everruns_core::capabilities::{Capability, CapabilityStatus, IntegrationPlugin};
use everruns_core::tools::Tool;

use tools::BraveWebSearchTool;

// ============================================================================
// Integration Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        factory: || Box::new(BraveSearchCapability),
    }
}

// ============================================================================
// Constants
// ============================================================================

const BRAVE_SEARCH_API_BASE: &str = "https://api.search.brave.com/res/v1";
const BRAVE_SEARCH_API_KEY_SECRET: &str = "BRAVE_SEARCH_API_KEY";
const BRAVE_SEARCH_CONNECTION_PROVIDER: &str = "brave_search";

// ============================================================================
// BraveSearchCapability
// ============================================================================

pub struct BraveSearchCapability;

impl Capability for BraveSearchCapability {
    fn id(&self) -> &str {
        "brave_search"
    }

    fn name(&self) -> &str {
        "[Experimental] Brave Search"
    }

    fn description(&self) -> &str {
        "Search the web using Brave Search API. \
         Agents can query the web and get relevant results including titles, URLs, and descriptions. \
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
            r#"## Brave Search (Experimental)

Web search via Brave Search API. Use this to find current information, research topics, and look up documentation.

Prerequisite: Brave Search API key must be configured in Settings > Connections, or set as session secret `BRAVE_SEARCH_API_KEY`.

Tools:
- `brave_web_search` - Search the web and return relevant results

Get a free API key at https://brave.com/search/api/"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BraveWebSearchTool)]
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
        let cap = BraveSearchCapability;
        assert_eq!(cap.id(), "brave_search");
        assert_eq!(cap.name(), "[Experimental] Brave Search");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("search"));
        assert_eq!(cap.category(), Some("Network"));
    }

    #[test]
    fn test_capability_has_all_tools() {
        let cap = BraveSearchCapability;
        let tools = cap.tools();
        assert_eq!(tools.len(), 1);

        let names: Vec<&str> = tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"brave_web_search"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = BraveSearchCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("brave_web_search"));
        assert!(prompt.contains("Brave Search"));
        assert!(prompt.contains("Experimental"));
    }

    #[test]
    fn test_all_tools_require_context() {
        let cap = BraveSearchCapability;
        for tool in cap.tools() {
            assert!(
                tool.requires_context(),
                "Tool {} should require context",
                tool.name()
            );
        }
    }

    #[test]
    fn test_capability_no_dependencies() {
        let cap = BraveSearchCapability;
        assert!(cap.dependencies().is_empty());
    }
}
