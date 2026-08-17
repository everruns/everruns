//! A2UI Catalog and Category Definitions
//!
//! A Catalog is the agent-facing description of which components are available.
//! Categories group components for readable prompt output.

use super::components::ComponentDef;

/// One category of components in a catalog (e.g. "Layout", "Forms").
pub struct ComponentCategory {
    pub name: &'static str,
    pub components: Vec<&'static str>,
    /// Notes rendered after the category's component signatures.
    pub notes: Vec<&'static str>,
}

/// A catalog describes the components an agent may emit.
pub struct Catalog {
    /// Hint for the recommended root component (typically "Card").
    pub root_hint: &'static str,
    pub components: Vec<ComponentDef>,
    pub categories: Vec<ComponentCategory>,
}

/// Default category grouping for the built-in catalog.
pub fn default_categories() -> Vec<ComponentCategory> {
    vec![
        ComponentCategory {
            name: "Layout",
            components: vec!["Card", "Stack", "Separator"],
            notes: vec![],
        },
        ComponentCategory {
            name: "Content",
            components: vec!["Heading", "Text", "Callout", "Badge", "Image", "CodeBlock"],
            notes: vec![],
        },
        ComponentCategory {
            name: "Data",
            components: vec!["List", "ListItem", "Table"],
            notes: vec![],
        },
        ComponentCategory {
            name: "Forms",
            components: vec!["Form", "TextField", "Textarea", "Select", "Checkbox"],
            notes: vec![
                "Form submission posts a chat message (see `submitMessage`). \
                 Form round-trips with typed responses are not yet supported.",
            ],
        },
        ComponentCategory {
            name: "Actions",
            components: vec!["Button", "ButtonGroup"],
            notes: vec![],
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn categories_have_no_duplicates() {
        let mut seen = HashSet::new();
        for cat in default_categories() {
            for c in cat.components {
                assert!(seen.insert(c), "duplicate component across categories: {c}");
            }
        }
    }

    #[test]
    fn categories_non_empty() {
        for cat in default_categories() {
            assert!(!cat.components.is_empty(), "empty category: {}", cat.name);
        }
    }
}
