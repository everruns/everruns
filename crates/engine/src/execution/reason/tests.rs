use super::compaction::materially_reduced;
use super::*;
use crate::driver_registry::{LlmCallConfig, PromptCacheConfig, PromptCacheStrategy};
use crate::events::CapabilityUsageKind;
use everruns_provider::reasoning::{ReasoningContentPart, ReasoningText};
use serde_json::json;
use std::collections::HashMap;

#[test]
fn compaction_cost_preserves_estimated_generation_cost() {
    let mut usage = TokenUsage::new(1_000_000, 500_000).with_cost(None, Some(7.5));

    add_compaction_cost(&mut usage, 0.0125);

    assert_eq!(usage.actual_cost_usd, None);
    assert_eq!(usage.estimated_cost_usd, Some(7.5));
    assert_eq!(usage.effective_cost_usd(), Some(7.5125));
}

#[test]
fn compaction_cost_combines_with_actual_generation_cost() {
    let mut usage = TokenUsage::new(10, 5).with_cost(Some(0.25), Some(0.2));

    add_compaction_cost(&mut usage, 0.0125);

    assert_eq!(usage.actual_cost_usd, Some(0.2625));
    assert_eq!(usage.effective_cost_usd(), Some(0.2625));
}

#[test]
fn material_reduction_requires_five_percent_at_normal_sizes() {
    assert!(!materially_reduced(1_000, 951));
    assert!(materially_reduced(1_000, 950));
}

#[test]
fn material_reduction_uses_absolute_floor_for_small_sizes() {
    assert!(!materially_reduced(0, 0));
    assert!(!materially_reduced(100, 69));
    assert!(materially_reduced(100, 68));
}

struct BlockWhenDeltaContains {
    needle: &'static str,
}

impl crate::output_guardrail::OutputGuardrailRun for BlockWhenDeltaContains {
    fn check(
        &mut self,
        _accumulated: &str,
        delta: &str,
    ) -> crate::output_guardrail::GuardrailDecision {
        if delta.contains(self.needle) {
            crate::output_guardrail::GuardrailDecision::block("test_leak", "[blocked]")
        } else {
            crate::output_guardrail::GuardrailDecision::Pass
        }
    }
}

fn test_armed_guardrail() -> ArmedGuardrail {
    ArmedGuardrail {
        capability_id: "test_capability".to_string(),
        guardrail_id: "test_guardrail".to_string(),
        run: Box::new(BlockWhenDeltaContains { needle: "secret" }),
    }
}

#[test]
fn test_append_guarded_thinking_delta_blocks_before_pending_emit() {
    let mut guardrails = vec![test_armed_guardrail()];
    let mut thinking = "safe ".to_string();
    let mut pending = "safe ".to_string();

    let tripped = append_guarded_thinking_delta(
        &mut guardrails,
        &mut thinking,
        &mut pending,
        "secret instructions",
    )
    .expect("thinking delta should trip guardrail");

    assert_eq!(tripped.capability_id, "test_capability");
    assert_eq!(tripped.guardrail_id, "test_guardrail");
    assert_eq!(tripped.block.reason_code, "test_leak");
    assert_eq!(tripped.block.replacement, "[blocked]");
    assert_eq!(thinking, "safe secret instructions");
    assert!(pending.is_empty());
}

#[test]
fn test_append_guarded_thinking_delta_allows_safe_pending_emit() {
    let mut guardrails = vec![test_armed_guardrail()];
    let mut thinking = String::new();
    let mut pending = String::new();

    let tripped = append_guarded_thinking_delta(
        &mut guardrails,
        &mut thinking,
        &mut pending,
        "ordinary reasoning",
    );

    assert!(tripped.is_none());
    assert_eq!(thinking, "ordinary reasoning");
    assert_eq!(pending, "ordinary reasoning");
}

