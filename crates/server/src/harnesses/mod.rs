//! Built-in harness definitions.
//!
//! Decision: Only platform-essential harnesses are auto-provisioned per org —
//! the canonical tree (Base → Conversation | Worker → Bashkit Worker |
//! Sandbox Worker) and the deprecated `worker-base` and `generic` rows kept
//! for existing bindings. Platform Chat is a managed Agent on Bashkit Worker.
//! Specialized harnesses
//! (`coding`, `data-analyst`) live in the
//! `examples` module and are adopted on demand via `/v1/harness-examples`
//! and `POST /v1/harnesses/import?from-example=…`.
//!
//! Each submodule still defines one harness (system prompt, capabilities,
//! tags, roles). The top-level `built_in_harnesses()` function collects the
//! always-installed ones into the ordered list consumed by
//! `oss_built_in_harnesses()`.

mod base;
mod bashkit_worker;
mod coding;
mod coding_prompt;
mod data_analyst;
pub mod examples;
mod generic;
mod levels;
mod sandbox_worker;
mod worker_base;

use crate::domains::harnesses::record::BuiltInHarnessDefinition;
use everruns_contracts::capability::BuiltInHarnessPreset;

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
        levels::definition(BuiltInHarnessPreset::Worker),
        bashkit_worker::definition(),
        sandbox_worker::definition(),
        worker_base::definition(),
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
            BuiltInHarnessPreset::Worker,
            BuiltInHarnessPreset::BashkitWorker,
        ] {
            let resolved =
                resolve_capability_configs(&preset.effective_capabilities(), &registry).unwrap();
            let collected = collect_capabilities_with_configs(&resolved, &registry, &ctx).await;
            let tools: Vec<_> = collected.tools.iter().map(|tool| tool.name()).collect();
            let worker = matches!(
                preset,
                BuiltInHarnessPreset::Worker | BuiltInHarnessPreset::BashkitWorker
            );
            assert_eq!(
                tools.contains(&"bash"),
                preset == BuiltInHarnessPreset::BashkitWorker
            );
            assert_eq!(tools.contains(&"read_file"), worker);
            assert_eq!(tools.contains(&"spawn_agent"), worker);
            assert_eq!(tools.contains(&"list_tasks"), worker);
            assert!(!tools.contains(&"secret_store"));
            if preset == BuiltInHarnessPreset::Base {
                // Base brings no tools beyond the approval gate.
                assert!(
                    tools.iter().all(|tool| tool.contains("approval")),
                    "unexpected tools: {tools:?}"
                );
            }
        }
    }

    #[test]
    fn hosted_parent_chain_matches_the_framework_presets() {
        let definitions = built_in_harnesses();
        for preset in [
            BuiltInHarnessPreset::Base,
            BuiltInHarnessPreset::Conversation,
            BuiltInHarnessPreset::Worker,
            BuiltInHarnessPreset::BashkitWorker,
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
            vec![
                "base",
                "conversation",
                "worker",
                "bashkit-worker",
                "sandbox-worker",
                "worker-base",
                "generic",
            ]
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
    /// the base system prompt stay platform-side.
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
        for definition in built_in_harnesses() {
            assert!(
                !definition.description.trim().is_empty()
                    && definition.description.chars().count() <= 160,
                "{} description should say when to use it, in picker length",
                definition.name
            );
        }
    }

    #[test]
    fn execution_is_declared_only_where_the_environment_is_chosen() {
        use crate::domains::harnesses::record::HarnessExecution;
        for definition in built_in_harnesses() {
            let expected = match definition.name.as_str() {
                "bashkit-worker" => HarnessExecution::FixedBashkit,
                "sandbox-worker" => HarnessExecution::FullSandbox,
                _ => HarnessExecution::Unbound,
            };
            assert_eq!(definition.execution, expected, "{}", definition.name);
        }
    }

    #[test]
    fn deprecated_worker_base_keeps_its_legacy_surface() {
        let definition = worker_base::definition();
        assert!(definition.tags.iter().any(|tag| tag == "deprecated"));
        assert_eq!(definition.parent_name.as_deref(), Some("conversation"));
        let ids: Vec<_> = definition
            .capabilities
            .iter()
            .map(|capability| capability.capability_id())
            .collect();
        assert!(ids.contains(&"bashkit_shell") && ids.contains(&"session_file_system"));
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
