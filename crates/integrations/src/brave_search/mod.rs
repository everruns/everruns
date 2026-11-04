#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Brave Search web search for Everruns agents.
//!
//! This integration contributes a `brave_web_search` tool and its connection
//! provider to the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_contracts::capability::IntoCapability;
//! use everruns_integrations::brave_search::BraveSearch;
//!
//! let capability = BraveSearch::new("your-api-key").into_capability();
//! # let _ = capability;
//! ```

pub mod client;
#[cfg(feature = "brave-search-hosted")]
pub mod connection;
mod framework;
mod search;
mod tools;

pub use framework::BraveSearch;
pub use search::SearchInput;

#[cfg(feature = "brave-search-hosted")]
use everruns_contracts::connector::ConnectorPlugin;
#[cfg(feature = "brave-search-hosted")]
use everruns_contracts::runtime::capabilities::IntegrationPlugin;
use everruns_contracts::runtime::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus,
};
use everruns_contracts::runtime::tools::Tool;

#[cfg(feature = "brave-search-hosted")]
use connection::BraveSearchConnector;
use tools::BraveWebSearchTool;

// ============================================================================
// Plugin Registration
// ============================================================================

/// Capability plugins this crate contributes to a hosted catalog.
#[cfg(feature = "brave-search-hosted")]
pub const CAPABILITY_PLUGINS: &[IntegrationPlugin] = &[IntegrationPlugin {
    feature_flag: None,
    factory: || Box::new(BraveSearchCapability),
}];

/// Connector plugins this crate contributes to a hosted catalog.
#[cfg(feature = "brave-search-hosted")]
pub const CONNECTOR_PLUGINS: &[ConnectorPlugin] = &[ConnectorPlugin {
    feature_flag: None,
    factory: || Box::new(BraveSearchConnector),
}];
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
         EXPERIMENTAL: This capability may change. \
         For provider-neutral web search, use Web search."
    }

    fn web_search_source(&self, _config: &serde_json::Value) -> bool {
        true
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
             описами. ЕКСПЕРИМЕНТАЛЬНО: ця можливість може змінитися. Для пошуку без \
             прив'язки до провайдера використовуйте Вебпошук.",
        )]
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn system_prompt_within_budget() {
        let cap = BraveSearchCapability;
        let ctx =
            everruns_contracts::runtime::capabilities::SystemPromptContext::without_file_store(
                everruns_contracts::typed_id::SessionId::new(),
            );
        let prompt = cap.system_prompt_contribution(&ctx).await.unwrap();
        assert!(prompt.len() <= 250, "prompt is {} bytes", prompt.len());
    }
}
