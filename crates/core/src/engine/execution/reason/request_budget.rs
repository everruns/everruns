//! Approximate request budgeting after tool discovery and schema projection.
//! Provider wrappers/tokenizers differ; this is a byte/4 estimate,
//! not provider-reported usage or an exact tokenizer. Never count private deferred full schemas.

use crate::engine::compaction_policy::CompactionPolicy;
use crate::engine::driver_registry::{LlmCallConfig, Message, MessageRole};
use crate::engine::error::{AgentLoopError, Result};
use crate::engine::tool_types::ToolDefinition;

fn tool_tokens(tool: &ToolDefinition) -> usize {
    let projected = tool.parameters_for_model();
    let normalized = everruns_contracts::tool_schema_compat::strict_openai_tool_schema(&projected);
    let parameters = normalized
        .as_ref()
        .filter(|schema| schema.to_string().len() > projected.to_string().len())
        .unwrap_or(&projected);
    // Include a function envelope as well as the schema. Flat provider formats
    // use fewer bytes; omitting the envelope would undercount large catalogs.
    let definition = serde_json::json!({
        "type": "function",
        "function": {
            "name": tool.name(),
            "description": tool.description(),
            "parameters": parameters,
            "strict": normalized.is_some(),
        },
    });
    // Value serialization cannot fail. Round up rather than losing small schemas.
    definition.to_string().len().div_ceil(4)
}

pub(super) fn tools_tokens(tools: &[ToolDefinition]) -> usize {
    tools.iter().map(tool_tokens).fold(0, usize::saturating_add)
}

pub(super) fn tool_tokens_for_request(config: &LlmCallConfig, messages: &[Message]) -> usize {
    let current = tools_tokens(&config.tools);
    let Some(baseline) = &config.tool_baseline else {
        return current;
    };
    // Inline-capable drivers replay the baseline plus every addition/redefinition;
    // other endpoints fall back to the current declarations. Reserve the larger
    // representation without guessing endpoint-specific inline eligibility.
    let replay = messages
        .iter()
        .filter_map(|m| m.tool_changes.as_ref())
        .fold(tools_tokens(baseline), |tokens, changes| {
            tokens
                .saturating_add(tools_tokens(&changes.added))
                .saturating_add(tools_tokens(&changes.redefined))
                .saturating_add(changes.note.as_ref().map_or(0, |s| s.len().div_ceil(4)))
                .saturating_add(
                    changes
                        .removed
                        .iter()
                        .map(|s| s.len().div_ceil(4))
                        .sum::<usize>(),
                )
        });
    current.max(replay)
}

pub(super) fn system_tokens(policy: &dyn CompactionPolicy, messages: &[Message]) -> usize {
    messages
        .iter()
        .filter(|m| m.role == MessageRole::System)
        .map(|m| policy.estimate_total_tokens(std::slice::from_ref(m)))
        .fold(0, usize::saturating_add)
}

/// Soft compaction budgets must never cause the immutable prefix to disappear.
/// Reject an impossible prefix against the hard window, not the soft threshold.
pub(super) fn history_budget(
    policy: &dyn CompactionPolicy,
    messages: &[Message],
    tool_tokens: usize,
    context_window: usize,
    budget_percent: f32,
) -> Result<usize> {
    let system_tokens = system_tokens(policy, messages);
    if tool_tokens.saturating_add(system_tokens) >= context_window {
        return Err(AgentLoopError::request_too_large(format!(
            "Tool declarations and system instructions require approximately {} tokens, exceeding the {}-token context window. Reduce the active tool schemas or system instructions, or use a larger-context model; history compaction cannot remove this fixed input.",
            tool_tokens.saturating_add(system_tokens),
            context_window,
        )));
    }
    Ok(((context_window as f32 * budget_percent) as usize)
        .saturating_sub(tool_tokens)
        .max(system_tokens.saturating_add(1)))
}

#[cfg(all(test, feature = "builtins"))]
mod tests {
    use super::*;
    use crate::builtins::{Capability, CompactionCapability};
    use serde_json::json;

    fn policy() -> std::sync::Arc<dyn CompactionPolicy> {
        CompactionCapability
            .compaction_policy(&json!({}))
            .expect("policy")
    }

