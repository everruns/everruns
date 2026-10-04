//! Built-in harness definitions.
//!
//! Decision: Only platform-essential harnesses are auto-provisioned per org —
//! the canonical levels and deprecated `generic`. Platform Chat is a managed Agent.
//! Specialized harnesses
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
mod levels;

use everruns_contracts::capability::BuiltInHarnessPreset;
use crate::records::BuiltInHarnessDefinition;

pub use examples::{
    HarnessExampleDef, LEGACY_BUILT_IN_NAMES, find_harness_example, harness_examples,
};

/// All built-in harness definitions in provisioning order.
///
/// Only platform-essential harnesses are listed here. Specialized harnesses
/// (data analyst and coding) are adopted from `harness_examples()`.
pub fn built_in_harnesses() -> Vec<BuiltInHarnessDefinition> {
    vec![
        base::definition(),
        levels::definition(BuiltInHarnessPreset::Conversation),
        levels::definition(BuiltInHarnessPreset::WorkerBase),
        levels::definition(BuiltInHarnessPreset::Worker),
        generic::definition(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn effective_runtime_tools_match_level_boundaries() {
        use everruns_core::capabilities::{
            SystemPromptContext, collect_capabilities_with_configs, resolve_capability_configs,
        };
        let registry = crate::platform::oss_capability_registry();
        let ctx =
            SystemPromptContext::without_file_store(everruns_contracts::typed_id::SessionId::new());
        for preset in [
            BuiltInHarnessPreset::Base,
            BuiltInHarnessPreset::Conversation,
            BuiltInHarnessPreset::WorkerBase,
            BuiltInHarnessPreset::Worker,
        ] {
            let resolved =
                resolve_capability_configs(&preset.effective_capabilities(), &registry).unwrap();
            let collected = collect_capabilities_with_configs(&resolved, &registry, &ctx).await;
            let tools: Vec<_> = collected.tools.iter().map(|tool| tool.name()).collect();
            assert_eq!(
                tools.contains(&"bash"),
                matches!(
                    preset,
                    BuiltInHarnessPreset::WorkerBase | BuiltInHarnessPreset::Worker
                )
            );
            assert_eq!(
                tools.contains(&"spawn_agent"),
                preset == BuiltInHarnessPreset::Worker
            );
            assert_eq!(
                tools.contains(&"list_tasks"),
                preset == BuiltInHarnessPreset::Worker
            );
            assert!(!tools.contains(&"secret_store"));
            if preset == BuiltInHarnessPreset::Base || preset == BuiltInHarnessPreset::Conversation
            {
                assert!(tools.is_empty(), "unexpected tools: {tools:?}");
            }
        }
    }

    #[test]
    fn hosted_parent_chain_matches_the_framework_presets() {
        let definitions = built_in_harnesses();
        for preset in [
            BuiltInHarnessPreset::Base,
            BuiltInHarnessPreset::Conversation,
            BuiltInHarnessPreset::WorkerBase,
            BuiltInHarnessPreset::Worker,
        ] {
            let mut chain = vec![];
            let mut current = definitions
                .iter()
                .find(|h| h.name == preset.name())
                .unwrap();
            loop {
                chain.push(current);
                match &current.parent_name {
                    Some(name) => current = definitions.iter().find(|h| &h.name == name).unwrap(),
                    None => break,
                }
            }
            let effective: Vec<_> = chain
                .into_iter()
                .rev()
                .flat_map(|h| h.capabilities.clone())
                .collect();
            assert_eq!(
                serde_json::to_value(effective).unwrap(),
                serde_json::to_value(preset.effective_capabilities()).unwrap()
            );
        }
    }

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
        assert_eq!(
            names,
            vec!["base", "conversation", "worker-base", "worker", "generic",]
        );
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
            // The description is the purpose shown in the harness picker, so it
            // stays a sentence about when to choose it rather than a capability list.
            assert!(
                !definition.description.trim().is_empty()
                    && definition.description.chars().count() <= 160,
                "{} description should say when to use it, in picker length",
                definition.name
            );
        }

        let base = base::definition();
        assert!(
            base.capabilities
                .iter()
                .all(|capability| capability.capability_id() != "ask_user")
        );
        assert!(
            !base.description.trim().is_empty() && base.description.chars().count() <= 160,
            "base description should say when to use it, in picker length"
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
