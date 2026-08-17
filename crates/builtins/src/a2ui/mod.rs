//! A2UI catalog, prompt generator, and capability.
//!
//! When enabled, the capability appends the generated A2UI system prompt to the agent's
//! system prompt. The LLM wraps JSON component trees in ```a2ui fenced code
//! blocks, which the UI detects and renders using native shadcn/ui primitives.
//!
//! Runs in parallel with `openui`. Both may be enabled on the same agent, but
//! that is rarely useful — instruct the agent to prefer one protocol.
//!
//! Ref: knowledge/ui/a2ui.md
//! Ref: https://github.com/google/a2ui
//!
//! ```
//! use everruns_builtins::a2ui::{PromptOptions, default_catalog, generate_prompt};
//!
//! let prompt = generate_prompt(default_catalog(), &PromptOptions::default());
//! assert!(prompt.contains("```a2ui"));
//! ```

mod catalog;
mod components;
mod prompt;

pub use catalog::{Catalog, ComponentCategory};
pub use components::{ComponentDef, PropDef};
pub use prompt::{PromptOptions, generate_prompt};

use std::sync::LazyLock;

use super::{Capability, CapabilityLocalization, CapabilityStatus};

/// The default A2UI catalog.
pub fn default_catalog() -> &'static Catalog {
    &DEFAULT_CATALOG
}

static DEFAULT_CATALOG: LazyLock<Catalog> = LazyLock::new(|| Catalog {
    root_hint: "Card",
    components: components::all_components(),
    categories: catalog::default_categories(),
});

/// Generates the default A2UI system prompt with standard options.
pub fn default_prompt() -> &'static str {
    &DEFAULT_PROMPT
}

static DEFAULT_PROMPT: LazyLock<String> =
    LazyLock::new(|| generate_prompt(default_catalog(), &PromptOptions::default()));

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

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "A2UI",
            "Дає агенту змогу видавати генеративний UI як дерева компонентів Google A2UI у форматі JSON, які інтерфейс рендерить нативними компонентами дизайн-системи.",
        )]
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
        Some(default_prompt())
    }

    fn features(&self) -> Vec<&'static str> {
        vec!["a2ui"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Metadata/tool-list/registration constants covered by builtin_capabilities_satisfy_registry_invariants.

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
    fn system_prompt_lists_core_components() {
        let cap = A2UiCapability;
        let p = cap.system_prompt_addition().unwrap();
        for name in ["Card", "Stack", "Button", "Form", "List", "Table"] {
            assert!(p.contains(name), "prompt missing component {name}");
        }
    }
}

#[cfg(test)]
mod catalog_tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn default_catalog_covers_components_and_categories() {
        let catalog = default_catalog();
        let names: Vec<&str> = catalog
            .components
            .iter()
            .map(|component| component.name)
            .collect();
        for expected in ["Card", "Stack", "Button", "List", "Table", "Form"] {
            assert!(names.contains(&expected), "missing component: {expected}");
        }

        let grouped: HashSet<&str> = catalog
            .categories
            .iter()
            .flat_map(|category| category.components.iter().copied())
            .collect();
        for component in &catalog.components {
            assert!(
                grouped.contains(component.name),
                "component '{}' is not in any category",
                component.name
            );
        }
    }

    #[test]
    fn default_prompt_contains_every_component_and_action_types() {
        let prompt = default_prompt();
        for component in &default_catalog().components {
            assert!(prompt.contains(component.name));
        }
        assert!(prompt.contains("\"message\""));
        assert!(prompt.contains("\"open_url\""));
    }
}
