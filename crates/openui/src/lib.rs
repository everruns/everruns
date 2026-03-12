//! OpenUI Component Library and Prompt Generator
//!
//! Rust reimplementation of the OpenUI component definitions and prompt generator.
//! Produces system prompts that instruct LLMs to generate OpenUI Lang code.
//!
//! The generated prompt is compatible with `@openuidev/react-lang` and `@openuidev/react-ui`
//! — the React runtime parses the LLM output and renders it into interactive components.
//!
//! Ref: https://github.com/thesysdev/openui
//! - Component definitions: packages/react-ui/src/genui-lib/
//! - Prompt generator: packages/react-lang/src/parser/prompt.ts
//! - Library/types: packages/react-lang/src/library.ts

mod components;
mod groups;
mod prompt;

pub use components::{ComponentDef, Library, PropDef};
pub use groups::ComponentGroup;
pub use prompt::{PromptOptions, generate_prompt};

use std::sync::LazyLock;

/// The default OpenUI library matching `@openuidev/react-ui` openuiChatLibrary.
/// Ref: packages/react-ui/src/genui-lib/openuiChatLibrary.tsx
pub fn default_library() -> &'static Library {
    &DEFAULT_LIBRARY
}

static DEFAULT_LIBRARY: LazyLock<Library> = LazyLock::new(|| Library {
    root: "Card",
    components: components::all_components(),
    groups: groups::default_groups(),
});

/// Generate the default OpenUI system prompt with standard options.
///
/// This is the convenience function used by the OpenUI capability.
pub fn default_prompt() -> &'static str {
    &DEFAULT_PROMPT
}

static DEFAULT_PROMPT: LazyLock<String> =
    LazyLock::new(|| generate_prompt(default_library(), &PromptOptions::default()));

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_library_has_components() {
        let lib = default_library();
        assert!(!lib.components.is_empty());
        assert_eq!(lib.root, "Card");
    }

    #[test]
    fn test_default_library_has_all_expected_components() {
        let lib = default_library();
        let names: Vec<&str> = lib.components.iter().map(|c| c.name).collect();

        // Layout
        assert!(names.contains(&"Stack"));
        assert!(names.contains(&"Tabs"));
        assert!(names.contains(&"Accordion"));

        // Content
        assert!(names.contains(&"Card"));
        assert!(names.contains(&"TextContent"));
        assert!(names.contains(&"CodeBlock"));

        // Charts
        assert!(names.contains(&"BarChart"));
        assert!(names.contains(&"PieChart"));
        assert!(names.contains(&"LineChart"));

        // Tables
        assert!(names.contains(&"Table"));

        // Forms
        assert!(names.contains(&"Form"));
        assert!(names.contains(&"Input"));
        assert!(names.contains(&"Select"));

        // Buttons
        assert!(names.contains(&"Button"));

        // Chat-specific
        assert!(names.contains(&"ListBlock"));
        assert!(names.contains(&"FollowUpBlock"));
        assert!(names.contains(&"SectionBlock"));
    }

    #[test]
    fn test_default_prompt_not_empty() {
        let prompt = default_prompt();
        assert!(!prompt.is_empty());
        assert!(prompt.len() > 500);
    }

    #[test]
    fn test_default_prompt_contains_key_sections() {
        let prompt = default_prompt();
        assert!(prompt.contains("## Syntax Rules"));
        assert!(prompt.contains("## Component Signatures"));
        assert!(prompt.contains("## Hoisting & Streaming"));
        assert!(prompt.contains("## Important Rules"));
    }

    #[test]
    fn test_default_prompt_instructs_openui_code_blocks() {
        let prompt = default_prompt();
        assert!(prompt.contains("```openui"));
    }

    #[test]
    fn test_component_count() {
        let lib = default_library();
        // Should have all ~55 components from the chat library
        assert!(
            lib.components.len() >= 50,
            "Expected at least 50 components, got {}",
            lib.components.len()
        );
    }

    #[test]
    fn test_groups_cover_all_components() {
        let lib = default_library();
        let grouped: Vec<&str> = lib
            .groups
            .iter()
            .flat_map(|g| g.components.iter().copied())
            .collect();

        for comp in &lib.components {
            assert!(
                grouped.contains(&comp.name),
                "Component '{}' is not in any group",
                comp.name
            );
        }
    }

    #[test]
    fn test_prompt_contains_all_component_signatures() {
        let lib = default_library();
        let prompt = default_prompt();

        for comp in &lib.components {
            assert!(
                prompt.contains(&format!("{}(", comp.name)),
                "Prompt missing signature for component '{}'",
                comp.name
            );
        }
    }
}