    fn tool(description: &str) -> ToolDefinition {
        ToolDefinition::function("lookup", description, json!({"type": "object"}))
    }

    #[test]
    fn empty_and_deferred_catalogs_only_count_model_visible_schemas() {
        let mut config = LlmCallConfig::new("test");
        assert_eq!(tool_tokens_for_request(&config, &[]), 0);
        let mut deferred = tool("Lookup");
        if let ToolDefinition::ClientSide(t) = &mut deferred {
            t.full_parameters = Some(json!({"description": "secret".repeat(10_000)}));
        }
        config.tools.push(deferred);
        let small = tool_tokens_for_request(&config, &[]);
        assert!(small > 0 && small < 100);
        if let ToolDefinition::ClientSide(t) = &mut config.tools[0] {
            t.parameters = t.full_parameters.take().expect("full schema");
        }
        assert!(tool_tokens_for_request(&config, &[]) > small + 10_000);
    }

    #[test]
    fn request_pressure_includes_tools_and_respects_disabled_proactive() {
        let messages = vec![Message::text(MessageRole::User, "hello")];
        assert!(!policy().should_compact_with_fixed_input(&messages, 1_000, 0));
        assert!(policy().should_compact_with_fixed_input(&messages, 1_000, 900));
        let disabled = CompactionCapability
            .compaction_policy(&json!({"proactive": false}))
            .expect("policy");
        assert!(!disabled.should_compact_with_fixed_input(&messages, 1_000, 900));
    }

    #[test]
    fn fixed_prefix_overflow_is_explicit_and_never_saturates_to_a_zero_budget() {
        let p = policy();
        assert!(history_budget(p.as_ref(), &[], 1_000, 1_000, 0.85).is_err());
        assert!(history_budget(p.as_ref(), &[], usize::MAX, 1_000, 0.85).is_err());
        let messages = vec![Message::text(MessageRole::System, "s".repeat(400))];
        assert!(history_budget(p.as_ref(), &messages, 900, 1_000, 0.85).is_err());
        // A prefix above the soft threshold but below the hard window survives.
        assert_eq!(
            history_budget(p.as_ref(), &messages, 800, 1_000, 0.85).expect("fits"),
            101
        );
    }

    #[test]
    fn trimming_reserves_schema_budget_and_keeps_system_instructions() {
        let p = policy();
        let messages = vec![
            Message::text(MessageRole::System, "s".repeat(200)),
            Message::text(MessageRole::User, "x".repeat(400)),
            Message::text(MessageRole::Assistant, "y".repeat(400)),
        ];
        let budget = history_budget(p.as_ref(), &messages, 650, 1_000, 0.85).expect("fits");
        assert_eq!(budget, 200);
        let trimmed = p.aggressive_trim(&messages, budget, true);
        assert_eq!(trimmed[0].role, MessageRole::System);
        assert!(p.estimate_total_tokens(&trimmed) + 650 <= 850);
        assert!(trimmed.len() < messages.len());
    }

    #[test]
    fn inline_replay_is_reserved_without_double_counting_current_tools() {
        let mut config = LlmCallConfig::new("test");
        config.tools = vec![tool("current")];
        let current = tool_tokens_for_request(&config, &[]);
        config.tool_baseline = Some(config.tools.clone());
        assert_eq!(tool_tokens_for_request(&config, &[]), current);
        let mut message = Message::text(MessageRole::User, "reveal");
        message.tool_changes = Some(everruns_contracts::tool_changes::ToolChanges {
            redefined: vec![tool(&"large description ".repeat(100))],
            ..Default::default()
        });
        assert!(tool_tokens_for_request(&config, &[message]) > current + 400);
    }
    struct SmallWindowDriver;
    #[async_trait::async_trait]
    impl crate::engine::ChatDriver for SmallWindowDriver {
        fn effective_context_window(&self, _: &str) -> Option<usize> {
            Some(1_000)
        }
        async fn chat_completion_stream(
            &self,
            _: &crate::engine::ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> Result<crate::engine::driver_registry::LlmResponseStream> {
            panic!("compaction must not make a provider call")
        }
    }

