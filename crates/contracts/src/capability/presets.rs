//! Built-in harness capability sets, as data both surfaces load.
//!
//! Shared presets keep organization provisioning and embedded applications
//! aligned on the same capability sets.
//!
//! This crate is the shared floor because it is the one both already depend
//! on: the platform re-exports [`CapabilityRef`] as `BuiltInCapabilityDefinition`
//! and the `everruns` facade re-exports it under its own name. Putting the
//! list here means an application adopting it pulls in no platform code.
//!
//! Only the capability set is shared. The base system prompt and the hosted
//! record's presentation fields (`icon`, `starters`, `intro_markdown`) stay
//! with the platform: a written world-description drifts from the world it
//! describes, and presentation has no meaning in a library.

use crate::capability::CapabilityRef;

/// Canonical presets shared by hosted provisioning and the application framework.
///
/// Decision: each layer answers one question. Base: what every agent needs to
/// run reliably. Conversation or Worker: does the agent get a workspace.
/// Bashkit Worker (and the hosted-only Sandbox Worker): where that workspace
/// runs. Inheritance has no subtraction, so a layer only holds what every
/// child wants; the execution environment is the last split because it is the
/// most specific choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltInHarnessPreset {
    /// System essentials every agent needs: context and tool-call robustness.
    Base,
    /// Simple chat: no workspace, no shell.
    Conversation,
    /// The worker kit (files, instructions, skills, delegation) with no compute.
    Worker,
    /// Worker plus the Bashkit virtual shell.
    BashkitWorker,
}

impl BuiltInHarnessPreset {
    /// Stable addressable built-in name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Conversation => "conversation",
            Self::Worker => "worker",
            Self::BashkitWorker => "bashkit-worker",
        }
    }

    /// The live parent on the hosted platform.
    pub const fn parent(self) -> Option<Self> {
        match self {
            Self::Base => None,
            Self::Conversation | Self::Worker => Some(Self::Base),
            Self::BashkitWorker => Some(Self::Worker),
        }
    }

    /// Only this layer's additions; hosted harnesses inherit the parent live.
    pub fn local_capabilities(self) -> Vec<CapabilityRef> {
        match self {
            Self::Base => vec![
                CapabilityRef::with_config(
                    "compaction",
                    serde_json::json!({
                        "strategy": "auto", "proactive": true, "budget_percent": 0.85
                    }),
                ),
                CapabilityRef::new("error_disclosure"),
                CapabilityRef::new("tool_call_repair"),
                CapabilityRef::new("loop_detection"),
                CapabilityRef::with_config(
                    "parallel_tool_calls",
                    serde_json::json!({"mode": "prefer"}),
                ),
                // Guidance, not a wall: free on safe work, and the gate that
                // `ask_user` points destructive actions at.
                CapabilityRef::new("soft_approval"),
            ],
            Self::Conversation => vec![
                CapabilityRef::new("ask_user"),
                CapabilityRef::new("message_metadata"),
            ],
            Self::Worker => vec![
                CapabilityRef::new("session_file_system"),
                CapabilityRef::new("agent_instructions"),
                CapabilityRef::new("session"),
                CapabilityRef::new("tool_output_persistence"),
                CapabilityRef::new("tool_output_distillation"),
                CapabilityRef::new("skills"),
                CapabilityRef::new("infinity_context"),
                CapabilityRef::new("auto_tool_search"),
                CapabilityRef::new("budgeting"),
                CapabilityRef::new("self_budget"),
                CapabilityRef::new("stateless_todo_list"),
                CapabilityRef::new("subagents"),
                CapabilityRef::new("session_tasks"),
            ],
            Self::BashkitWorker => vec![CapabilityRef::new("bashkit_shell")],
        }
    }

    /// Flatten the same parent chain for frameworks without stored harness rows.
    pub fn effective_capabilities(self) -> Vec<CapabilityRef> {
        let mut capabilities = self
            .parent()
            .map(Self::effective_capabilities)
            .unwrap_or_default();
        capabilities.extend(self.local_capabilities());
        capabilities
    }
}

/// Local capabilities of the retired `worker-base` level.
///
/// The hosted row stays, deprecated, for existing bindings and the custom
/// harnesses adopted from examples before the tree changed. It keeps exactly
/// what it declared, under Conversation, so nothing bound to it loses a tool.
pub fn legacy_worker_base_capabilities() -> Vec<CapabilityRef> {
    vec![
        CapabilityRef::new("session_file_system"),
        CapabilityRef::new("bashkit_shell"),
        CapabilityRef::new("agent_instructions"),
        CapabilityRef::new("session"),
        CapabilityRef::new("tool_output_persistence"),
        CapabilityRef::new("tool_output_distillation"),
        CapabilityRef::with_config("parallel_tool_calls", serde_json::json!({"mode": "prefer"})),
        CapabilityRef::new("soft_approval"),
    ]
}

/// The name built-in harnesses are addressed by.
///
/// Identifies a preset by name rather than by a deployment-specific UUID.
pub const GENERIC_HARNESS_NAME: &str = "generic";

