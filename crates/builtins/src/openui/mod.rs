//! OpenUI catalog, prompt generator, and capability.
//!
//! The catalog mirrors `@openuidev/react-ui`. When enabled, the capability appends
//! the generated OpenUI system prompt to the agent's
//! system prompt. The prompt instructs the LLM to respond with OpenUI Lang code
//! wrapped in ```openui fenced code blocks when visual content is requested.
//!
//! The UI detects these blocks and renders them using the @openuidev/react-lang
//! parser and @openuidev/react-ui component library.
//!
//! Ref: https://github.com/thesysdev/openui
//! Ref: knowledge/ui/openui.md
//!
//! ```
//! use everruns_builtins::openui::{PromptOptions, default_library, generate_prompt};
//!
//! let prompt = generate_prompt(default_library(), &PromptOptions::default());
//! assert!(prompt.contains("```openui"));
//! ```

mod components;
mod groups;
mod prompt;

pub use components::{ComponentDef, Library, PropDef};
pub use groups::ComponentGroup;
pub use prompt::{PromptOptions, generate_prompt};

use std::sync::LazyLock;

use super::{Capability, CapabilityLocalization, CapabilityStatus};

/// The default OpenUI library matching `@openuidev/react-ui` openuiChatLibrary.
pub fn default_library() -> &'static Library {
    &DEFAULT_LIBRARY
}

static DEFAULT_LIBRARY: LazyLock<Library> = LazyLock::new(|| Library {
    root: "Card",
    components: components::all_components(),
    groups: groups::default_groups(),
});

/// Generates the default OpenUI system prompt with standard options.
pub fn default_prompt() -> &'static str {
    &DEFAULT_PROMPT
}

static DEFAULT_PROMPT: LazyLock<String> =
    LazyLock::new(|| generate_prompt(default_library(), &PromptOptions::default()));

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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "OpenUI",
            "Дає агенту змогу генерувати насичені інтерактивні UI-компоненти (графіки, таблиці, форми, картки тощо) за допомогою OpenUI Lang.",
        )]
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
        Some(default_prompt())
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["openui"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/tool-list constants covered by builtin_capabilities_satisfy_registry_invariants.

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

#[cfg(test)]
mod catalog_tests {
    use super::*;

    #[test]
    fn default_library_covers_catalog_and_groups() {
        let library = default_library();
        assert_eq!(library.root, "Card");
        assert!(library.components.len() >= 50);

        let grouped: Vec<&str> = library
            .groups
            .iter()
            .flat_map(|group| group.components.iter().copied())
            .collect();
        for component in &library.components {
            assert!(
                grouped.contains(&component.name),
                "component '{}' is not in any group",
                component.name
            );
        }
    }

    #[test]
    fn default_prompt_contains_every_component_signature() {
        let prompt = default_prompt();
        for component in &default_library().components {
            assert!(
                prompt.contains(&format!("{}(", component.name)),
                "prompt missing signature for component '{}'",
                component.name
            );
        }
    }
}
