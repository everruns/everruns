//! Canonical levels: shared capability data, hosted presentation and live parents.
use crate::records::{BuiltInHarnessDefinition, BuiltInHarnessRole};
use everruns_contracts::capability::BuiltInHarnessPreset;

pub fn definition(preset: BuiltInHarnessPreset) -> BuiltInHarnessDefinition {
    let (display_name, icon, description) = match preset {
        BuiltInHarnessPreset::Base => (
            "Base",
            "square-dashed",
            "Minimal foundation with zero capabilities.",
        ),
        BuiltInHarnessPreset::Conversation => (
            "Conversation",
            "message-circle",
            "Simple conversations with context compaction and tool-call robustness.",
        ),
        BuiltInHarnessPreset::WorkerBase => (
            "Worker Base",
            "terminal",
            "Working files, bash, project instructions, session tools and durable tool output. A foundation for specialized workers.",
        ),
        BuiltInHarnessPreset::Worker => (
            "Worker",
            "box",
            "General-purpose worker with skills, long-context support, budgeting, subagents and task coordination.",
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
    if preset == BuiltInHarnessPreset::Conversation {
        definition = definition.with_roles([BuiltInHarnessRole::Default]);
    }
    if preset == BuiltInHarnessPreset::Base {
        definition = definition.with_roles([BuiltInHarnessRole::Base]);
    }
    definition
}
