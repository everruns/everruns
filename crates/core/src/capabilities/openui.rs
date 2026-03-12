//! OpenUI Capability — enables agents to generate rich interactive UI
//!
//! When enabled, this capability appends the OpenUI system prompt to the agent's
//! system prompt. The prompt instructs the LLM to respond with OpenUI Lang code
//! wrapped in ```openui fenced code blocks when visual content is requested.
//!
//! The UI detects these blocks and renders them using the @openuidev/react-lang
//! parser and @openuidev/react-ui component library.
//!
//! Ref: https://github.com/thesysdev/openui
//! Ref: specs/openui.md

use super::{Capability, CapabilityStatus};

/// Capability ID constant for external reference.
pub const OPENUI_CAPABILITY_ID: &str = "openui";

/// OpenUI capability — adds OpenUI Lang prompt and declares the "openui" feature.
pub struct OpenUiCapability;

impl Capability for OpenUiCapability {
    fn id(&self) -> &str {
        OPENUI_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "OpenUI"
    }

    fn description(&self) -> &str {
        "Enables the agent to generate rich interactive UI components (charts, tables, forms, cards, etc.) using OpenUI Lang."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("layout")
    }

    fn category(&self) -> Option<&str> {
        Some("UI")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(everruns_openui::default_prompt())
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["openui"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;

    #[test]
    fn test_capability_metadata() {
        let cap = OpenUiCapability;
        assert_eq!(cap.id(), OPENUI_CAPABILITY_ID);
        assert_eq!(cap.name(), "OpenUI");
        assert_eq!(cap.icon(), Some("layout"));
        assert_eq!(cap.category(), Some("UI"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn test_capability_has_no_tools() {
        let cap = OpenUiCapability;
        let tools = cap.tools();
        assert!(tools.is_empty());
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = OpenUiCapability;
        let prompt = cap.system_prompt_addition();
        assert!(prompt.is_some());
        let text = prompt.unwrap();
        assert!(text.contains("openui-lang"));
        assert!(text.contains("```openui"));
        assert!(text.contains("## Syntax Rules"));
        assert!(text.contains("## Component Signatures"));
    }

    #[test]
    fn test_capability_features() {
        let cap = OpenUiCapability;
        let features = cap.features();
        assert_eq!(features, vec!["openui"]);
    }

    #[test]
    fn test_capability_in_registry() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get(OPENUI_CAPABILITY_ID).unwrap();
        assert_eq!(cap.id(), OPENUI_CAPABILITY_ID);
        assert!(cap.system_prompt_addition().is_some());
    }

    #[test]
    fn test_system_prompt_has_all_component_groups() {
        let cap = OpenUiCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("### Content"));
        assert!(prompt.contains("### Tables"));
        assert!(prompt.contains("### Charts (2D)"));
        assert!(prompt.contains("### Charts (1D)"));
        assert!(prompt.contains("### Forms"));
        assert!(prompt.contains("### Buttons"));
        assert!(prompt.contains("### Layout"));
    }
}
