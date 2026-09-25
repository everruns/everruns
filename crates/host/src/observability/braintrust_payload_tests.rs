//! Event payload, serialization, and delivery tests.

use super::*;

// =============================================================================
// _is_merge Serialization Tests
// =============================================================================
//
// These tests verify that the _is_merge field is correctly serialized:
// - Started events: is_merge = None (creates new span)
// - Completed events: is_merge = Some(true) (merges with existing span)

#[test]
fn test_is_merge_serialization_started_events() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();

    // turn.started should have is_merge = None
    let turn_data = TurnStartedData {
        turn_id,
        input_message_id,
        input_content: Some("Test".to_string()),
        agent_id: None,
        agent_name: None,
        agent_description: None,
    };
    let event = Event::new(
        SessionId::new(),
        EventContext::empty(),
        EventData::TurnStarted(turn_data.clone()),
    );
    let bt_event = listener.convert_turn_started(&event, &turn_data);

    // is_merge should be None for started events
    assert!(
        bt_event.is_merge.is_none(),
        "turn.started should have is_merge = None"
    );

    // Serialize to JSON and verify _is_merge is not present
    let json = serde_json::to_string(&bt_event).unwrap();
    assert!(
        !json.contains("_is_merge"),
        "turn.started JSON should not contain _is_merge: {}",
        json
    );
}

#[test]
fn test_is_merge_serialization_completed_events() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();

    // turn.completed should have is_merge = Some(true)
    let turn_data = TurnCompletedData {
        turn_id,
        iterations: 1,
        duration_ms: Some(1000),
        usage: None,
        input_content: Some("Test".to_string()),
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
        EventData::TurnCompleted(turn_data.clone()),
    );
    let bt_event = listener.convert_turn_completed(&event, &turn_data);

    // is_merge should be Some(true) for completed events
    assert_eq!(
        bt_event.is_merge,
        Some(true),
        "turn.completed should have is_merge = Some(true)"
    );

    // Serialize to JSON and verify _is_merge is present and true
    let json = serde_json::to_string(&bt_event).unwrap();
    assert!(
        json.contains("\"_is_merge\":true"),
        "turn.completed JSON should contain _is_merge:true: {}",
        json
    );
}

#[test]
fn test_is_merge_serialization_reason_events() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let reason_span_id = Uuid::now_v7().to_string();

    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(reason_span_id.clone());
    context.parent_span_id = Some(turn_id.to_string());

    // reason.started - should NOT have _is_merge
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
    let started_json = serde_json::to_string(&bt_started).unwrap();
    assert!(
        !started_json.contains("_is_merge"),
        "reason.started should not have _is_merge: {}",
        started_json
    );

    // reason.completed - should have _is_merge: true
    let completed_data = ReasonCompletedData {
        success: true,
        text_preview: None,
        has_tool_calls: false,
        tool_call_count: 0,
        error: None,
        duration_ms: Some(500),
        usage: None,
    };
    let completed_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ReasonCompleted(completed_data.clone()),
    );
    let bt_completed = listener.convert_reason_completed(&completed_event, &completed_data);
    let completed_json = serde_json::to_string(&bt_completed).unwrap();
    assert!(
        completed_json.contains("\"_is_merge\":true"),
        "reason.completed should have _is_merge:true: {}",
        completed_json
    );
}

#[test]
fn test_is_merge_serialization_act_events() {
    use everruns_core::events::ToolCallSummary;

    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let act_span_id = Uuid::now_v7().to_string();

    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(act_span_id.clone());
    context.parent_span_id = Some(turn_id.to_string());

    // act.started - should NOT have _is_merge
    let started_data = ActStartedData {
        tool_calls: vec![ToolCallSummary {
            id: "call_1".to_string(),
            name: "search".to_string(),
            display_name: None,
            narration: None,
            completed_narration: None,
        }],
        headline: None,
    };
    let started_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ActStarted(started_data.clone()),
    );
    let bt_started = listener.convert_act_started(&started_event, &started_data);
    let started_json = serde_json::to_string(&bt_started).unwrap();
    assert!(
        !started_json.contains("_is_merge"),
        "act.started should not have _is_merge: {}",
        started_json
    );

    // act.completed - should have _is_merge: true
    let completed_data = ActCompletedData {
        completed: true,
        success_count: 1,
        error_count: 0,
        duration_ms: Some(200),
        headline: None,
    };
    let completed_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ActCompleted(completed_data.clone()),
    );
    let bt_completed = listener.convert_act_completed(&completed_event, &completed_data);
    let completed_json = serde_json::to_string(&bt_completed).unwrap();
    assert!(
        completed_json.contains("\"_is_merge\":true"),
        "act.completed should have _is_merge:true: {}",
        completed_json
    );
}