    async fn compact(messages: &mut Vec<Message>, config: &mut LlmCallConfig) -> Result<()> {
        use super::super::{ProactiveCompactionContext, apply_proactive_compaction};
        let p = policy();
        let emitter = crate::engine::test_fixtures::TestEventEmitter::new();
        let event_context = crate::engine::events::EventContext::default();
        let sequences = std::collections::HashMap::new();
        apply_proactive_compaction(
            ProactiveCompactionContext {
                chat_driver: &SmallWindowDriver,
                policy: p.as_ref(),
                checkpoint_store: None,
                event_emitter: &emitter,
                event_context: &event_context,
                session_id: crate::engine::typed_id::SessionId::new(),
                message_source_sequence: None,
                provider_type: "test",
                model: "test",
                system_prompt: Some("system"),
                stateful_response_continuation: false,
                checkpoint_suffix_message_count: 0,
                raw_tool_result_bytes: 0,
                prior_usage: None,
                history: &[],
                message_sequences: &sequences,
                current_floor: None,
            },
            messages,
            config,
        )
        .await
        .map(|_| ())
    }

    #[tokio::test]
    async fn proactive_flow_trims_for_offered_schemas_before_the_provider_call() {
        let mut config = LlmCallConfig::new("test");
        config.tools = vec![tool(&"x".repeat(2_400))];
        let mut messages = vec![
            Message::text(MessageRole::System, "system"),
            Message::text(MessageRole::User, "u".repeat(400)),
            Message::text(MessageRole::Assistant, "a".repeat(400)),
            Message::text(MessageRole::User, "v".repeat(400)),
        ];
        assert!(policy().estimate_total_tokens(&messages) < 850);
        compact(&mut messages, &mut config).await.expect("trim");
        assert!(messages.len() < 4);
        assert_eq!(messages[0].role, MessageRole::System);
        assert!(policy().estimate_total_tokens(&messages) + tools_tokens(&config.tools) <= 850);
    }

    #[tokio::test]
    async fn historical_schema_overflow_rebases_instead_of_rejecting_current_request() {
        let mut config = LlmCallConfig::new("test");
        config.tools = vec![tool("small")];
        config.tool_baseline = Some(vec![tool(&"old".repeat(2_000))]);
        config.system_prompt_baseline = Some("old instructions".into());
        let mut messages = vec![
            Message::text(MessageRole::System, "system"),
            Message::text(MessageRole::User, "hello"),
        ];
        compact(&mut messages, &mut config)
            .await
            .expect("rebase fits");
        assert_eq!(messages.len(), 2);
        assert!(config.tool_baseline.is_none());
        assert!(config.system_prompt_baseline.is_none());
    }

    #[tokio::test]
    async fn impossible_current_schema_fails_without_mutating_history() {
        let mut config = LlmCallConfig::new("test");
        config.tools = vec![tool(&"x".repeat(5_000))];
        let mut messages = vec![
            Message::text(MessageRole::System, "system"),
            Message::text(MessageRole::User, "hello"),
        ];
        let error = compact(&mut messages, &mut config)
            .await
            .expect_err("fixed input overflow");
        assert!(error.is_request_too_large());
        assert_eq!(messages.len(), 2);
    }
    #[test]
    fn strict_schema_expansion_and_catalog_changes_are_accounted_for() {
        let declaration = ToolDefinition::function(
            "query",
            "Lookup",
            json!({
                "type": "object", "properties": {"optional": {"type": "string"}}
            }),
        );
        let raw = json!({"name": declaration.name(), "description": declaration.description(),
            "parameters": declaration.parameters_for_model()})
        .to_string()
        .len()
        .div_ceil(4);
        assert!(tool_tokens(&declaration) > raw);
        use super::super::proactive_source_fingerprint;
        let original = proactive_source_fingerprint(None, &[], &[]);
        let with_tool = proactive_source_fingerprint(None, &[], std::slice::from_ref(&declaration));
        assert_ne!(original, with_tool);
        assert_ne!(
            with_tool,
            proactive_source_fingerprint(None, &[], &[tool("changed")])
        );
    }
}
