//! Harness examples — adoptable templates analogous to agent examples.
//!
//! Decision: Examples live in code, not in DB. Mirrors `seed::SEED_AGENTS`.
//! Decision: Listed via `GET /v1/harness-examples`; adopted via
//!   `POST /v1/harnesses/import?from-example={name}` which creates a normal
//!   org-owned harness (`is_built_in = false`) inheriting from the org's
//!   `generic` harness by name.
//! Decision: Examples are filtered at runtime by capability registration —
//!   examples whose required capabilities are missing are hidden, matching
//!   agent examples behaviour.

use everruns_platform::BuiltInHarnessDefinition;

use super::{coding_container, coding_daytona, data_analyst};

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
/// Each example is filtered at request time by capability registration so that
/// `coding-container` (and any future feature-gated capability) only appears
/// when the corresponding capability plugin is registered for the deployment.
pub fn harness_examples() -> Vec<HarnessExampleDef> {
    vec![
        HarnessExampleDef::new(coding_daytona::definition()),
        HarnessExampleDef::new(coding_container::definition()),
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
pub const LEGACY_BUILT_IN_NAMES: &[&str] = &["coding-container", "coding-daytona", "data-analyst"];

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
    fn examples_inherit_from_generic_by_name() {
        for ex in harness_examples() {
            assert_eq!(
                ex.definition.parent_name.as_deref(),
                Some("generic"),
                "harness example {} must inherit from `generic` by name",
                ex.definition.name
            );
        }
    }

    #[test]
    fn legacy_names_match_examples() {
        let example_names: Vec<String> = harness_examples()
            .iter()
            .map(|e| e.definition.name.clone())
            .collect();
        for name in LEGACY_BUILT_IN_NAMES {
            assert!(
                example_names.iter().any(|n| n == name),
                "legacy built-in {name} must be re-exposed as a harness example"
            );
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