#[test]
fn test_is_merge_serialization_tool_events() {
    use everruns_provider::tool_types::ToolCall;

    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let tool_span_id = Uuid::now_v7().to_string();
    let act_span_id = Uuid::now_v7().to_string();

    let mut context = EventContext::empty();
    context.turn_id = Some(turn_id);
    context.trace_id = Some(turn_id.to_string());
    context.span_id = Some(tool_span_id.clone());
    context.parent_span_id = Some(act_span_id.clone());

    // tool.started - should NOT have _is_merge
    let started_data = ToolStartedData {
        tool_call: ToolCall {
            id: "call_1".to_string(),
            name: "search".to_string(),
            arguments: serde_json::json!({"query": "test"}),
        },
        tool_call_fingerprint: None,
        display_name: None,
        narration: None,
    };
    let started_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ToolStarted(started_data.clone()),
    );
    let bt_started = listener.convert_tool_call_started(&started_event, &started_data);
    let started_json = serde_json::to_string(&bt_started).unwrap();
    assert!(
        !started_json.contains("_is_merge"),
        "tool.started should not have _is_merge: {}",
        started_json
    );

    // tool.completed - should have _is_merge: true
    let completed_data = ToolCompletedData {
        tool_call_id: "call_1".to_string(),
        tool_name: "search".to_string(),
        tool_call_fingerprint: None,
        tool_result_fingerprint: None,
        display_name: None,
        success: true,
        status: "success".to_string(),
        result: None,
        error: None,
        duration_ms: Some(100),
        capability_id: None,
        capability_name: None,
        narration: None,
    };
    let completed_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ToolCompleted(completed_data.clone()),
    );
    let bt_completed = listener.convert_tool_call_completed(&completed_event, &completed_data);
    let completed_json = serde_json::to_string(&bt_completed).unwrap();
    assert!(
        completed_json.contains("\"_is_merge\":true"),
        "tool.completed should have _is_merge:true: {}",
        completed_json
    );
}

#[test]
fn test_convert_reason_thinking_started() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let span_id = Uuid::new_v4().to_string();

    let data = ReasonThinkingStartedData {
        turn_id,
        model: Some("claude-sonnet-4-20250514".to_string()),
    };

    // Context with parent span
    let context = EventContext {
        turn_id: Some(turn_id),
        input_message_id: None,
        span_id: Some(span_id.clone()),
        parent_span_id: Some(turn_id.to_string()),
        exec_id: None,
        trace_id: Some(turn_id.to_string()),
    };

    let event = Event::new(
        SessionId::new(),
        context,
        EventData::ReasonThinkingStarted(data.clone()),
    );

    let bt_event = listener.convert_reason_thinking_started(&event, &data);

    assert_eq!(bt_event.span_attributes.name, "thinking");
    assert_eq!(bt_event.span_attributes.span_type, "task");
    assert_eq!(bt_event.span_id, Some(span_id.clone()));
    assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
    assert!(bt_event.is_merge.is_none()); // First event creates the span
    assert!(bt_event.metrics.as_ref().unwrap().start.is_some());
    assert!(bt_event.metrics.as_ref().unwrap().end.is_none());

    // Verify metadata
    let metadata = bt_event.metadata;
    assert_eq!(metadata["model"], "claude-sonnet-4-20250514");
    assert_eq!(metadata["turn_id"], turn_id.to_string());
}

