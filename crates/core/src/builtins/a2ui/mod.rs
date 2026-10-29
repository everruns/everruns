//! A2UI catalog, prompt generator, and capability.
//!
//! When enabled, the capability appends the generated A2UI system prompt to the agent's
//! system prompt. The LLM wraps A2UI 1.0 operations in ```a2ui fenced code
//! blocks, rendered by the shared browser catalog in chat and Work Chat.
//!
//! Runs in parallel with `openui`. Both may be enabled on the same agent, but
//! that is rarely useful — instruct the agent to prefer one protocol.
//!
//! [A2UI project](https://github.com/google/a2ui).
//!
//! ```
//! use everruns_core::builtins::a2ui::{PromptOptions, default_catalog, generate_prompt};
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
    LazyLock::new(|| include_str!("standard-catalog.md").to_owned());

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
        "Enables A2UI 1.0 components, interactive forms, charts and surfaces in chat and Work Chat."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "A2UI",
            "Дає агенту змогу створювати компоненти A2UI 1.0, інтерактивні форми, графіки й поверхні в чаті та Work Chat.",
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
    fn default_prompt_contains_standard_catalog_and_surface_contract() {
        let prompt = default_prompt();
        for name in ["Column(", "Chart(", "Map(", "Modal(", "DateTimeInput(", "Form("] {
            assert!(prompt.contains(name), "missing standard component: {name}");
        }
        for operation in ["createSurface", "updateComponents", "updateDataModel", "deleteSurface"] {
            assert!(prompt.contains(operation));
        }
        assert!(prompt.contains("copilotkit/standard@1"));
        assert!(prompt.contains("v1.0"));
        assert!(prompt.contains("Never emit callRendererFunction"));
    }
}