#[test]
fn test_reason_result_default() {
    let result = ReasonResult::default();
    assert!(!result.success);
    assert!(result.text.is_empty());
    assert!(result.tool_calls.is_empty());
    assert!(!result.has_tool_calls);
    // Default derive gives 0, but serde deserialization gives 100 via default_max_iterations()
    assert_eq!(result.max_iterations, 0);
}

#[test]
fn test_reason_result_serde_default() {
    // Test that serde uses the default_max_iterations function
    let json = r#"{"success":true,"text":"","has_tool_calls":false}"#;
    let result: ReasonResult = serde_json::from_str(json).unwrap();
    assert_eq!(result.max_iterations, 500);
}

#[test]
fn test_capability_usage_snapshot_keeps_resolved_and_exposed_separate() {
    let registry = CapabilityRegistry::new();
    let tool = ToolDefinition::Builtin(crate::tool_types::BuiltinTool {
        name: "demo_tool".to_string(),
        display_name: None,
        description: "demo".to_string(),
        parameters: json!({"type": "object"}),
        policy: crate::tool_types::ToolPolicy::Auto,
        category: None,
        deferrable: crate::tool_types::DeferrablePolicy::default(),
        hints: crate::tool_types::ToolHints::default(),
        full_parameters: None,
    })
    .with_capability_attribution("cap:demo", Some("Demo Capability"));

    let records = capability_usage_snapshot_records(
        &registry,
        &[crate::CapabilityRef::new("current_time")],
        &[tool],
    );

    assert!(records.iter().any(|record| {
        matches!(record.usage_kind, CapabilityUsageKind::Resolved)
            && record.capability_id == "current_time"
            && record.tool_name.is_none()
    }));
    assert!(records.iter().any(|record| {
        matches!(record.usage_kind, CapabilityUsageKind::Exposed)
            && record.capability_id == "cap:demo"
            && record.tool_name.as_deref() == Some("demo_tool")
    }));
}

#[test]
fn stream_stall_deadline_ignores_empty_keepalive_events() {
    assert!(!advances_stall_deadline(&LlmStreamEvent::TextDelta(
        String::new()
    )));
    assert!(!advances_stall_deadline(&LlmStreamEvent::ReasoningDelta {
        delta: String::new(),
        summary: false,
    }));
    // An artifact with neither replay state nor readable text is not progress.
    assert!(!advances_stall_deadline(&LlmStreamEvent::ReasoningItem(
        ReasoningContentPart::opaque("openai")
    )));
}

#[test]
fn stream_stall_deadline_advances_on_output_progress() {
    assert!(advances_stall_deadline(&LlmStreamEvent::TextDelta(
        "hello".to_string()
    )));
    assert!(advances_stall_deadline(&LlmStreamEvent::ReasoningDelta {
        delta: "thinking".to_string(),
        summary: false,
    }));
    // Replay state alone is progress: the provider produced something the next
    // request must carry, even with nothing readable to show.
    assert!(advances_stall_deadline(&LlmStreamEvent::ReasoningItem(
        ReasoningContentPart::opaque("openai")
            .with_item_id("item_1")
            .with_encrypted("encrypted")
    )));
    assert!(advances_stall_deadline(&LlmStreamEvent::ReasoningItem(
        ReasoningContentPart::opaque("openai").with_text(ReasoningText::Summary {
            parts: vec!["summary".to_string()],
        })
    )));
    assert!(advances_stall_deadline(&LlmStreamEvent::ToolCalls(vec![
        ToolCall {
            id: "call_1".to_string(),
            name: "demo".to_string(),
            arguments: json!({}),
        }
    ])));
}

#[tokio::test]
async fn test_repair_dangling_tool_calls_no_tool_calls() {
    use crate::events::EventContext;
    use crate::typed_id::SessionId;
    let messages = vec![Message::user("Hello"), Message::assistant("Hi there!")];
    let emitter = crate::test_fixtures::NoopEventEmitter;
    let session_id = SessionId::new();
    let ctx = EventContext::empty();
    let patched =
        repair_dangling_tool_calls(&messages, None, &emitter, session_id, &ctx, "turn_01").await;
    assert_eq!(patched.len(), 2);
}

