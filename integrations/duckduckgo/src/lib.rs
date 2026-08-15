//! DuckDuckGo Instant Answer integration for Everruns.
//!
//! Instant answers via DuckDuckGo Instant Answer API.
//! Provides `duckduckgo_instant_answer` tool for agents to get instant answers,
//! abstracts, related topics, and definitions. This is an instant-answer lookup,
//! not a full web/SERP search.
//!
//! This crate is part of the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_core::capabilities::Capability;
//! use everruns_integrations_duckduckgo::DuckDuckGoCapability;
//!
//! let capability = DuckDuckGoCapability;
//! assert_eq!(capability.id(), "duckduckgo");
//! assert_eq!(capability.tools().len(), 1);
//! ```

pub mod client;
mod tools;

use everruns_core::capabilities::{
    Capability, CapabilityLocalization, CapabilityStatus, IntegrationPlugin,
};
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
pub const DUCKDUCKGO_CAPABILITY_ID: &str = "duckduckgo";

/// Activate DuckDuckGo instant answers with its default configuration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DuckDuckGo;

impl everruns_capability::IntoCapability for DuckDuckGo {
    fn into_capability(self) -> everruns_capability::CapabilitySpec {
        everruns_capability::CapabilityRef::new(DUCKDUCKGO_CAPABILITY_ID).into()
    }
}

// ============================================================================
// DuckDuckGoCapability
// ============================================================================

pub struct DuckDuckGoCapability;

impl Capability for DuckDuckGoCapability {
    fn id(&self) -> &str {
        DUCKDUCKGO_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "[Experimental] DuckDuckGo"
    }

    fn description(&self) -> &str {
        "Look up instant answers using the DuckDuckGo Instant Answer API. \
         Agents can get abstracts, definitions, related topics, and direct answers. \
         This is an instant-answer lookup, not a full web/SERP search — no result does \
         not mean no web pages exist. \
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
        // Behavioral note only: the tool description covers what
        // `duckduckgo_instant_answer` does. The model needs to know it is an
        // instant-answer API (curated facts/abstracts), not a general web
        // search — and that an empty result does not mean no web pages exist.
        Some(
            "`duckduckgo_instant_answer` returns curated instant answers (facts, definitions, abstracts), not general web results. An empty result does not mean no matching web pages exist; prefer a web-search or web-fetch tool for web discovery, but this tool can serve as a lightweight fallback when none is available.",
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(DuckDuckGoSearchTool)]
    }

    fn dependencies(&self) -> Vec<&'static str> {
        vec![]
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "[Експериментально] DuckDuckGo",
            "Шукайте миттєві відповіді через DuckDuckGo Instant Answer API. Агенти можуть \
             отримувати анотації, визначення, пов'язані теми та прямі відповіді. \
             Це пошук миттєвих відповідей, а не повноцінний веб-пошук — відсутність \
             результату не означає, що немає відповідних веб-сторінок. \
             ЕКСПЕРИМЕНТАЛЬНО: ця можливість може змінитися.",
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
        assert!(names.contains(&"duckduckgo_instant_answer"));
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = DuckDuckGoCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        // Tool name and behavioral distinction (instant answers, not full
        // web search) are the only things the prompt needs to convey —
        // the rest lives in the tool description.
        assert!(prompt.contains("duckduckgo_instant_answer"));
        assert!(prompt.contains("instant answers"));
    }

    #[test]
    fn test_capability_no_dependencies() {
        let cap = DuckDuckGoCapability;
        assert!(cap.dependencies().is_empty());
    }
}
