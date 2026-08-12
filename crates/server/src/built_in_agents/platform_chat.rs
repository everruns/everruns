//! Platform Chat — the built-in agent behind the global chat interface.
//!
//! The IA contract is that a thread is bound to exactly one agent. Platform
//! Chat used to be bound to a harness instead, which forced the new-chat picker
//! to carry a `__platform_chat__` sentinel. Shipping it as an agent makes the
//! contract true rather than weakening it.
//!
//! The agent owns the Chat-role harness, which supplies the substantial system
//! prompt and tool surface. The agent layer stays deliberately thin: duplicating
//! the harness prompt here would give two places to edit and one to forget.

use everruns_platform::BuiltInAgentDefinition;

/// Name of the harness this agent runs on. Must match the Chat-role built-in
/// harness in `crate::harnesses::platform_chat`.
pub const HARNESS_NAME: &str = "platform-chat";

/// Agent name, unique per org. Used to resolve the agent at runtime rather than
/// hardcoding an ID.
pub const AGENT_NAME: &str = "platform-chat";

pub fn definition() -> BuiltInAgentDefinition {
    BuiltInAgentDefinition::new(
        AGENT_NAME,
        "Platform Chat",
        "Conversational agent for managing the Everruns platform.",
        SYSTEM_PROMPT,
        HARNESS_NAME,
    )
    .with_tags(["chat", "built-in"])
    // Capabilities come from the harness. Repeating them here would let the two
    // drift, and the harness is the layer that owns this agent's tool surface.
    .with_capabilities([])
}

/// Layered over the harness prompt, which carries the operating instructions.
const SYSTEM_PROMPT: &str = "\
You are the Everruns platform assistant. Follow the operating instructions \
provided by your harness.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_chat_agent_owns_the_chat_harness() {
        let definition = definition();
        assert_eq!(definition.name, AGENT_NAME);
        assert_eq!(
            definition.harness_name, HARNESS_NAME,
            "the agent must bind to the Chat-role harness, or agent-first \
             session creation would derive the wrong harness"
        );
    }

    #[test]
    fn platform_chat_agent_defers_capabilities_to_its_harness() {
        assert!(
            definition().capabilities.is_empty(),
            "capabilities belong to the harness; duplicating them here lets the \
             two drift"
        );
    }

    /// The harness definition is the one that must carry the real instructions.
    #[test]
    fn platform_chat_agent_prompt_stays_thin() {
        assert!(
            SYSTEM_PROMPT.len() < 500,
            "the agent prompt should defer to the harness, not restate it"
        );
    }
}
