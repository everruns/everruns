//! Event conversion and hierarchy tests.

use super::*;

#[test]
fn test_listener_creation() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    assert_eq!(listener.name(), "BraintrustListener");
}

/// Telemetry init must never panic the host process. `new` returns a
/// `Result` so a misconfigured TLS/proxy environment disables Braintrust
/// instead of aborting the process. With no env config, `from_env` yields
/// `None` (disabled) rather than panicking.
#[test]
fn test_init_never_panics_and_disables_on_missing_config() {
    // Valid config builds successfully (Ok, not a panic).
    assert!(BraintrustListener::new(test_config()).is_ok());

    // `from_env` must not panic; with the API key absent it returns None.
    // Guard against an ambient key in the environment so the test is
    // deterministic in any CI/runtime.
    let key_present = std::env::var("BRAINTRUST_API_KEY").is_ok();
    let listener = BraintrustListener::from_env();
    if !key_present {
        assert!(
            listener.is_none(),
            "from_env should disable Braintrust when no API key is configured"
        );
    }
}

#[test]
fn test_event_types() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let types = listener.event_types().unwrap();
    // 16 event types: 4 turn lifecycle + 4 atom lifecycle + 2 thinking + 1 llm + 2 tool + 3 session lifecycle
    assert_eq!(types.len(), 16);
    // Turn lifecycle
    assert!(types.contains(&TURN_STARTED));
    assert!(types.contains(&TURN_COMPLETED));
    assert!(types.contains(&TURN_FAILED));
    assert!(types.contains(&TURN_CANCELLED));
    // Atom lifecycle
    assert!(types.contains(&REASON_STARTED));
    assert!(types.contains(&REASON_COMPLETED));
    assert!(types.contains(&ACT_STARTED));
    assert!(types.contains(&ACT_COMPLETED));
    // Extended thinking
    assert!(types.contains(&REASON_THINKING_STARTED));
    assert!(types.contains(&REASON_THINKING_COMPLETED));
    // LLM
    assert!(types.contains(&LLM_GENERATION));
    // Tool
    assert!(types.contains(&TOOL_STARTED));
    assert!(types.contains(&TOOL_COMPLETED));
    // Session lifecycle
    assert!(types.contains(&SESSION_STARTED));
    assert!(types.contains(&SESSION_ACTIVATED));
    assert!(types.contains(&SESSION_IDLED));
}

#[test]
fn test_convert_turn_started() {
    let listener = BraintrustListener::new(test_config()).unwrap();

    let turn_id = TurnId::new();
    let message_id = MessageId::new();

    let data = TurnStartedData {
        turn_id,
        input_message_id: message_id,
        input_content: Some("Hello, how are you?".to_string()),
        agent_id: None,
        agent_name: None,
        agent_description: None,
    };

    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::TurnStarted(data.clone()),
    );

    let bt_event = listener.convert_turn_started(&event, &data);

    assert_eq!(bt_event.id, turn_id.to_string());
    assert_eq!(bt_event.span_attributes.span_type, "task");
    assert_eq!(bt_event.span_attributes.name, "agent turn");
    // Root span self-references for proper trace correlation
    assert_eq!(bt_event.span_id, Some(turn_id.to_string()));
    assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
    assert!(bt_event.span_parents.is_none()); // Root has no parents
}

#[test]
fn test_convert_llm_generation_with_parent() {
    let listener = BraintrustListener::new(test_config()).unwrap();

    let turn_id = TurnId::new();

    let data = LlmGenerationData {
        messages: vec![
            RuntimeMessage::user("Hello"),
            RuntimeMessage::assistant("Hi there!"),
        ],
        tools: vec![],
        output: LlmGenerationOutput {
            text: Some("Hi there!".to_string()),
            tool_calls: vec![],
        },
        metadata: LlmGenerationMetadata {
            model: "gpt-4".to_string(),
            provider: Some("openai".to_string()),
            response_model: None,
            usage: Some(TokenUsage {
                input_tokens: 10,
                output_tokens: 5,
                cache_read_tokens: None,
                cache_creation_tokens: None,
                actual_cost_usd: None,
                estimated_cost_usd: None,
                effective_cost_usd: None,
            }),
            duration_ms: Some(100),
            time_to_first_token_ms: Some(25),
            success: true,
            error: None,
            finish_reasons: Some(vec!["stop".to_string()]),
            response_id: Some("resp_123".to_string()),
            retry: None,
            compaction: None,
            request_options: None,
        },
    };

    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);

    let event = Event::new(
        SessionId::new(),
        context,
        EventData::LlmGeneration(data.clone()),
    );

    let bt_event = listener.convert_llm_generation(&event, &data);

    assert_eq!(bt_event.span_attributes.name, "chat gpt-4");
    assert_eq!(bt_event.span_attributes.span_type, "llm");
    assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
    assert_eq!(bt_event.span_parents, Some(vec![turn_id.to_string()]));
}