#[tokio::test]
async fn test_repair_dangling_tool_calls_with_result() {
    use crate::events::EventContext;
    use crate::typed_id::SessionId;
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        name: "get_weather".to_string(),
        arguments: serde_json::json!({"city": "NYC"}),
    };

    let messages = vec![
        Message::user("What's the weather?"),
        Message::assistant_with_tools("Let me check", vec![tool_call]),
        Message::tool_result("call_123", Some(serde_json::json!({"temp": 72})), None),
    ];

    let emitter = crate::test_fixtures::NoopEventEmitter;
    let session_id = SessionId::new();
    let ctx = EventContext::empty();
    let patched =
        repair_dangling_tool_calls(&messages, None, &emitter, session_id, &ctx, "turn_01").await;
    assert_eq!(patched.len(), 3);
}

#[tokio::test]
async fn test_repair_dangling_tool_calls_missing_result_no_store() {
    use crate::events::EventContext;
    use crate::typed_id::SessionId;
    let tool_call = ToolCall {
        id: "call_456".to_string(),
        name: "search_web".to_string(),
        arguments: serde_json::json!({"query": "rust"}),
    };

    let messages = vec![
        Message::user("Search for rust"),
        Message::assistant_with_tools("Searching...", vec![tool_call]),
        Message::user("Actually, never mind"),
    ];

    let emitter = crate::test_fixtures::NoopEventEmitter;
    let session_id = SessionId::new();
    let ctx = EventContext::empty();
    let patched =
        repair_dangling_tool_calls(&messages, None, &emitter, session_id, &ctx, "turn_01").await;
    // Should have added a cancelled result
    assert_eq!(patched.len(), 4);
    assert_eq!(patched[2].role, MessageRole::ToolResult);
    assert_eq!(patched[2].tool_call_id(), Some("call_456"));
}

