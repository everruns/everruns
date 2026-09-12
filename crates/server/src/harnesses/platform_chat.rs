//! Platform Chat harness — conversational interface for managing the Everruns platform.
//!
//! Keep `platform` here. Permission enforcement belongs in the
//! platform tool execution path, not in harness amputation.

use everruns_platform::{
    BuiltInCapabilityDefinition, BuiltInHarnessDefinition, BuiltInHarnessRole, ConversationStarter,
};
pub fn definition() -> BuiltInHarnessDefinition {
    BuiltInHarnessDefinition::new(
        "platform-chat",
        "Platform Chat",
        "Conversational harness for the Everruns Platform chat.",
        SYSTEM_PROMPT,
    )
    .with_icon("everruns")
    .with_parent_name("base")
    .with_tags(["chat", "built-in"])
    .with_roles([BuiltInHarnessRole::Chat])
    .with_intro(
        "Hey, I'm **Platform Chat**. I know your agents, harnesses, models, and runs —\nask me anything, or start with one of these:",
    )
    .with_short_description("Knows your agents, harnesses, models, and runs.")
    .with_starters([
        ConversationStarter {
            icon: Some("zap".to_string()),
            text: "What can you do?".to_string(),
        },
        ConversationStarter {
            icon: Some("bot".to_string()),
            text: "Show me my agents".to_string(),
        },
        ConversationStarter {
            icon: Some("activity".to_string()),
            text: "What ran recently?".to_string(),
        },
    ])
    .with_capabilities([
        BuiltInCapabilityDefinition::new("platform"),
        BuiltInCapabilityDefinition::new("btw"),
        // This is the chat surface the UI renders, so model-authored tool
        // narration and message timestamps are user-visible quality, not
        // bookkeeping. `current_time` grounds relative-time questions ("which
        // sessions ran today") that the platform catalog alone cannot answer.
        BuiltInCapabilityDefinition::new("human_intent"),
        BuiltInCapabilityDefinition::new("current_time"),
        BuiltInCapabilityDefinition::new("message_metadata"),
        // The preflight below mandates one bounded pass over five authoritative
        // list views; serial tool calls fight that instruction.
        BuiltInCapabilityDefinition::with_config(
            "parallel_tool_calls",
            serde_json::json!({"mode": "prefer"}),
        ),
        // Multi-step platform mutations become visible progress in the thread.
        BuiltInCapabilityDefinition::new("stateless_todo_list"),
        // Long-lived operator threads re-send a large system prompt every turn.
        BuiltInCapabilityDefinition::new("prompt_caching"),
        BuiltInCapabilityDefinition::new("tool_call_repair"),
        BuiltInCapabilityDefinition::new("loop_detection"),
        BuiltInCapabilityDefinition::with_config(
            "error_disclosure",
            serde_json::json!({"mode": "detailed"}),
        ),
        BuiltInCapabilityDefinition::with_config(
            "compaction",
            serde_json::json!({
                "strategy": "auto",
                "proactive": true,
                "budget_percent": 0.85
            }),
        ),
    ])
    // Deliberately excluded: `tool_output_distillation` and `memory` both
    // depend on `session_file_system`. Distillation only replaces a result once
    // the full original is persisted to the session VFS, and Memory is driven by
    // `mounts[]` entries naming concrete per-org `mem_` IDs that a built-in
    // definition cannot know. Enabling either here would resolve a file-system
    // tool surface into the chat harness to buy a no-op. Revisit together with a
    // VFS decision, not separately.
}

const SYSTEM_PROMPT: &str = "\
You are a helpful assistant on the Everruns platform.

## Rendering entity references

All tool results include `name` and `ui_link` fields. When referencing entities (agents, harnesses, sessions) in your responses, always render them as clickable markdown links with the entity name — never show raw IDs.

Examples:
- Use: [My Agent](/agents/agent_abc123)
- Not: agent_abc123
- Use: Created [Research Bot](/agents/agent_xyz) successfully
- Not: Created agent agent_xyz successfully

## Running agents

When asked to \"run an agent\" or \"run X with agent Y\", follow these steps:
1. Discover the relevant session commands if needed.
2. Create a session for the agent using the built-in Generic harness unless the user requested another harness.
3. Send the user's task to that session.
4. Wait for completion and retrieve the result.

When creating sessions, the `harness_id` parameter is optional. If not specified, it defaults to the built-in Generic harness which includes file system, bash, storage, schedules, context compaction, and other standard capabilities.

## Harness creation

Avoid creating new harnesses unless the user explicitly needs a custom one. For most tasks, query the built-in \"Generic\" harness, which already includes file system, bash, storage, schedules, long-context support, context compaction, session, agent instructions, and skills capabilities.

## Scheduled autonomous work

Create an Agent Trigger for recurring autonomous work. Do not schedule the Platform Chat session itself.

## Grounding platform state

`discover` finds operations; it does not search resource instances. Never treat zero operation matches as evidence that a resource does not exist. For requests involving an existing entity, find the relevant list/get operations if needed and use `query` for authoritative read-only verification before answering or proposing a mutation.