#[test]
fn test_convert_turn_completed_with_usage() {
    let listener = BraintrustListener::new(test_config()).unwrap();

    let turn_id = TurnId::new();

    let data = TurnCompletedData {
        turn_id,
        iterations: 3,
        duration_ms: Some(5000),
        usage: Some(TokenUsage {
            input_tokens: 100,
            output_tokens: 50,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            actual_cost_usd: None,
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }),
        input_content: Some("Hello, how are you?".to_string()),
        final_message_id: None,
        final_answer_preview: None,
        time_to_first_token_ms: None,
        tool_call_count: None,
        llm_call_count: None,
        status: None,
    };

    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::TurnCompleted(data.clone()),
    );

    let bt_event = listener.convert_turn_completed(&event, &data);

    assert_eq!(bt_event.id, turn_id.to_string());
    assert!(bt_event.metrics.is_some());
    let metrics = bt_event.metrics.unwrap();
    assert_eq!(metrics.prompt_tokens, Some(100));
    assert_eq!(metrics.completion_tokens, Some(50));
    assert_eq!(metrics.tokens, Some(150));
}

// =============================================================================
// Event Hierarchy Tests
// =============================================================================
//
// These tests verify the span ID relationships required for proper trace hierarchy:
//
// agent turn (root)
// ├── reason (parent: turn)
// │   └── llm.generation (parent: reason)
// ├── act (parent: turn)
// │   └── tool.call (parent: act)
// └── ...

#[test]
fn test_turn_events_are_self_referencing_root_spans() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();

    // Test turn.started
    let started_data = TurnStartedData {
        turn_id,
        input_message_id,
        input_content: Some("Test input content".to_string()),
        agent_id: None,
        agent_name: None,
        agent_description: None,
    };
    let started_event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::TurnStarted(started_data.clone()),
    );
    let bt_started = listener.convert_turn_started(&started_event, &started_data);

    // Root span: span_id = root_span_id = turn_id, no parents
    assert_eq!(
        bt_started.id,
        turn_id.to_string(),
        "turn.started id should be turn_id"
    );
    assert_eq!(
        bt_started.span_id,
        Some(turn_id.to_string()),
        "turn.started span_id should be turn_id"
    );
    assert_eq!(
        bt_started.root_span_id,
        Some(turn_id.to_string()),
        "turn.started root_span_id should be turn_id"
    );
    assert!(
        bt_started.span_parents.is_none(),
        "turn.started should have no parents (root span)"
    );

    // Test turn.completed uses same IDs (for merging)
    let completed_data = TurnCompletedData {
        turn_id,
        iterations: 1,
        duration_ms: Some(1000),
        usage: None,
        input_content: Some("Test input content".to_string()),
        final_message_id: None,
        final_answer_preview: None,
        time_to_first_token_ms: None,
        tool_call_count: None,
        llm_call_count: None,
        status: None,
    };
    let completed_event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::TurnCompleted(completed_data.clone()),
    );
    let bt_completed = listener.convert_turn_completed(&completed_event, &completed_data);

    // Same IDs as started for proper merging
    assert_eq!(
        bt_completed.id,
        turn_id.to_string(),
        "turn.completed id should match turn.started"
    );
    assert_eq!(
        bt_completed.span_id,
        Some(turn_id.to_string()),
        "turn.completed span_id should match turn.started"
    );
    assert_eq!(bt_completed.root_span_id, Some(turn_id.to_string()));
}

#[test]
fn test_reason_events_have_turn_as_parent() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let reason_span_id = Uuid::now_v7().to_string();

    // Create event context with proper span linkage
    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(reason_span_id.clone());
    context.parent_span_id = Some(turn_id.to_string());

    let data = ReasonStartedData {
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        metadata: None,
    };
    let event = Event::new(
        SessionId::new(),
        context,
        EventData::ReasonStarted(data.clone()),
    );

    let bt_event = listener.convert_reason_started(&event, &data);

    // Reason should be child of turn
    assert_eq!(
        bt_event.span_id,
        Some(reason_span_id.clone()),
        "reason span_id should be the reason's span"
    );
    assert_eq!(
        bt_event.root_span_id,
        Some(turn_id.to_string()),
        "reason root_span_id should be turn_id"
    );
    assert_eq!(
        bt_event.span_parents,
        Some(vec![turn_id.to_string()]),
        "reason parent should be turn"
    );
}