#[test]
fn test_convert_reason_thinking_completed() {
    let mut config = test_config();
    config.content.record_thinking = BraintrustThinkingMode::Full;
    let listener = BraintrustListener::new(config).unwrap();
    let turn_id = TurnId::new();
    let span_id = Uuid::new_v4().to_string();

    let thinking_content = "Let me think about this problem step by step...".to_string();
    let data = ReasonThinkingCompletedData {
        turn_id,
        thinking: thinking_content.clone(),
    };

    // Context with parent span
    let context = EventContext {
        turn_id: Some(turn_id),
        input_message_id: None,
        span_id: Some(span_id.clone()),
        parent_span_id: Some(turn_id.to_string()),
        exec_id: None,
        trace_id: Some(turn_id.to_string()),
    };

    let event = Event::new(
        SessionId::new(),
        context,
        EventData::ReasonThinkingCompleted(data.clone()),
    );

    let bt_event = listener.convert_reason_thinking_completed(&event, &data);

    assert_eq!(bt_event.span_attributes.name, "thinking");
    assert_eq!(bt_event.span_attributes.span_type, "task");
    assert_eq!(bt_event.span_id, Some(span_id.clone()));
    assert_eq!(bt_event.root_span_id, Some(turn_id.to_string()));
    assert_eq!(bt_event.is_merge, Some(true)); // Merges with started event

    // Verify output contains thinking content
    let output = bt_event.output.unwrap();
    assert_eq!(output["thinking"], thinking_content);

    // Verify metadata
    let metadata = bt_event.metadata;
    assert_eq!(metadata["thinking_length"], thinking_content.len());
    assert_eq!(metadata["turn_id"], turn_id.to_string());

    // Verify metrics have end time
    assert!(bt_event.metrics.as_ref().unwrap().end.is_some());
}

#[test]
fn test_turn_started_metadata_includes_session_grouping_fields() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    listener.update_session_state(&session_id.to_string(), |session_state| {
        session_state.harness_id = Some(HarnessId::new().to_string());
        session_state.agent_id = Some(AgentId::new().to_string());
        session_state.last_status = Some("active".to_string());
    });

    let event = Event::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        EventData::TurnStarted(TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("hello braintrust".to_string()),
            agent_id: None,
            agent_name: None,
            agent_description: None,
        }),
    )
    .with_sequence(42);

    let data = match &event.data {
        EventData::TurnStarted(data) => data.clone(),
        _ => unreachable!(),
    };
    listener.record_turn_started_state(&event, &data);
    let bt_event = listener.convert_turn_started(&event, &data);

    assert_eq!(bt_event.metadata["session_id"], session_id.to_string());
    assert_eq!(bt_event.metadata["turn_id"], turn_id.to_string());
    assert_eq!(
        bt_event.metadata["input_message_id"],
        input_message_id.to_string()
    );
    assert_eq!(bt_event.metadata["session_event_sequence"], 42);
    assert_eq!(bt_event.metadata["turn_started_sequence"], 42);
    assert_eq!(bt_event.metadata["deployment_grade"], "dev");
    assert_eq!(bt_event.metadata["session_status"], "active");
    assert!(bt_event.metadata.get("harness_id").is_some());
    assert!(bt_event.metadata.get("agent_id").is_some());
}

#[test]
fn test_tool_started_redacts_arguments_when_configured() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let event = Event::new(
        SessionId::new(),
        EventContext::turn(turn_id, MessageId::new()),
        EventData::ToolStarted(ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "exec".to_string(),
                arguments: json!({"secret": "value", "path": "/tmp/test"}),
            },
            tool_call_fingerprint: None,
            display_name: Some("Execute".to_string()),
            narration: Some("Running exec".to_string()),
        }),
    );

    let data = match &event.data {
        EventData::ToolStarted(data) => data.clone(),
        _ => unreachable!(),
    };
    let bt_event = listener.convert_tool_call_started(&event, &data);
    assert_eq!(
        bt_event.input.as_ref().unwrap()["arguments"]["redacted"],
        true
    );
    assert!(
        bt_event.input.as_ref().unwrap()["arguments"]["summary"]["keys"]
            .as_array()
            .unwrap()
            .iter()
            .any(|value| value == "secret")
    );
    assert!(bt_event.metadata.get("display_name").is_none());
    assert!(bt_event.metadata.get("narration").is_none());
}