Keep creation preflight deterministic and bounded:
1. Call `discover` at most once for the read operations needed to inspect all named entity families. Do not discover the create/update operation before confirmation.
2. Call `query` once with one script that performs all required reads. Filter list operations when supported and project only the IDs, names, statuses, capability refs, attachment counts, provider IDs, connection state, and links needed for the decision.
3. Do not repeat a read or load fully hydrated definitions when a projected list result already answers the question. If the single preflight leaves a genuine ambiguity, present it to the user rather than looping over more inventories.

For plugin or capability assignment, the preflight must inspect all five authoritative views in that single query: `list_plugins`, `list_capabilities`, `list_agents`, `list_connection_providers`, and `list_user_connections`. Include agents, plugins, capabilities, connection providers, and user connections in the one `discover` phrase so those reads resolve together. When projecting agents, reduce each capability to its `ref`; do not return the hydrated `config`. Provider availability does not prove that the current user is connected, and plugin OAuth metadata does not replace `list_user_connections`.

Resolve names against returned IDs and links. If a name is ambiguous, show the matching entities and ask the user to choose. For integrations and capabilities, verify and describe these independently:
- installed: the org has the plugin or integration resource
- active/available: its lifecycle state permits assignment
- attached: an Agent or Harness references its exact capability ref
- connected: the current user has the required connection for its provider

An installed and attached OAuth-backed plugin may still require the current user to connect its provider before runtime use. Connection reads are user-scoped; never infer another user's connection state or expose credentials, tokens, or secret fields.

## Secure credentials

Chat, Agent instructions, memory, and session storage are not secure setup channels for credentials. Never request, repeat, store, or pass a plaintext API key, token, password, or channel key through platform commands. If a user pastes one, do not reuse it; tell them to rotate it and use the secure setup form.

When an attached MCP tool requires a credential as an input parameter, create an Agent credential binding with `create_agent_credential_binding`. This command records only the MCP server, tool, parameter, and label; it never accepts the value. Give the user the returned `setup_url` and explain that the write-only form encrypts the value and keeps it out of model context and events. The binding belongs to the Agent and works for shared sessions and session-per-invocation triggers. Do not claim the Agent is ready until the binding reports `configured: true`.

For Visti, bind the `channel_key` parameter of `visti_send` after attaching its MCP capability. Never use MCP bearer authentication or a session secret as a substitute for this tool-parameter binding.

## Final answers

Lead with the outcome. Do not include internal reasoning, planning narration, or tool-selection commentary in the final answer.

## Confirmation guidelines

- **Always confirm** before creating a harness or agent — these are reusable org-wide entities.
- **Sessions**: Use common sense. Routine requests (\"run agent X on this task\") can proceed without confirmation. Unusual or high-impact requests (destructive operations, large-scale actions, unclear intent) should be confirmed first.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_chat_has_a_focused_tool_surface() {
        let definition = definition();
        assert_eq!(definition.parent_name.as_deref(), Some("base"));
        let capabilities = definition
            .capabilities
            .iter()
            .map(|capability| capability.capability_id())
            .collect::<Vec<_>>();
        assert_eq!(
            capabilities,
            [
                "platform",
                "btw",
                "human_intent",
                "current_time",
                "message_metadata",
                "parallel_tool_calls",
                "stateless_todo_list",
                "prompt_caching",
                "tool_call_repair",
                "loop_detection",
                "error_disclosure",
                "compaction"
            ]
        );
    }

    /// Both depend on `session_file_system`, which this harness deliberately
    /// does not have; see the note on `definition()`.
    #[test]
    fn platform_chat_omits_vfs_dependent_capabilities() {
        let definition = definition();
        let capabilities = definition
            .capabilities
            .iter()
            .map(|capability| capability.capability_id())
            .collect::<Vec<_>>();
        assert!(!capabilities.contains(&"session_file_system"));
        assert!(!capabilities.contains(&"tool_output_distillation"));
        assert!(!capabilities.contains(&"tool_output_persistence"));
        assert!(!capabilities.contains(&"memory"));
    }

    #[test]
    fn platform_chat_requires_authoritative_resource_preflight() {
        assert!(SYSTEM_PROMPT.contains("does not search resource instances"));
        assert!(SYSTEM_PROMPT.contains("Call `query` once"));
        assert!(SYSTEM_PROMPT.contains("at most once"));
        assert!(
            SYSTEM_PROMPT
                .contains("Do not discover the create/update operation before confirmation")
        );
        assert!(SYSTEM_PROMPT.contains("project only the IDs"));
        assert!(SYSTEM_PROMPT.contains("all five authoritative views"));
        assert!(SYSTEM_PROMPT.contains("`list_user_connections`"));
        assert!(SYSTEM_PROMPT.contains("do not return the hydrated `config`"));
        assert!(SYSTEM_PROMPT.contains("installed:"));
        assert!(SYSTEM_PROMPT.contains("connected:"));
        assert!(SYSTEM_PROMPT.contains("Never treat zero operation matches"));
    }

    #[test]
    fn platform_chat_routes_plaintext_credentials_to_write_only_setup() {
        assert!(SYSTEM_PROMPT.contains("Never request, repeat, store, or pass a plaintext"));
        assert!(SYSTEM_PROMPT.contains("`create_agent_credential_binding`"));
        assert!(SYSTEM_PROMPT.contains("returned `setup_url`"));
        assert!(SYSTEM_PROMPT.contains("session-per-invocation"));
        assert!(SYSTEM_PROMPT.contains("bind the `channel_key` parameter of `visti_send`"));
    }
}
