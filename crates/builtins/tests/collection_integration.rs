//! Capability-collection behaviour that depends on the portable built-ins.
//!
//! These cases moved out of `crates/core/src/capabilities/mod.rs` with their
//! implementations (EVE-884): tool-search resolution and the infinity-context
//! deferral to compaction are bundle behaviour, and core cannot depend on the
//! bundle to test them.

use everruns_builtins::{
    AutoToolSearchCapability, ClaudeToolSearchCapability, DEFAULT_TOOL_SEARCH_THRESHOLD,
    InfinityContextCapability, OpenAiToolSearchCapability, TOOL_SEARCH_TOOL_NAME,
};
use everruns_core::RuntimeAgent;
use everruns_core::capabilities::{
    AgentCapabilityConfig, COMPACTION_CAPABILITY_ID, CapabilityId, CapabilityRegistry,
    CompactionCapability, SystemPromptContext, apply_capabilities, collect_capabilities,
    collect_capabilities_with_configs, collect_message_filters_only,
};
use everruns_core::typed_id::SessionId;

fn test_ctx() -> SystemPromptContext {
    SystemPromptContext::without_file_store(SessionId::new())
}

/// The product registry: kernel preset plus the portable bundle, which is what
/// these collection paths run against in a real host.
fn fixture_registry() -> CapabilityRegistry {
    let mut registry = CapabilityRegistry::with_builtins();
    everruns_builtins::register_product_builtins(&mut registry);
    registry
}

const INFINITY_CONTEXT_CAPABILITY_ID: &str = "infinity_context";

#[test]
fn test_infinity_context_defers_to_compaction_end_to_end() {
    use everruns_core::message::Message;

    let mut registry = CapabilityRegistry::new();
    registry.register(CompactionCapability);

    let tight = serde_json::json!({
        "context_budget_tokens": 1,
        "min_recent_messages": 1
    });

    // Infinity context alone (tight budget): it trims and injects a notice.
    let solo = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new(INFINITY_CONTEXT_CAPABILITY_ID),
        tight.clone(),
    )];
    let mut messages = vec![
        Message::user("task"),
        Message::assistant("old ".repeat(400)),
        Message::user("recent"),
    ];
    collect_message_filters_only(&solo, &registry).apply_post_load_filters(&mut messages);
    assert!(
        messages
            .iter()
            .any(|m| m.text().is_some_and(|t| t.contains("NOT visible"))),
        "infinity context alone should trim and notice"
    );

    // Infinity context + compaction: infinity context defers, no eviction.
    let both = vec![
        AgentCapabilityConfig::with_config(
            CapabilityId::new(INFINITY_CONTEXT_CAPABILITY_ID),
            tight,
        ),
        AgentCapabilityConfig::with_config(
            CapabilityId::new(COMPACTION_CAPABILITY_ID),
            serde_json::json!({}),
        ),
    ];
    let mut messages = vec![
        Message::user("task"),
        Message::assistant("old ".repeat(400)),
        Message::user("recent"),
    ];
    collect_message_filters_only(&both, &registry).apply_post_load_filters(&mut messages);
    assert_eq!(messages.len(), 3, "compaction owns reduction; no eviction");
    assert!(
        messages
            .iter()
            .all(|m| !m.text().is_some_and(|t| t.contains("NOT visible"))),
        "no hidden-history notice when compaction is the active reducer"
    );
}

#[tokio::test]
async fn test_apply_capabilities_openai_tool_search() {
    let registry = CapabilityRegistry::with_builtins();
    let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.4");

    let applied = apply_capabilities(
        base_runtime_agent.clone(),
        &["openai_tool_search".to_string()],
        &registry,
        &test_ctx(),
    )
    .await;

    // OpenAiToolSearchCapability provides no tools and no system prompt
    assert_eq!(
        applied.runtime_agent.system_prompt,
        base_runtime_agent.system_prompt
    );
    assert!(applied.tool_registry.is_empty());
    assert_eq!(applied.applied_ids, vec!["openai_tool_search"]);

    // tool_search config should be set on the runtime agent
    let ts = applied.runtime_agent.tool_search.as_ref().unwrap();
    assert!(ts.enabled);
    assert_eq!(ts.threshold, DEFAULT_TOOL_SEARCH_THRESHOLD);
}

