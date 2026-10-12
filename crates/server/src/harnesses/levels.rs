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
            "System essentials: compaction, error disclosure, tool-call repair, loop detection and approval guidance. Best for custom harnesses that pick their own tools.",
        ),
        BuiltInHarnessPreset::Conversation => (
            "Conversation",
            "message-circle",
            "Chat with structured questions and message timestamps. No workspace and no shell. Best for assistants, support bots and Q&A.",
        ),
        BuiltInHarnessPreset::Worker => (
            "Worker",
            "box",
            "Files, AGENTS.md, skills, subagents and tasks, with no shell. Best for agents that act through tools and MCP servers, such as triage and research.",
        ),
        BuiltInHarnessPreset::BashkitWorker => (
            "Bashkit Worker",
            "terminal",
            "Worker plus the Bashkit virtual shell. No sandbox provider needed, but no native packages, processes or ports. Best for light automation and scripting.",
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