#[tokio::test]
async fn test_repair_dangling_tool_calls_settled_result_replayed() {
    use crate::events::EventContext;
    use crate::typed_id::SessionId;
    use crate::{
        durability::DurableToolCallStatus, durability::DurableToolResultStore,
        durability::ToolCallClaimResult,
    };

    struct MockSettledStore;
    #[async_trait::async_trait]
    impl DurableToolResultStore for MockSettledStore {
        async fn try_claim_tool_call(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> crate::error::Result<ToolCallClaimResult> {
            Ok(ToolCallClaimResult::Claimed {
                claim_token: uuid::Uuid::new_v4(),
            })
        }
        async fn settle_tool_call(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
            _: &str,
            _: uuid::Uuid,
        ) -> crate::error::Result<bool> {
            Ok(true)
        }
        async fn get_tool_call_status(
            &self,
            _turn_id: &str,
            _tool_call_id: &str,
        ) -> crate::error::Result<Option<DurableToolCallStatus>> {
            Ok(Some(DurableToolCallStatus::Settled {
                result_json: serde_json::json!({
                    "tool_call_id": "call_789",
                    "result": {"answer": 42},
                    "error": null,
                    "images": null,
                    "connection_required": null,
                    "raw_output": null
                }),
            }))
        }
    }

    let tool_call = ToolCall {
        id: "call_789".to_string(),
        name: "compute".to_string(),
        arguments: serde_json::json!({"x": 21}),
    };
    let messages = vec![
        Message::user("Compute"),
        Message::assistant_with_tools("Computing...", vec![tool_call]),
    ];

    let store = MockSettledStore;
    let emitter = crate::test_fixtures::NoopEventEmitter;
    let session_id = SessionId::new();
    let ctx = EventContext::empty();
    let patched = repair_dangling_tool_calls(
        &messages,
        Some(&store as &dyn DurableToolResultStore),
        &emitter,
        session_id,
        &ctx,
        "turn_01",
    )
    .await;

    // Settled result should be replayed (not cancelled message)
    assert_eq!(patched.len(), 3);
    assert_eq!(patched[2].role, MessageRole::ToolResult);
    assert_eq!(patched[2].tool_call_id(), Some("call_789"));
}

#[tokio::test]
async fn test_repair_dangling_tool_calls_interrupted_result_replayed() {
    use crate::events::EventContext;
    use crate::typed_id::SessionId;
    use crate::{
        durability::DurableToolCallStatus, durability::DurableToolResultStore,
        durability::ToolCallClaimResult,
    };

    struct MockInterruptedStore;
    #[async_trait::async_trait]
    impl DurableToolResultStore for MockInterruptedStore {
        async fn try_claim_tool_call(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> crate::error::Result<ToolCallClaimResult> {
            Ok(ToolCallClaimResult::Claimed {
                claim_token: uuid::Uuid::new_v4(),
            })
        }
        async fn settle_tool_call(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
            _: &str,
            _: uuid::Uuid,
        ) -> crate::error::Result<bool> {
            Ok(true)
        }
        async fn get_tool_call_status(
            &self,
            _turn_id: &str,
            _tool_call_id: &str,
        ) -> crate::error::Result<Option<DurableToolCallStatus>> {
            Ok(Some(DurableToolCallStatus::Interrupted {
                result_json: None,
            }))
        }
    }

    let tool_call = ToolCall {
        id: "call_int".to_string(),
        name: "slow_op".to_string(),
        arguments: serde_json::json!({}),
    };
    let messages = vec![
        Message::user("Do it"),
        Message::assistant_with_tools("Doing...", vec![tool_call]),
    ];

    let store = MockInterruptedStore;
    let emitter = crate::test_fixtures::NoopEventEmitter;
    let session_id = SessionId::new();
    let ctx = EventContext::empty();
    let patched = repair_dangling_tool_calls(
        &messages,
        Some(&store as &dyn DurableToolResultStore),
        &emitter,
        session_id,
        &ctx,
        "turn_01",
    )
    .await;

    assert_eq!(patched.len(), 3);
    let repair = &patched[2];
    assert_eq!(repair.role, MessageRole::ToolResult);
    assert_eq!(repair.tool_call_id(), Some("call_int"));
    // Interrupted replay uses the stored error or fallback text; must contain "interrupted"
    let content = format!("{:?}", repair);
    assert!(
        content.contains("interrupted") || content.contains("not complete"),
        "expected interrupted message, got: {content}"
    );
}

#[tokio::test]
async fn test_repair_dangling_tool_calls_running_synthesized() {
    use crate::events::EventContext;
    use crate::typed_id::SessionId;
    use crate::{
        durability::DurableToolCallStatus, durability::DurableToolResultStore,
        durability::ToolCallClaimResult,
    };

    struct MockRunningStore;
    #[async_trait::async_trait]
    impl DurableToolResultStore for MockRunningStore {
        async fn try_claim_tool_call(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> crate::error::Result<ToolCallClaimResult> {
            Ok(ToolCallClaimResult::Claimed {
                claim_token: uuid::Uuid::new_v4(),
            })
        }
        async fn settle_tool_call(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
            _: &str,
            _: uuid::Uuid,
        ) -> crate::error::Result<bool> {
            Ok(true)
        }
        async fn get_tool_call_status(
            &self,
            _turn_id: &str,
            _tool_call_id: &str,
        ) -> crate::error::Result<Option<DurableToolCallStatus>> {
            Ok(Some(DurableToolCallStatus::Running))
        }
    }

    let tool_call = ToolCall {
        id: "call_run".to_string(),
        name: "long_job".to_string(),
        arguments: serde_json::json!({}),
    };
    let messages = vec![
        Message::user("Start job"),
        Message::assistant_with_tools("Starting...", vec![tool_call]),
    ];

    let store = MockRunningStore;
    let emitter = crate::test_fixtures::NoopEventEmitter;
    let session_id = SessionId::new();
    let ctx = EventContext::empty();
    let patched = repair_dangling_tool_calls(
        &messages,
        Some(&store as &dyn DurableToolResultStore),
        &emitter,
        session_id,
        &ctx,
        "turn_01",
    )
    .await;

    assert_eq!(patched.len(), 3);
    let repair = &patched[2];
    assert_eq!(repair.role, MessageRole::ToolResult);
    assert_eq!(repair.tool_call_id(), Some("call_run"));
    // Running stale claim must warn "uncertain; do not retry automatically"
    let content = format!("{:?}", repair);
    assert!(
        content.contains("uncertain") || content.contains("do not retry"),
        "expected uncertain/do-not-retry message, got: {content}"
    );
}

#[tokio::test]
async fn test_repair_dangling_tool_calls_store_error_unknown() {
    use crate::error::AgentLoopError;
    use crate::events::EventContext;
    use crate::typed_id::SessionId;
    use crate::{durability::DurableToolResultStore, durability::ToolCallClaimResult};

    struct MockErrorStore;
    #[async_trait::async_trait]
    impl DurableToolResultStore for MockErrorStore {
        async fn try_claim_tool_call(
            &self,
            _: &str,
            _: &str,
            _: &str,
            _: &str,
        ) -> crate::error::Result<ToolCallClaimResult> {
            Ok(ToolCallClaimResult::Claimed {
                claim_token: uuid::Uuid::new_v4(),
            })
        }
        async fn settle_tool_call(
            &self,
            _: &str,
            _: &str,
            _: serde_json::Value,
            _: &str,
            _: uuid::Uuid,
        ) -> crate::error::Result<bool> {
            Ok(true)
        }
        async fn get_tool_call_status(
            &self,
            _turn_id: &str,
            _tool_call_id: &str,
        ) -> crate::error::Result<Option<crate::durability::DurableToolCallStatus>> {
            Err(AgentLoopError::tool("simulated store failure"))
        }
    }

    let tool_call = ToolCall {
        id: "call_err".to_string(),
        name: "risky_op".to_string(),
        arguments: serde_json::json!({}),
    };
    let messages = vec![
        Message::user("Do risky op"),
        Message::assistant_with_tools("On it...", vec![tool_call]),
    ];

    let store = MockErrorStore;
    let emitter = crate::test_fixtures::NoopEventEmitter;
    let session_id = SessionId::new();
    let ctx = EventContext::empty();
    let patched = repair_dangling_tool_calls(
        &messages,
        Some(&store as &dyn DurableToolResultStore),
        &emitter,
        session_id,
        &ctx,
        "turn_01",
    )
    .await;

    assert_eq!(patched.len(), 3);
    let repair = &patched[2];
    assert_eq!(repair.role, MessageRole::ToolResult);
    assert_eq!(repair.tool_call_id(), Some("call_err"));
    // Store error must NOT say "safe to retry"
    let content = format!("{:?}", repair);
    assert!(
        !content.contains("safe to retry"),
        "store error must not say 'safe to retry', got: {content}"
    );
    assert!(
        content.contains("do not retry") || content.contains("status unknown"),
        "expected do-not-retry/status-unknown message, got: {content}"
    );
}

#[test]
fn test_build_request_options_for_openai_prompt_cache() {
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gpt-5.4".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: HashMap::new(),
        previous_response_id: Some("resp_123".to_string()),
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: Some(PromptCacheConfig {
            enabled: true,
            strategy: PromptCacheStrategy::Auto,
            gemini_cached_content: None,
        }),
        openrouter_routing: None,
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
    };

    let request_options = build_request_options(&config, "openai").unwrap();
    assert_eq!(
        request_options
            .prompt_cache
            .and_then(|info| info.provider_mode),
        Some("prompt_cache_key".to_string())
    );
    assert_eq!(
        request_options.provider_options.get("openai"),
        Some(&json!({ "previous_response_id": true }))
    );
}

#[test]
fn test_build_request_options_for_gemini_explicit_cache() {
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gemini-2.5-pro".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: Some(PromptCacheConfig {
            enabled: true,
            strategy: PromptCacheStrategy::Auto,
            gemini_cached_content: Some("cachedContents/demo-cache".to_string()),
        }),
        openrouter_routing: None,
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
    };

    let request_options = build_request_options(&config, "gemini").unwrap();
    assert_eq!(
        request_options
            .prompt_cache
            .and_then(|info| info.provider_mode),
        Some("cached_content".to_string())
    );
    assert_eq!(
        request_options.provider_options.get("gemini"),
        Some(&json!({ "cached_content": true }))
    );
}

