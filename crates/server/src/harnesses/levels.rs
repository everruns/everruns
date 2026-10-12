//! Canonical levels: shared capability data, hosted presentation and live parents.
//!
//! Base → Conversation | Worker → Bashkit Worker | Sandbox Worker. The tree,
//! use cases and diagram are in `docs/features/harnesses.md`; the capability
//! data is in `everruns_contracts::capability::presets`.
use crate::domains::harnesses::record::{
    BuiltInHarnessDefinition, BuiltInHarnessRole, HarnessExecution,
};
use everruns_contracts::capability::BuiltInHarnessPreset;

pub fn definition(preset: BuiltInHarnessPreset) -> BuiltInHarnessDefinition {
    let (display_name, icon, description) = match preset {
        BuiltInHarnessPreset::Base => (
            "Base",
            "square-dashed",
            "System essentials every agent needs: context compaction, error disclosure, tool-call repair, loop detection and approval guidance.",
        ),
        BuiltInHarnessPreset::Conversation => (
            "Conversation",
            "message-circle",
            "Simple chat: structured questions and message timestamps on Base. No workspace and no shell.",
        ),
        BuiltInHarnessPreset::Worker => (
            "Worker",
            "box",
            "Worker without compute: files, AGENTS.md, skills, long context, budgeting, subagents and tasks. Works through tools and MCP.",
        ),
        BuiltInHarnessPreset::BashkitWorker => (
            "Bashkit Worker",
            "terminal",
            "Worker with the Bashkit virtual shell, fixed to the recoverable Bashkit Virtual Workspace. Needs no sandbox provider.",
        ),
    };
    let mut definition = BuiltInHarnessDefinition::new(
        preset.name(),
        display_name,
        description,
        if preset == BuiltInHarnessPreset::Base {
            "You are a helpful assistant."
        } else {
            ""
        },
    )
    .with_icon(icon)
    .with_tags([preset.name(), "built-in"])
    .with_capabilities(preset.local_capabilities());
    if let Some(parent) = preset.parent() {
        definition = definition.with_parent_name(parent.name());
    }
    match preset {
        BuiltInHarnessPreset::Base => definition.with_roles([BuiltInHarnessRole::Base]),
        BuiltInHarnessPreset::Conversation => definition.with_roles([BuiltInHarnessRole::Default]),
        BuiltInHarnessPreset::Worker => definition,
        BuiltInHarnessPreset::BashkitWorker => definition
            .with_tags([preset.name(), "built-in", "environment-fixed"])
            .with_execution(HarnessExecution::FixedBashkit),
    }
}
