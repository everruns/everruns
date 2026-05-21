//! A2UI component catalog and prompt generator for Everruns.
//!
//! Produces system prompts that instruct LLMs to emit A2UI JSON component trees.
//! The JSON is transported in ```a2ui fenced code blocks and rendered by the UI
//! using native shadcn/ui primitives.
//!
//! This crate is part of the [Everruns](https://everruns.com) ecosystem.
//!
//! # Example
//!
//! ```
//! use everruns_a2ui::{PromptOptions, default_catalog, generate_prompt};
//!
//! let prompt = generate_prompt(default_catalog(), &PromptOptions::default());
//! assert!(prompt.contains("```a2ui"));
//! ```
//!
//! Ref: specs/a2ui.md
//! Ref: <https://github.com/google/a2ui>

mod catalog;
mod components;
mod prompt;

pub use catalog::{Catalog, ComponentCategory};
pub use components::{ComponentDef, PropDef};
pub use prompt::{PromptOptions, generate_prompt};

use std::sync::LazyLock;

/// The default A2UI catalog.
pub fn default_catalog() -> &'static Catalog {
    &DEFAULT_CATALOG
}

static DEFAULT_CATALOG: LazyLock<Catalog> = LazyLock::new(|| Catalog {
    root_hint: "Card",
    components: components::all_components(),
    categories: catalog::default_categories(),
});

/// Generate the default A2UI system prompt with standard options.
pub fn default_prompt() -> &'static str {
    &DEFAULT_PROMPT
}

static DEFAULT_PROMPT: LazyLock<String> =
    LazyLock::new(|| generate_prompt(default_catalog(), &PromptOptions::default()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_catalog_has_components() {
        let cat = default_catalog();
        assert!(!cat.components.is_empty());
    }

    #[test]
    fn default_catalog_contains_core_components() {
        let cat = default_catalog();
        let names: Vec<&str> = cat.components.iter().map(|c| c.name).collect();
        for expected in [
            "Card",
            "Stack",
            "Heading",
            "Text",
            "Button",
            "ButtonGroup",
            "List",
            "ListItem",
            "Table",
            "Form",
            "TextField",
            "Select",
            "Checkbox",
            "Callout",
            "Badge",
        ] {
            assert!(names.contains(&expected), "missing component: {expected}");
        }
    }

    #[test]
    fn default_prompt_is_substantial() {
        let prompt = default_prompt();
        assert!(
            prompt.len() > 800,
            "prompt too short: {} chars",
            prompt.len()
        );
    }

    #[test]
    fn default_prompt_mentions_a2ui_fence() {
        assert!(default_prompt().contains("```a2ui"));
    }

    #[test]
    fn default_prompt_mentions_json_shape() {
        let prompt = default_prompt();
        assert!(prompt.contains("\"type\""));
        assert!(prompt.contains("\"props\""));
        assert!(prompt.contains("\"children\""));
    }

    #[test]
    fn default_prompt_lists_every_component() {
        let cat = default_catalog();
        let prompt = default_prompt();
        for comp in &cat.components {
            assert!(
                prompt.contains(comp.name),
                "prompt missing component {}",
                comp.name
            );
        }
    }

    #[test]
    fn default_prompt_includes_action_types() {
        let prompt = default_prompt();
        assert!(prompt.contains("\"message\""));
        assert!(prompt.contains("\"open_url\""));
    }

    #[test]
    fn categories_cover_all_components() {
        let cat = default_catalog();
        let grouped: std::collections::HashSet<&str> = cat
            .categories
            .iter()
            .flat_map(|g| g.components.iter().copied())
            .collect();
        for comp in &cat.components {
            assert!(
                grouped.contains(comp.name),
                "component '{}' not in any category",
                comp.name
            );
        }
    }
}