#[test]
fn test_llm_generation_with_span_context_has_reason_as_parent() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let reason_span_id = Uuid::now_v7().to_string();
    let llm_span_id = Uuid::now_v7().to_string();

    // LLM generation event with span context (parent is reason)
    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(llm_span_id.clone());
    context.parent_span_id = Some(reason_span_id.clone());

    let data = LlmGenerationData {
        messages: vec![RuntimeMessage::user("Hello")],
        tools: vec![],
        output: LlmGenerationOutput {
            text: Some("Hi!".to_string()),
            tool_calls: vec![],
        },
        metadata: bare_generation_metadata("gpt-4"),
    };
    let event = Event::new(
        SessionId::new(),
        context,
        EventData::LlmGeneration(data.clone()),
    );

    let bt_event = listener.convert_llm_generation(&event, &data);

    // LLM should be child of reason
    assert_eq!(
        bt_event.span_id,
        Some(llm_span_id),
        "llm span_id should be its own span"
    );
    assert_eq!(
        bt_event.root_span_id,
        Some(turn_id.to_string()),
        "llm root_span_id should be turn_id"
    );
    assert_eq!(
        bt_event.span_parents,
        Some(vec![reason_span_id]),
        "llm parent should be reason span"
    );
}

#[test]
fn test_act_events_have_turn_as_parent() {
    use everruns_core::events::ToolCallSummary;

    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let act_span_id = Uuid::now_v7().to_string();

    // Create event context with proper span linkage
    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(act_span_id.clone());
    context.parent_span_id = Some(turn_id.to_string());

    let data = ActStartedData {
        tool_calls: vec![
            ToolCallSummary {
                id: "call_1".to_string(),
                name: "search".to_string(),
                display_name: None,
                narration: None,
                completed_narration: None,
            },
            ToolCallSummary {
                id: "call_2".to_string(),
                name: "fetch".to_string(),
                display_name: None,
                narration: None,
                completed_narration: None,
            },
        ],
        headline: None,
    };
    let event = Event::new(
        SessionId::new(),
        context,
        EventData::ActStarted(data.clone()),
    );

    let bt_event = listener.convert_act_started(&event, &data);

    // Act should be child of turn
    assert_eq!(
        bt_event.span_id,
        Some(act_span_id.clone()),
        "act span_id should be the act's span"
    );
    assert_eq!(
        bt_event.root_span_id,
        Some(turn_id.to_string()),
        "act root_span_id should be turn_id"
    );
    assert_eq!(
        bt_event.span_parents,
        Some(vec![turn_id.to_string()]),
        "act parent should be turn"
    );
}

#[test]
fn test_tool_call_events_have_act_as_parent() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let act_span_id = Uuid::now_v7().to_string();
    let tool_span_id = Uuid::now_v7().to_string();

    // Tool call event with span context (parent is act)
    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(tool_span_id.clone());
    context.parent_span_id = Some(act_span_id.clone());

    let data = ToolCompletedData {
        tool_call_id: "call_123".to_string(),
        tool_name: "search".to_string(),
        tool_call_fingerprint: None,
        tool_result_fingerprint: None,
        display_name: None,
        success: true,
        status: "success".to_string(),
        result: None,
        error: None,
        duration_ms: Some(200),
        capability_id: None,
        capability_name: None,
        narration: None,
    };
    let event = Event::new(
        SessionId::new(),
        context,
        EventData::ToolCompleted(data.clone()),
    );

    let bt_event = listener.convert_tool_call_completed(&event, &data);

    // Tool should be child of act
    assert_eq!(
        bt_event.span_id,
        Some(tool_span_id),
        "tool span_id should be its own span"
    );
    assert_eq!(
        bt_event.root_span_id,
        Some(turn_id.to_string()),
        "tool root_span_id should be turn_id"
    );
    assert_eq!(
        bt_event.span_parents,
        Some(vec![act_span_id]),
        "tool parent should be act span"
    );
}

#[test]
fn test_started_completed_pairs_share_span_id() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let shared_span_id = Uuid::now_v7().to_string();

    // Both started and completed should use the same span_id
    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(shared_span_id.clone());
    context.parent_span_id = Some(turn_id.to_string());

    // reason.started
    let started_data = ReasonStartedData {
        harness_id: HarnessId::from_seed(1),
        agent_id: Some(AgentId::new()),
        metadata: None,
    };
    let started_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ReasonStarted(started_data.clone()),
    );
    let bt_started = listener.convert_reason_started(&started_event, &started_data);

    // reason.completed with same span context
    let completed_data = ReasonCompletedData {
        success: true,
        text_preview: Some("Hello".to_string()),
        has_tool_calls: false,
        tool_call_count: 0,
        error: None,
        duration_ms: Some(1500),
        usage: None,
    };
    let completed_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ReasonCompleted(completed_data.clone()),
    );
    let bt_completed = listener.convert_reason_completed(&completed_event, &completed_data);

    // Both should have same span_id for Braintrust to merge them
    assert_eq!(
        bt_started.span_id, bt_completed.span_id,
        "started and completed should share span_id"
    );
    assert_eq!(
        bt_started.id, bt_completed.id,
        "started and completed should share log id"
    );
}
