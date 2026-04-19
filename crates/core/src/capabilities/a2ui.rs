//! A2UI Capability — enables agents to emit Google A2UI JSON component trees
//!
//! When enabled, this capability appends the A2UI system prompt to the agent's
//! system prompt. The LLM wraps JSON component trees in ```a2ui fenced code
//! blocks, which the UI detects and renders using native shadcn/ui primitives.
//!
//! Runs in parallel with `openui`. Both may be enabled on the same agent, but
//! that is rarely useful — instruct the agent to prefer one protocol.
//!
//! Ref: specs/a2ui.md
//! Ref: https://github.com/google/a2ui

use super::{Capability, CapabilityStatus};

/// Capability ID constant for external reference.
pub const A2UI_CAPABILITY_ID: &str = "a2ui";

/// A2UI capability — adds the A2UI catalog prompt and declares the `a2ui` feature.
pub struct A2UiCapability;

impl Capability for A2UiCapability {
    fn id(&self) -> &str {
        A2UI_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "A2UI"
    }

    fn description(&self) -> &str {
        "Enables the agent to emit generative UI as Google A2UI JSON component trees, rendered by the UI with native design-system components."
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("layout-grid")
    }

    fn category(&self) -> Option<&str> {
        Some("UI")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(everruns_a2ui::default_prompt())
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["a2ui"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::CapabilityRegistry;

    #[test]
    fn capability_metadata() {
        let cap = A2UiCapability;
        assert_eq!(cap.id(), A2UI_CAPABILITY_ID);
        assert_eq!(cap.name(), "A2UI");
        assert_eq!(cap.category(), Some("UI"));
        assert_eq!(cap.status(), CapabilityStatus::Available);
    }

    #[test]
    fn capability_has_no_tools() {
        let cap = A2UiCapability;
        assert!(cap.tools().is_empty());
    }

    #[test]
    fn capability_has_system_prompt() {
        let cap = A2UiCapability;
        let prompt = cap.system_prompt_addition().expect("prompt present");
        assert!(prompt.contains("```a2ui"));
        assert!(prompt.contains("## Schema"));
        assert!(prompt.contains("## Catalog"));
        assert!(prompt.contains("## Actions"));
    }

    #[test]
    fn capability_features() {
        let cap = A2UiCapability;
        assert_eq!(cap.features(), vec!["a2ui"]);
    }

    #[test]
    fn capability_in_registry() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get(A2UI_CAPABILITY_ID).expect("registered");
        assert_eq!(cap.id(), A2UI_CAPABILITY_ID);
        assert!(cap.system_prompt_addition().is_some());
    }

    #[test]
    fn capability_coexists_with_openui() {
        let registry = CapabilityRegistry::with_builtins();
        assert!(
            registry.get("openui").is_some(),
            "openui should stay registered"
        );
        assert!(
            registry.get(A2UI_CAPABILITY_ID).is_some(),
            "a2ui should be registered"
        );
    }

    #[test]
    fn system_prompt_lists_core_components() {
        let cap = A2UiCapability;
        let p = cap.system_prompt_addition().unwrap();
        for name in ["Card", "Stack", "Button", "Form", "List", "Table"] {
            assert!(p.contains(name), "prompt missing component {name}");
        }
    }
}