/// Capabilities composing the `generic` harness.
///
/// Ordered as the platform declared them; the order is not semantic, but
/// keeping it makes the two sides diffable by eye.
pub fn generic_capabilities() -> Vec<CapabilityRef> {
    vec![
        CapabilityRef::new("human_intent"),
        CapabilityRef::new("session_file_system"),
        CapabilityRef::new("bashkit_shell"),
        CapabilityRef::with_config(
            "web_fetch",
            serde_json::json!({"enable_file_download": true}),
        ),
        CapabilityRef::new("session_storage"),
        CapabilityRef::new("session"),
        CapabilityRef::new("session_schedule"),
        CapabilityRef::new("btw"),
        CapabilityRef::new("agent_instructions"),
        CapabilityRef::new("skills"),
        CapabilityRef::new("infinity_context"),
        CapabilityRef::new("auto_tool_search"),
        CapabilityRef::new("budgeting"),
        CapabilityRef::new("self_budget"),
        CapabilityRef::new("loop_detection"),
        // Batch independent reads/searches: request parallel tool calls where the
        // provider supports it; the local scheduler already runs batches
        // concurrently by class.
        CapabilityRef::with_config("parallel_tool_calls", serde_json::json!({"mode": "prefer"})),
        // Trusted, operator-facing default harness: show full provider error
        // detail so failures (bad key, quota, outage) are self-explanatory.
        CapabilityRef::with_config("error_disclosure", serde_json::json!({"mode": "detailed"})),
        CapabilityRef::new("message_metadata"),
        CapabilityRef::with_config(
            "compaction",
            serde_json::json!({
                "strategy": "auto",
                "proactive": true,
                "budget_percent": 0.85
            }),
        ),
        CapabilityRef::new("tool_output_persistence"),
        CapabilityRef::new("tool_output_distillation"),
        // Citations (knowledge/runtime-resources/citations.md). citation_retrieval attaches
        // claim-level citations to answers from any retrieval feed
        // (search_index / search_knowledge) — a no-op until a knowledge
        // capability is also enabled on the agent. citation_verification stamps
        // faithfulness verdicts; its default `heuristic` mode is deterministic
        // and adds no model call.
        CapabilityRef::new("citation_retrieval"),
        CapabilityRef::new("citation_verification"),
        // Enabled on the default chat harnesses by #3737, which landed while
        // this list was being extracted. Kept in that PR's position so the two
        // sides stay diffable by eye.
        CapabilityRef::new("ask_user"),
        // Soft approval, at the default `normal` level: this harness has a
        // shell, a file system, and the network, so an unattended agent can
        // delete or publish for real. The gate is guidance rather than a
        // permission wall, so it costs nothing on safe work and is overridden
        // per agent with `{"mode": "off"}`.
        CapabilityRef::new("soft_approval"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_have_distinct_effective_surfaces() {
        use BuiltInHarnessPreset::*;
        for preset in [Base, Conversation, Worker, BashkitWorker] {
            let caps = preset.effective_capabilities();
            let ids: Vec<_> = caps.iter().map(CapabilityRef::capability_id).collect();
            let unique: std::collections::HashSet<_> = ids.iter().collect();
            assert_eq!(
                ids.len(),
                unique.len(),
                "{} repeats a capability",
                preset.name()
            );
            // Every level carries the system essentials.
            for essential in [
                "compaction",
                "tool_call_repair",
                "loop_detection",
                "soft_approval",
            ] {
                assert!(
                    ids.contains(&essential),
                    "{} lacks {essential}",
                    preset.name()
                );
            }
            // Only Bashkit Worker brings compute; Worker is the no-compute kit.
            assert_eq!(ids.contains(&"bashkit_shell"), preset == BashkitWorker);
            let worker_kit = matches!(preset, Worker | BashkitWorker);
            assert_eq!(ids.contains(&"subagents"), worker_kit);
            assert_eq!(ids.contains(&"session_file_system"), worker_kit);
            // Chat affordances stay on Conversation; workers do not inherit it.
            assert_eq!(ids.contains(&"ask_user"), preset == Conversation);
            for opt_in in [
                "web_fetch",
                "session_storage",
                "session_schedule",
                "memory",
                "citation_retrieval",
            ] {
                assert!(
                    !ids.contains(&opt_in),
                    "{opt_in} leaked into {}",
                    preset.name()
                );
            }
        }
        assert!(
            !generic_capabilities()
                .iter()
                .any(|cap| cap.capability_id() == "subagents")
        );
    }

    #[test]
    fn worker_branch_skips_conversation() {
        assert_eq!(
            BuiltInHarnessPreset::Worker.parent(),
            Some(BuiltInHarnessPreset::Base)
        );
        assert_eq!(
            BuiltInHarnessPreset::BashkitWorker.parent(),
            Some(BuiltInHarnessPreset::Worker)
        );
    }

    #[test]
    fn generic_carries_no_duplicate_capability() {
        let capabilities = generic_capabilities();
        let mut ids: Vec<_> = capabilities
            .iter()
            .map(|capability| capability.capability_id().to_string())
            .collect();
        ids.sort();
        let count = ids.len();
        ids.dedup();
        assert_eq!(count, ids.len(), "a capability is listed twice");
    }

    #[test]
    fn generic_configs_are_objects() {
        // `validate_capability_config` is what the write paths enforce; a
        // preset that could not survive it would be a definition nothing can
        // actually store.
        for capability in generic_capabilities() {
            assert!(
                capability.config_value().is_object(),
                "{} carries a non-object config",
                capability.capability_id()
            );
        }
    }

    #[test]
    fn generic_names_the_shell_and_filesystem_it_promises() {
        let ids: Vec<_> = generic_capabilities()
            .iter()
            .map(|capability| capability.capability_id().to_string())
            .collect();
        for expected in ["session_file_system", "bashkit_shell", "web_fetch"] {
            assert!(ids.iter().any(|id| id == expected), "missing {expected}");
        }
    }
}