#[test]
fn test_build_request_options_omits_gemini_cache_flag_when_disabled() {
    let config = LlmCallConfig {
        speed: None,
        verbosity: None,
        model: "gemini-2.5-pro".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        metadata: HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: Some(PromptCacheConfig {
            enabled: false,
            strategy: PromptCacheStrategy::Auto,
            gemini_cached_content: Some("cachedContents/demo-cache".to_string()),
        }),
        openrouter_routing: None,
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
    };

    assert!(build_request_options(&config, "gemini").is_none());
}

#[test]
fn system_keys_override_embedder_keys_in_metadata() {
    // Pins the injection order: embedder keys inserted first, then system
    // keys overwrite any collision. A harness author cannot shadow session_id,
    // turn_id, etc. by including them in embedder_metadata.
    let embedder_metadata: HashMap<String, String> = [
        ("session_id".to_string(), "attacker_value".to_string()),
        ("custom_key".to_string(), "custom_value".to_string()),
    ]
    .into();

    let mut metadata: HashMap<String, String> = HashMap::new();

    // Inject embedder metadata first (mirrors execute_single_turn logic)
    for (k, v) in &embedder_metadata {
        metadata.insert(k.clone(), v.clone());
    }

    // System key injected second — must overwrite
    metadata.insert("session_id".to_string(), "real_session_id".to_string());

    assert_eq!(
        metadata.get("session_id").map(String::as_str),
        Some("real_session_id"),
        "system key must overwrite embedder key with same name"
    );
    assert_eq!(
        metadata.get("custom_key").map(String::as_str),
        Some("custom_value"),
        "non-colliding embedder key must be preserved"
    );
}

