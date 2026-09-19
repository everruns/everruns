//! Built-in harness capability sets, as data both surfaces load.
//!
//! `generic` used to be written twice: once in `crates/server/src/harnesses/`
//! for org provisioning, and again as a builder chain in every embedding
//! application. They drifted, and nothing detected it (EVE-1041).
//!
//! This crate is the shared floor because it is the one both already depend
//! on: the platform re-exports [`CapabilityRef`] as `BuiltInCapabilityDefinition`
//! and the `everruns` facade re-exports it under its own name. Putting the
//! list here means an application adopting it pulls in no platform code.
//!
//! Only the capability set is shared. The base system prompt and the hosted
//! record's presentation fields (`icon`, `starters`, `intro_markdown`) stay
//! with the platform: a written world-description drifts from the world it
//! describes, and presentation has no meaning in a library. See
//! `knowledge/framework/harnesses.md` — "What can be shared with the platform,
//! and what cannot".

use crate::CapabilityRef;

/// The name built-in harnesses are addressed by.
///
/// A name, never a UUID: `knowledge/harnesses/harness-types.md` makes that the
/// contract, and a shared definition that reintroduced a hardcoded id would
/// break it for every deployment at once.
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