#[test]
fn test_tool_started_includes_labels_when_tool_args_full() {
    let mut config = test_config();
    config.content.tool_args_mode = BraintrustPayloadMode::Full;
    let listener = BraintrustListener::new(config).unwrap();
    let turn_id = TurnId::new();
    let event = Event::new(
        SessionId::new(),
        EventContext::turn(turn_id, MessageId::new()),
        EventData::ToolStarted(ToolStartedData {
            tool_call: ToolCall {
                id: "call_1".to_string(),
                name: "exec".to_string(),
                arguments: json!({"secret": "value", "path": "/tmp/test"}),
            },
            tool_call_fingerprint: None,
            display_name: Some("Execute".to_string()),
            narration: Some("Running exec".to_string()),
        }),
    );

    let data = match &event.data {
        EventData::ToolStarted(data) => data.clone(),
        _ => unreachable!(),
    };
    let bt_event = listener.convert_tool_call_started(&event, &data);
    assert_eq!(bt_event.metadata["display_name"], "Execute");
    assert_eq!(bt_event.metadata["narration"], "Running exec");
}

#[test]
fn test_parse_env_bool_accepts_common_boolean_values() {
    for value in ["true", "TRUE", "1", "yes", "on"] {
        assert_eq!(parse_env_bool(value), Some(true));
    }
    for value in ["false", "FALSE", "0", "no", "off"] {
        assert_eq!(parse_env_bool(value), Some(false));
    }
    assert_eq!(parse_env_bool("maybe"), None);
}

#[test]
fn test_turn_started_without_content_recording_omits_input_preview() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    let data = TurnStartedData {
        turn_id,
        input_message_id,
        input_content: Some("secret prompt".to_string()),
        agent_id: None,
        agent_name: None,
        agent_description: None,
    };
    let event = Event::new(
        SessionId::new(),
        EventContext::turn(turn_id, input_message_id),
        EventData::TurnStarted(data.clone()),
    );

    let bt_event = listener.convert_turn_started(&event, &data);
    let input = bt_event.input.unwrap();
    assert_eq!(input["content_recorded"], false);
    assert_eq!(input["has_input_content"], true);
    assert!(!input.as_object().unwrap().contains_key("input_preview"));
}

#[test]
fn test_llm_output_without_content_recording_omits_text_preview() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let data = LlmGenerationData {
        messages: vec![RuntimeMessage::user("Hello")],
        tools: vec![],
        output: LlmGenerationOutput {
            text: Some("Sensitive completion".to_string()),
            tool_calls: vec![ToolCall {
                id: "call_1".to_string(),
                name: "search".to_string(),
                arguments: json!({"query": "rust"}),
            }],
        },
        metadata: bare_generation_metadata("gpt-4"),
    };

    let output = listener.llm_output_payload(&data).unwrap();
    assert_eq!(output["text_recorded"], false);
    assert_eq!(output["text_present"], true);
    assert!(!output.as_object().unwrap().contains_key("text_preview"));
}