#[tokio::test]
async fn test_apply_capabilities_openai_tool_search_with_other_capabilities() {
    let registry = fixture_registry();
    let base_runtime_agent = RuntimeAgent::new("You are a helpful assistant.", "gpt-5.4");

    let applied = apply_capabilities(
        base_runtime_agent,
        &[
            "current_time".to_string(),
            "openai_tool_search".to_string(),
            "test_math".to_string(),
        ],
        &registry,
        &test_ctx(),
    )
    .await;

    // Should have tools from current_time and test_math
    assert!(applied.tool_registry.has("get_current_time"));
    assert!(applied.tool_registry.has("add"));
    assert!(applied.tool_registry.has("subtract"));
    assert!(applied.tool_registry.has("multiply"));
    assert!(applied.tool_registry.has("divide"));

    // tool_search should still be configured
    let ts = applied.runtime_agent.tool_search.as_ref().unwrap();
    assert!(ts.enabled);
    assert_eq!(ts.threshold, DEFAULT_TOOL_SEARCH_THRESHOLD);
}

#[tokio::test]
async fn test_collect_capabilities_auto_tool_search_resolves_to_generic_off_native() {
    let registry = fixture_registry();

    let configs = vec![
        AgentCapabilityConfig::with_config(
            CapabilityId::new("auto_tool_search"),
            serde_json::json!({"threshold": 2}),
        ),
        AgentCapabilityConfig::with_config(CapabilityId::new("test_math"), serde_json::json!({})),
    ];

    // No native support (pre-4 Claude) → resolves to the generic client-side
    // mechanism: no hosted config, but the tool_search tool + DeferSchemaHook
    // are collected.
    let ctx = test_ctx().with_model("claude-3-5-haiku");
    let collected = collect_capabilities_with_configs(&configs, &registry, &ctx).await;

    assert!(
        collected.tool_search.is_none(),
        "auto_tool_search must not set a hosted config on a non-native model"
    );
    assert!(
        collected
            .tools
            .iter()
            .any(|t| t.name() == TOOL_SEARCH_TOOL_NAME),
        "auto_tool_search must contribute the client-side tool_search tool"
    );
    assert!(
        !collected.tool_definition_hooks.is_empty(),
        "auto_tool_search must contribute a client-side deferral hook"
    );

    let mut transformed = collected.tool_definitions.clone();
    for hook in &collected.tool_definition_hooks {
        transformed = hook.transform(transformed);
    }
    let add_tool = transformed
        .iter()
        .find(|tool| tool.name() == "add")
        .expect("test_math contributes add");
    assert!(
        add_tool.parameters().get("properties").is_none(),
        "generic auto_tool_search must honor the configured threshold"
    );
}

#[tokio::test]
async fn test_collect_capabilities_auto_tool_search_resolves_to_hosted_on_native() {
    let registry = CapabilityRegistry::with_builtins();

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("auto_tool_search"),
        serde_json::json!({"threshold": 7}),
    )];

    // Native support → resolves to the hosted OpenAI mechanism: a hosted
    // config (honoring the configured threshold) and no client-side tool/hook.
    let ctx = test_ctx().with_model("gpt-5.4");
    let collected = collect_capabilities_with_configs(&configs, &registry, &ctx).await;

    let ts = collected
        .tool_search
        .as_ref()
        .expect("auto_tool_search must set a hosted config on a native model");
    assert!(ts.enabled);
    assert_eq!(ts.threshold, 7);
    assert!(
        !collected
            .tools
            .iter()
            .any(|t| t.name() == TOOL_SEARCH_TOOL_NAME),
        "hosted mechanism must not contribute the client-side tool_search tool"
    );
    assert!(
        collected.tool_definition_hooks.is_empty(),
        "hosted mechanism must not contribute a client-side deferral hook"
    );
}

#[tokio::test]
async fn test_collect_capabilities_auto_tool_search_resolves_to_hosted_on_anthropic() {
    let registry = CapabilityRegistry::with_builtins();

    let configs = vec![AgentCapabilityConfig::with_config(
        CapabilityId::new("auto_tool_search"),
        serde_json::json!({"threshold": 9}),
    )];

    // Native Claude support → resolves to the hosted Anthropic mechanism: a
    // hosted config (honoring the threshold) and no client-side tool/hook.
    let ctx = test_ctx().with_model("claude-opus-4-8");
    let collected = collect_capabilities_with_configs(&configs, &registry, &ctx).await;

    let ts = collected
        .tool_search
        .as_ref()
        .expect("auto_tool_search must set a hosted config on a native Claude model");
    assert!(ts.enabled);
    assert_eq!(ts.threshold, 9);
    assert!(
        !collected
            .tools
            .iter()
            .any(|t| t.name() == TOOL_SEARCH_TOOL_NAME),
        "hosted mechanism must not contribute the client-side tool_search tool"
    );
    assert!(
        collected.tool_definition_hooks.is_empty(),
        "hosted mechanism must not contribute a client-side deferral hook"
    );
}
