//! Built-in harness definitions.
//!
//! Decision: Only platform-essential harnesses are auto-provisioned per org —
//! `base` and `generic`. Platform Chat is a managed Agent. Specialized harnesses
//! (`coding`, `data-analyst`) live in the
//! `examples` module and are adopted on demand via `/v1/harness-examples`
//! and `POST /v1/harnesses/import?from-example=…`.
//!
//! Each submodule still defines one harness (system prompt, capabilities,
//! tags, roles). The top-level `built_in_harnesses()` function collects the
//! always-installed ones into the ordered list consumed by
//! `oss_built_in_harnesses()` in platform.rs.

mod base;
mod coding;
mod coding_prompt;
mod data_analyst;
pub mod examples;
mod generic;

use everruns_platform::BuiltInHarnessDefinition;

pub use examples::{
    HarnessExampleDef, LEGACY_BUILT_IN_NAMES, find_harness_example, harness_examples,
};

/// All built-in harness definitions in provisioning order.
///
/// Only platform-essential harnesses are listed here. Specialized harnesses
/// (data analyst and coding) are adopted from `harness_examples()`.
pub fn built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    vec![base::definition(), generic::definition()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_harness_definition_declares_an_icon() {
        // Icons are code-defined: the UI renders `Harness.icon` and only falls
        // back to a generic glyph for user-created harnesses.
        let definitions = built_in_harnesses()
            .into_iter()
            .chain(harness_examples().into_iter().map(|ex| ex.definition));
        for definition in definitions {
            assert!(
                definition.icon.is_some(),
                "harness {} must declare an icon",
                definition.name
            );
        }
    }

    #[test]
    fn built_in_list_excludes_example_harnesses() {
        let names: Vec<String> = built_in_harnesses().into_iter().map(|h| h.name).collect();

        // The default built-in list now contains only platform-essential
        // harnesses. Specialized coding/data harnesses moved to examples.
        assert_eq!(names, vec!["base", "generic"]);
        for legacy in LEGACY_BUILT_IN_NAMES {
            assert!(
                !names.iter().any(|n| n == legacy),
                "{legacy} should no longer be a default built-in"
            );
        }
    }

    /// EVE-1041: `generic` is one definition, read by org provisioning here and
    /// by `everruns::Harness::generic()` in the facade.
    ///
    /// Sharing a function already makes divergence impossible; this fails if
    /// someone re-hardcodes the list locally, which is how it drifted before.
    #[test]
    fn shared_generic_capabilities_are_the_platform_ones() {
        let provisioned: Vec<String> = generic::definition()
            .capabilities
            .iter()
            .map(|capability| capability.capability_id().to_string())
            .collect();
        let shared: Vec<String> = everruns_contracts::generic_capabilities()
            .iter()
            .map(|capability| capability.capability_id().to_string())
            .collect();
        assert_eq!(provisioned, shared);
        assert!(
            provisioned.len() > 10,
            "a truncated list would pass a bare equality check against itself"
        );
    }

    /// The shared floor carries capability references only. Presentation and
    /// the base system prompt stay platform-side, per
    /// `knowledge/framework/harnesses.md`.
    #[test]
    fn the_shared_floor_leaves_presentation_to_the_platform() {
        let definition = generic::definition();
        assert_eq!(definition.icon.as_deref(), Some("box"));
        assert!(!definition.system_prompt.is_empty());
    }

    #[test]
    fn interactive_harnesses_expose_ask_user_and_describe_it() {
        {
            let definition = generic::definition();
            assert!(
                definition
                    .capabilities
                    .iter()
                    .any(|capability| capability.capability_id() == "ask_user"),
                "{} must expose ask_user",
                definition.name
            );
            assert!(
                definition.description.contains("structured user questions"),
                "{} must describe structured user questions",
                definition.name
            );
        }

        assert!(
            base::definition()
                .capabilities
                .iter()
                .all(|capability| capability.capability_id() != "ask_user")
        );
    }

    #[test]
    fn ask_user_harnesses_also_expose_request_approval() {
        {
            let definition = generic::definition();
            let capabilities = definition
                .capabilities
                .iter()
                .map(|capability| capability.capability_id())
                .collect::<Vec<_>>();
            assert!(
                capabilities.contains(&"soft_approval"),
                "{} must keep request_approval available alongside ask_user",
                definition.name
            );
        }
    }
}