#[test]
fn test_llm_content_recording_respects_tool_payload_modes() {
    let mut config = test_config();
    config.content.record_content = true;
    let listener = BraintrustListener::new(config).unwrap();
    let data = LlmGenerationData {
        messages: vec![
            RuntimeMessage::assistant_with_tools(
                "Calling tool",
                vec![ToolCall {
                    id: "call_1".to_string(),
                    name: "search".to_string(),
                    arguments: json!({"secret": "value", "query": "rust"}),
                }],
            ),
            RuntimeMessage::tool_result(
                "call_1",
                Some(json!({"secret_result": "top secret", "count": 3})),
                None,
            ),
        ],
        tools: vec![],
        output: LlmGenerationOutput {
            text: Some("Done".to_string()),
            tool_calls: vec![ToolCall {
                id: "call_2".to_string(),
                name: "write_file".to_string(),
                arguments: json!({"token": "shh", "path": "/tmp/out"}),
            }],
        },
        metadata: bare_generation_metadata("gpt-4"),
    };

    let input = listener.llm_input_payload(&data).unwrap();
    let messages = input.as_array().unwrap();
    let assistant_args = messages[0]["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .unwrap();
    assert!(assistant_args.contains("\"redacted\":true"));
    assert!(!assistant_args.contains("value"));

    let tool_result_content = messages[1]["content"].as_str().unwrap();
    assert!(tool_result_content.contains("\"redacted\":true"));
    assert!(!tool_result_content.contains("top secret"));

    let output = listener.llm_output_payload(&data).unwrap();
    let output_args = output["tool_calls"][0]["function"]["arguments"]
        .as_str()
        .unwrap();
    assert!(output_args.contains("\"redacted\":true"));
    assert!(!output_args.contains("shh"));
}

#[test]
fn test_tool_completed_summary_omits_text_preview() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let data = ToolCompletedData {
        tool_call_id: "call_123".to_string(),
        tool_name: "search".to_string(),
        tool_call_fingerprint: None,
        tool_result_fingerprint: None,
        display_name: None,
        success: true,
        status: "success".to_string(),
        result: Some(vec![everruns_core::ContentPart::text(
            "sensitive tool output",
        )]),
        error: None,
        duration_ms: Some(50),
        capability_id: None,
        capability_name: None,
        narration: None,
    };
    let event = Event::new(
        SessionId::new(),
        EventContext::turn(turn_id, MessageId::new()),
        EventData::ToolCompleted(data.clone()),
    );

    let bt_event = listener.convert_tool_call_completed(&event, &data);
    let result = bt_event.output.unwrap()["result"].clone();
    assert_eq!(result["redacted"], true);
    assert_eq!(result["summary"]["text_part_count"], 1);
    assert!(!result.to_string().contains("sensitive tool output"));
    assert!(
        !result["summary"]
            .as_object()
            .unwrap()
            .contains_key("text_preview")
    );
}

#[tokio::test]
async fn test_session_idled_prunes_session_state() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();

    listener
        .on_event(&Event::new(
            session_id,
            EventContext::empty(),
            EventData::SessionStarted(everruns_core::events::SessionStartedData {
                harness_id: HarnessId::from_seed(1),
                agent_id: None,
                model_id: None,
            }),
        ))
        .await;
    assert!(
        listener
            .current_session_state(&session_id.to_string())
            .is_some()
    );

    listener
        .on_event(&Event::new(
            session_id,
            EventContext::turn(turn_id, input_message_id),
            EventData::SessionIdled(everruns_core::events::SessionIdledData {
                turn_id,
                iterations: Some(1),
                usage: None,
            }),
        ))
        .await;

    assert!(
        listener
            .current_session_state(&session_id.to_string())
            .is_none()
    );
}

#[tokio::test]
async fn test_on_event_batches_multiple_events_into_one_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = test_config();
    config.api_url = server.uri();
    config.delivery.flush_interval = Duration::from_millis(20);
    config.delivery.max_batch_size = 10;
    let listener = BraintrustListener::new(config).unwrap();

    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    let start = Event::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        EventData::TurnStarted(TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("hello".to_string()),
            agent_id: None,
            agent_name: None,
            agent_description: None,
        }),
    );
    let completed = Event::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        EventData::TurnCompleted(TurnCompletedData {
            turn_id,
            iterations: 1,
            duration_ms: Some(5),
            usage: None,
            input_content: Some("hello".to_string()),
            final_message_id: None,
            final_answer_preview: None,
            time_to_first_token_ms: None,
            tool_call_count: None,
            llm_call_count: None,
            status: None,
        }),
    );

    listener.on_event(&start).await;
    listener.on_event(&completed).await;
    sleep(Duration::from_millis(80)).await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["events"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn test_permanent_rejection_is_reported_once_per_process() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
        .expect(2)
        .mount(&server)
        .await;

    let mut config = test_config();
    config.api_url = server.uri();
    config.delivery.flush_interval = Duration::from_millis(10);
    let listener = BraintrustListener::new(config).unwrap();
    assert!(
        !listener
            .state
            .permanent_failure_logged
            .load(Ordering::Relaxed)
    );

    for _ in 0..2 {
        let turn_id = TurnId::new();
        let input_message_id = MessageId::new();
        listener
            .on_event(&Event::new(
                SessionId::new(),
                EventContext::turn(turn_id, input_message_id),
                EventData::TurnStarted(TurnStartedData {
                    turn_id,
                    input_message_id,
                    input_content: Some("hello".to_string()),
                    agent_id: None,
                    agent_name: None,
                    agent_description: None,
                }),
            ))
            .await;
        sleep(Duration::from_millis(60)).await;
    }

    assert_eq!(listener.state.failed_batches.load(Ordering::Relaxed), 2);
    assert_eq!(listener.state.retried_batches.load(Ordering::Relaxed), 0);
    assert!(
        listener
            .state
            .permanent_failure_logged
            .load(Ordering::Relaxed)
    );
}

