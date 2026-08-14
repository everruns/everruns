//! Brave Search web search for Everruns agents.
//!
//! This integration contributes a `brave_web_search` tool and its connection
//! provider to the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_integrations_brave_search::BraveSearchCapability;
//!
//! let capability = BraveSearchCapability;
//! # let _ = capability;
//! ```

pub mod client;
pub mod connection;
mod tools;

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin,
};
use everruns_core::tools::Tool;
use everruns_platform::connector::ConnectorPlugin;

use connection::BraveSearchConnector;
use tools::BraveWebSearchTool;

// ============================================================================
// Plugin Registration
// ============================================================================

inventory::submit! {
    IntegrationPlugin {
        experimental_only: true,
        feature_flag: None,
        factory: || Box::new(BraveSearchCapability),
    }
}

inventory::submit! {
    ConnectorPlugin {
        experimental_only: true,
        factory: || Box::new(BraveSearchConnector),
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
            "`brave_web_search` performs current web search via Brave Search; use it for recent facts, research, and documentation lookups.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(BraveWebSearchTool)]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![]
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "[Експериментально] Brave Search",
            "Шукайте в інтернеті через Brave Search API. Агенти можуть виконувати пошукові \
             запити й отримувати релевантні результати із заголовками, URL-адресами та \
             описами. ЕКСПЕРИМЕНТАЛЬНО: ця можливість може змінитися.",
        )]
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
        assert!(prompt.contains("current web search"));
    }

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = BraveSearchCapability;
        let ctx = everruns_core::capabilities::SystemPromptContext::without_file_store(
            everruns_provider::typed_id::SessionId::new(),
        );
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(prompt.len() <= 250, "prompt is {} bytes", prompt.len());
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
