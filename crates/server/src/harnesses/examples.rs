//! Harness examples — adoptable templates analogous to agent examples.
//!
//! Decision: Examples live in code, not in DB. Mirrors `seed::SEED_AGENTS`.
//! Decision: Listed via `GET /v1/harness-examples`; adopted via
//!   `POST /v1/harnesses/import?from-example={name}` which creates a normal
//!   org-owned harness (`is_built_in = false`) inheriting from a built-in
//!   worker by name: Coding needs a full sandbox, Data Analyst runs on Bashkit.
//! Decision: Examples are filtered at runtime by capability registration —
//!   examples whose required capabilities are missing are hidden, matching
//!   agent examples behaviour.

use crate::domains::harnesses::record::BuiltInHarnessDefinition;

use super::{coding, data_analyst};

/// A harness example wrapping a `BuiltInHarnessDefinition` with adoption
/// metadata (dev gating).
pub struct HarnessExampleDef {
    /// Underlying harness definition (name, prompt, capabilities, parent name…).
    pub definition: BuiltInHarnessDefinition,
    /// If true, only available when experimental features are enabled.
    pub dev_only: bool,
}

impl HarnessExampleDef {
    fn new(definition: BuiltInHarnessDefinition) -> Self {
        Self {
            definition,
            dev_only: false,
        }
    }
}

/// Adoptable harness examples in display order.
///
/// Each example is filtered at request time by capability registration, so an
/// example only appears when its required plugins are available.
pub fn harness_examples() -> Vec<HarnessExampleDef> {
    vec![
        HarnessExampleDef::new(coding::definition()),
        HarnessExampleDef::new(data_analyst::definition()),
    ]
}

/// Look up a harness example by its `name`.
pub fn find_harness_example(name: &str) -> Option<HarnessExampleDef> {
    harness_examples()
        .into_iter()
        .find(|ex| ex.definition.name == name)
}

/// Names of harnesses that used to be installed as built-ins for every org but
/// are now opt-in examples. Used by reconciliation to release legacy
/// `is_built_in = true` rows back to org ownership without breaking existing
/// references.
pub const LEGACY_BUILT_IN_NAMES: &[&str] = &[
    "coding-container",
    "coding-daytona",
    "coding-session-sandbox",
    "data-analyst",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn examples_have_unique_names() {
        let examples = harness_examples();
        let names: Vec<&str> = examples
            .iter()
            .map(|e| e.definition.name.as_str())
            .collect();
        let mut unique = names.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(names.len(), unique.len(), "duplicate harness example names");
    }

    #[test]
    fn examples_inherit_from_a_built_in_worker_by_name() {
        let built_in: Vec<String> = super::super::built_in_harnesses()
            .into_iter()
            .filter(|h| !h.tags.iter().any(|tag| tag == "deprecated"))
            .map(|h| h.name)
            .collect();
        for ex in harness_examples() {
            let parent = ex.definition.parent_name.as_deref().unwrap_or_default();
            assert!(
                parent.ends_with("worker") && built_in.iter().any(|name| name == parent),
                "harness example {} must inherit from a current built-in worker, not {parent}",
                ex.definition.name
            );
        }
    }

    #[test]
    fn old_provider_specific_coding_names_remain_reconciliation_only() {
        let example_names = harness_examples()
            .into_iter()
            .map(|example| example.definition.name)
            .collect::<Vec<_>>();
        assert!(example_names.iter().any(|name| name == "coding"));
        for name in [
            "coding-container",
            "coding-daytona",
            "coding-session-sandbox",
        ] {
            assert!(!example_names.iter().any(|example| example == name));
            assert!(LEGACY_BUILT_IN_NAMES.contains(&name));
        }
    }

    #[test]
    fn find_returns_none_for_unknown() {
        assert!(find_harness_example("does-not-exist").is_none());
    }

    #[test]
    fn find_returns_some_for_known() {
        let ex = find_harness_example("data-analyst").expect("data-analyst is an example");
        assert_eq!(ex.definition.display_name, "Data Analyst");
    }
}