#[tokio::test]
async fn test_on_event_retries_after_429() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = test_config();
    config.api_url = server.uri();
    config.delivery.flush_interval = Duration::from_millis(10);
    config.delivery.base_retry_delay = Duration::from_millis(5);
    config.delivery.max_retry_delay = Duration::from_millis(10);
    let listener = BraintrustListener::new(config).unwrap();

    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    let event = Event::new(
        SessionId::new(),
        EventContext::turn(turn_id, input_message_id),
        EventData::TurnStarted(TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("hello".to_string()),
            agent_id: None,
            agent_name: None,
            agent_description: None,
        }),
    );
    listener.on_event(&event).await;
    sleep(Duration::from_millis(120)).await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(listener.state.retried_batches.load(Ordering::Relaxed) >= 1);
    assert_eq!(listener.state.failed_batches.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn test_on_event_retries_after_timeout() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(50)))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/project_logs/test-project-id/insert"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = test_config();
    config.api_url = server.uri();
    config.delivery.flush_interval = Duration::from_millis(10);
    config.delivery.request_timeout = Duration::from_millis(10);
    config.delivery.base_retry_delay = Duration::from_millis(5);
    config.delivery.max_retry_delay = Duration::from_millis(10);
    let listener = BraintrustListener::new(config).unwrap();

    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    let event = Event::new(
        session_id,
        EventContext::turn(turn_id, input_message_id),
        EventData::TurnStarted(TurnStartedData {
            turn_id,
            input_message_id,
            input_content: Some("hello".to_string()),
            agent_id: None,
            agent_name: None,
            agent_description: None,
        }),
    );
    listener.on_event(&event).await;
    sleep(Duration::from_millis(180)).await;

    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(listener.state.retried_batches.load(Ordering::Relaxed) >= 1);
    assert_eq!(listener.state.failed_batches.load(Ordering::Relaxed), 0);
}

#[test]
fn test_thinking_spans_merge_correctly() {
    let listener = BraintrustListener::new(test_config()).unwrap();
    let turn_id = TurnId::new();
    let span_id = Uuid::new_v4().to_string();

    let context = EventContext {
        turn_id: Some(turn_id),
        input_message_id: None,
        span_id: Some(span_id.clone()),
        parent_span_id: Some(turn_id.to_string()),
        exec_id: None,
        trace_id: Some(turn_id.to_string()),
    };

    // thinking.started
    let started_data = ReasonThinkingStartedData {
        turn_id,
        model: Some("claude-sonnet-4-20250514".to_string()),
    };
    let started_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ReasonThinkingStarted(started_data.clone()),
    );
    let bt_started = listener.convert_reason_thinking_started(&started_event, &started_data);

    // thinking.completed
    let completed_data = ReasonThinkingCompletedData {
        turn_id,
        thinking: "Complete thinking content".to_string(),
    };
    let completed_event = Event::new(
        SessionId::new(),
        context.clone(),
        EventData::ReasonThinkingCompleted(completed_data.clone()),
    );
    let bt_completed =
        listener.convert_reason_thinking_completed(&completed_event, &completed_data);

    // Both should use the same span_id for merging
    assert_eq!(bt_started.id, bt_completed.id);
    assert_eq!(bt_started.span_id, bt_completed.span_id);

    // started should not have _is_merge, completed should have _is_merge: true
    let started_json = serde_json::to_string(&bt_started).unwrap();
    assert!(
        !started_json.contains("\"_is_merge\""),
        "thinking.started should not have _is_merge: {}",
        started_json
    );

    let completed_json = serde_json::to_string(&bt_completed).unwrap();
    assert!(
        completed_json.contains("\"_is_merge\":true"),
        "thinking.completed should have _is_merge:true: {}",
        completed_json
    );
}