// =========================================================================
// ContinuePartial recovery tests (EVE-532)
// =========================================================================

use crate::test_fixtures::NoopPartialStreamStore;
use crate::{durability::PartialStreamState, durability::PartialStreamStore};

struct MockPartialStore(Option<PartialStreamState>);

#[async_trait::async_trait]
impl PartialStreamStore for MockPartialStore {
    async fn get_partial_stream(
        &self,
        _session_id: crate::typed_id::SessionId,
        _turn_id: &str,
    ) -> crate::error::Result<Option<PartialStreamState>> {
        Ok(self.0.clone())
    }
}

#[tokio::test]
async fn test_noop_partial_stream_store_returns_none() {
    let store = NoopPartialStreamStore;
    let result = store
        .get_partial_stream(crate::typed_id::SessionId::new(), "turn_01")
        .await
        .unwrap();
    assert!(result.is_none());
}

#[tokio::test]
async fn test_partial_stream_store_returns_accumulated_when_partial_exists() {
    let message_id = MessageId::new();
    let store = MockPartialStore(Some(PartialStreamState {
        message_id,
        accumulated: "partial text so far".to_string(),
    }));
    let result = store
        .get_partial_stream(crate::typed_id::SessionId::new(), "turn_01")
        .await
        .unwrap();
    let partial = result.unwrap();
    assert_eq!(partial.message_id, message_id);
    assert_eq!(partial.accumulated, "partial text so far");
}

#[tokio::test]
async fn test_partial_stream_store_returns_empty_when_started_no_delta() {
    let store = MockPartialStore(Some(PartialStreamState {
        message_id: MessageId::new(),
        accumulated: String::new(),
    }));
    let result = store
        .get_partial_stream(crate::typed_id::SessionId::new(), "turn_01")
        .await
        .unwrap();
    assert!(result.unwrap().accumulated.is_empty());
}
