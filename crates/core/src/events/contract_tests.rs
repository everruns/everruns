//! Tests moved out of events.rs: contract_tests.

use crate::typed_id::{AgentId, EventId, HarnessId, MessageId, ModelId, SessionId, TurnId};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::*;
use insta::{assert_json_snapshot, with_settings};
use serde_json::json;

/// Helper to create deterministic test IDs for snapshot stability
fn test_session_id() -> SessionId {
    SessionId::from_uuid(uuid::Uuid::from_u128(
        0x0000_0000_0000_0000_0000_0000_0000_0001,
    ))
}

fn test_turn_id() -> TurnId {
    TurnId::from_uuid(uuid::Uuid::from_u128(
        0x0000_0000_0000_0000_0000_0000_0000_0002,
    ))
}

fn test_message_id() -> MessageId {
    MessageId::from_uuid(uuid::Uuid::from_u128(
        0x0000_0000_0000_0000_0000_0000_0000_0003,
    ))
}

fn test_agent_id() -> AgentId {
    AgentId::from_uuid(uuid::Uuid::from_u128(
        0x0000_0000_0000_0000_0000_0000_0000_0004,
    ))
}

fn test_model_id() -> ModelId {
    ModelId::from_uuid(uuid::Uuid::from_u128(
        0x0000_0000_0000_0000_0000_0000_0000_0006,
    ))
}

fn test_harness_id() -> HarnessId {
    HarnessId::from_uuid(uuid::Uuid::from_u128(
        0x0000_0000_0000_0000_0000_0000_0000_0005,
    ))
}

// ========================================================================
// Serialization Snapshot Tests
// ========================================================================
// These tests capture the canonical JSON representation of each event type.
// Changes to these snapshots indicate a potential breaking change.

#[test]
fn snapshot_input_message() {
    let data = InputMessageData::new(RuntimeMessage::user("Hello, world!"));
    with_settings!({
        sort_maps => true,
    }, {
        // Redact volatile fields (id, created_at) to ensure snapshot stability
        assert_json_snapshot!("event_data_input_message", data, {
            ".message.id" => "[MESSAGE_ID]",
            ".message.created_at" => "[TIMESTAMP]"
        });
    });
}

#[test]
fn snapshot_output_message_started() {
    let data = OutputMessageStartedData {
        reasoning_state: None,
        turn_id: test_turn_id(),
        message_id: test_message_id(),
        model: Some("gpt-5.2".to_string()),
        iteration: None,
        phase: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_output_message_started", data);
    });
}

#[test]
fn snapshot_output_message_delta() {
    let data = OutputMessageDeltaData {
        turn_id: test_turn_id(),
        message_id: test_message_id(),
        delta: "Hello".to_string(),
        accumulated: "Well, Hello".to_string(),
        phase: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_output_message_delta", data);
    });
}

#[test]
fn snapshot_output_message_completed() {
    let data = OutputMessageCompletedData::new(RuntimeMessage::assistant("Hello!"));
    with_settings!({
        sort_maps => true,
    }, {
        // Redact volatile fields (id, created_at) to ensure snapshot stability
        assert_json_snapshot!("event_data_output_message_completed", data, {
            ".message.id" => "[MESSAGE_ID]",
            ".message.created_at" => "[TIMESTAMP]"
        });
    });
}

#[test]
fn snapshot_turn_started() {
    let data = TurnStartedData {
        turn_id: test_turn_id(),
        input_message_id: test_message_id(),
        input_content: Some("Hello".to_string()),
        agent_id: None,
        agent_name: None,
        agent_description: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_turn_started", data);
    });
}

#[test]
fn snapshot_turn_completed() {
    let data = TurnCompletedData {
        turn_id: test_turn_id(),
        iterations: 3,
        duration_ms: Some(1500),
        usage: Some(TokenUsage::new(100, 50)),
        input_content: None,
        final_message_id: Some(test_message_id()),
        final_answer_preview: Some("Done.".to_string()),
        time_to_first_token_ms: Some(120),
        tool_call_count: Some(2),
        llm_call_count: Some(3),
        status: Some("completed".to_string()),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_turn_completed", data);
    });
}

#[test]
fn snapshot_turn_failed() {
    let data = TurnFailedData {
        turn_id: test_turn_id(),
        error: "Rate limit exceeded".to_string(),
        error_code: Some("RATE_LIMIT".to_string()),
        error_fields: None,
        error_disclosure: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_turn_failed", data);
    });
}

#[test]
fn snapshot_turn_cancelled() {
    let data = TurnCancelledData {
        turn_id: test_turn_id(),
        reason: Some("User requested".to_string()),
        usage: Some(TokenUsage::new(50, 25)),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_turn_cancelled", data);
    });
}

#[test]
fn snapshot_reason_started() {
    let data = ReasonStartedData {
        harness_id: test_harness_id(),
        agent_id: Some(test_agent_id()),
        metadata: Some(ModelMetadata {
            model: "gpt-5.2".to_string(),
            model_id: None,
            provider_id: None,
        }),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_reason_started", data);
    });
}

#[test]
fn snapshot_reason_completed() {
    let data = ReasonCompletedData::success(
        "Hello world",
        true,
        2,
        Some(1000),
        Some(TokenUsage::new(100, 50)),
    );
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_reason_completed", data);
    });
}

#[test]
fn snapshot_act_started() {
    let data = ActStartedData {
        tool_calls: vec![ToolCallSummary {
            id: "tc_1".to_string(),
            name: "get_weather".to_string(),
            display_name: None,
            narration: None,
            completed_narration: None,
        }],
        headline: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_act_started", data);
    });
}

#[test]
fn snapshot_act_completed() {
    let data = ActCompletedData {
        completed: true,
        success_count: 2,
        error_count: 0,
        duration_ms: Some(500),
        headline: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_act_completed", data);
    });
}

#[test]
fn snapshot_tool_started() {
    let data = ToolStartedData {
        tool_call: ToolCall {
            id: "tc_1".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({"city": "London"}),
        },
        tool_call_fingerprint: None,
        display_name: None,
        narration: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_tool_started", data);
    });
}

#[test]
fn snapshot_tool_completed() {
    let data = ToolCompletedData::success(
        "tc_1".to_string(),
        "get_weather".to_string(),
        vec![crate::message::ContentPart::text("Sunny, 22°C")],
        Some(250),
    );
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_tool_completed", data);
    });
}

#[test]
fn snapshot_llm_generation() {
    let data = LlmGenerationData::success(
        vec![RuntimeMessage::user("Hello")],
        vec![ToolDefinitionSummary {
            name: "tool1".to_string(),
            display_name: None,
            category: None,
            capability_id: None,
            capability_name: None,
            description: "A tool".to_string(),
        }],
        Some("Hi there!".to_string()),
        vec![],
        "gpt-5.2".to_string(),
        Some("openai".to_string()),
        Some(TokenUsage::new(10, 5)),
        Some(100),
        Some(25),
    );
    with_settings!({
        sort_maps => true,
    }, {
        // Redact volatile fields (id, created_at) in messages array
        assert_json_snapshot!("event_data_llm_generation", data, {
            ".messages[].id" => "[MESSAGE_ID]",
            ".messages[].created_at" => "[TIMESTAMP]"
        });
    });
}

#[test]
fn snapshot_reason_thinking_started() {
    let data = ReasonThinkingStartedData {
        turn_id: test_turn_id(),
        model: Some("claude-4-opus".to_string()),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_reason_thinking_started", data);
    });
}

#[test]
fn snapshot_reason_thinking_delta() {
    let data = ReasonThinkingDeltaData {
        turn_id: test_turn_id(),
        delta: "Let me think...".to_string(),
        accumulated: "First thought. Let me think...".to_string(),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_reason_thinking_delta", data);
    });
}

#[test]
fn snapshot_reason_thinking_completed() {
    let data = ReasonThinkingCompletedData {
        turn_id: test_turn_id(),
        thinking: "I need to consider...".to_string(),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_reason_thinking_completed", data);
    });
}

#[test]
fn snapshot_reason_item() {
    let data = ReasonItemData {
        turn_id: test_turn_id(),
        provider: "openai".to_string(),
        model: Some("gpt-5.5".to_string()),
        item_id: "rs_test".to_string(),
        summary: vec!["safe summary".to_string()],
        token_count: Some(42),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_reason_item", data);
    });
}

#[test]
fn snapshot_session_started() {
    let data = SessionStartedData {
        harness_id: test_harness_id(),
        agent_id: Some(test_agent_id()),
        model_id: None,
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_session_started", data);
    });
}

#[test]
fn snapshot_session_activated() {
    let data = SessionActivatedData {
        turn_id: test_turn_id(),
        input_message_id: test_message_id(),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_session_activated", data);
    });
}

#[test]
fn snapshot_session_idled() {
    let data = SessionIdledData {
        turn_id: test_turn_id(),
        iterations: Some(3),
        usage: Some(TokenUsage::new(500, 200)),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_session_idled", data);
    });
}

#[test]
fn snapshot_session_title_updated() {
    let data = SessionTitleUpdatedData {
        previous_title: Some("Old title".to_string()),
        title: "Automatic Session Titles".to_string(),
    };
    with_settings!({
        sort_maps => true,
    }, {
        assert_json_snapshot!("event_data_session_title_updated", data);
    });

    let untitled = serde_json::to_value(SessionTitleUpdatedData {
        previous_title: None,
        title: "First title".to_string(),
    })
    .expect("serialize title event");
    assert!(untitled["previous_title"].is_null());
}

// ========================================================================
// Display Name Tests
// ========================================================================
// Verify display_name propagation through event data types.

#[test]
fn tool_call_summary_display_name_wire_contract() {
    for display_name in [None, Some("Get Weather")] {
        let summary = ToolCallSummary {
            id: "tc_1".into(),
            name: "get_weather".into(),
            display_name: display_name.map(str::to_owned),
            narration: None,
            completed_narration: None,
        };
        let mut expected = serde_json::json!({"id": "tc_1", "name": "get_weather"});
        if let Some(name) = display_name {
            expected["display_name"] = name.into();
        }
        assert_eq!(serde_json::to_value(summary).unwrap(), expected);
        let decoded: ToolCallSummary = serde_json::from_value(expected.clone()).unwrap();
        assert_eq!(decoded.display_name.as_deref(), display_name);
        assert_eq!(serde_json::to_value(decoded).unwrap(), expected);
    }
}

#[test]
fn act_started_with_definitions_populates_display_names() {
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};

    let tool_calls = vec![
        ToolCall {
            id: "tc_1".to_string(),
            name: "get_weather".to_string(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            id: "tc_2".to_string(),
            name: "unknown_tool".to_string(),
            arguments: serde_json::json!({}),
        },
    ];
    let tool_defs = vec![crate::tool_types::ToolDefinition::Builtin(BuiltinTool {
        name: "get_weather".to_string(),
        display_name: Some("Get Weather".to_string()),
        description: "Gets weather".to_string(),
        parameters: serde_json::json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: crate::tool_types::ToolHints::default(),
        full_parameters: None,
    })];

    let data = ActStartedData::with_definitions(&tool_calls, &tool_defs);
    assert_eq!(data.tool_calls.len(), 2);
    assert_eq!(
        data.tool_calls[0].display_name.as_deref(),
        Some("Get Weather")
    );
    assert_eq!(
        data.tool_calls[0].narration.as_deref(),
        Some("Running Get Weather")
    );
    assert_eq!(
        data.tool_calls[0].completed_narration.as_deref(),
        Some("Ran Get Weather")
    );
    assert_eq!(data.tool_calls[1].display_name, None);
}

#[test]
fn tool_completed_with_display_name_roundtrip() {
    let data = ToolCompletedData::success(
        "tc_1".to_string(),
        "get_weather".to_string(),
        vec![crate::message::ContentPart::text("Sunny")],
        Some(100),
    )
    .with_display_name(Some("Get Weather".to_string()));

    assert_eq!(data.display_name.as_deref(), Some("Get Weather"));

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["display_name"], "Get Weather");

    let deserialized: ToolCompletedData = serde_json::from_value(json).unwrap();
    assert_eq!(deserialized.display_name.as_deref(), Some("Get Weather"));
}

#[test]
fn tool_started_display_name_serialization() {
    let data = ToolStartedData {
        tool_call: ToolCall {
            id: "tc_1".to_string(),
            name: "bash".to_string(),
            arguments: serde_json::json!({"command": "ls"}),
        },
        tool_call_fingerprint: None,
        display_name: Some("Bash".to_string()),
        narration: None,
    };

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(json["display_name"], "Bash");
}

#[test]
fn tool_definition_summary_display_name() {
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolPolicy};

    let def = crate::tool_types::ToolDefinition::Builtin(BuiltinTool {
        name: "read_file".to_string(),
        display_name: Some("Read File".to_string()),
        description: "Reads a file".to_string(),
        parameters: serde_json::json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::default(),
        hints: crate::tool_types::ToolHints::default(),
        full_parameters: None,
    });

    let summary = ToolDefinitionSummary::from(&def);
    assert_eq!(summary.display_name.as_deref(), Some("Read File"));

    let json = serde_json::to_value(&summary).unwrap();
    assert_eq!(json["display_name"], "Read File");
}

// ========================================================================
// Forward Compatibility Tests
// ========================================================================
// These tests verify that unknown fields and types are handled correctly
// per the contract specification.

#[test]
fn forward_compat_unknown_fields_ignored() {
    // Unknown fields should be silently ignored during deserialization
    let json = r#"{
        "turn_id": "turn_00000000000000000000000000000002",
        "iterations": 3,
        "duration_ms": 1500,
        "usage": {"input_tokens": 100, "output_tokens": 50},
        "future_field": "should be ignored",
        "another_new_field": 42
    }"#;

    let data: TurnCompletedData = serde_json::from_str(json).unwrap();
    assert_eq!(data.iterations, 3);
    assert_eq!(data.duration_ms, Some(1500));
    assert_eq!(data.turn_id, test_turn_id());
    assert_eq!(
        serde_json::to_value(data.usage).unwrap(),
        json!({"input_tokens": 100, "output_tokens": 50})
    );
}

#[test]
fn forward_compat_unsupported_preserves_data() {
    // Unsupported events should preserve the original data for debugging
    // Unknown types and malformed known payloads preserve diagnostic data.
    for (kind, original) in [
        (
            "unknown.event",
            serde_json::json!({"key": "value", "nested": {"a": 1}}),
        ),
        (
            "turn.completed",
            serde_json::json!({"iterations": "invalid"}),
        ),
    ] {
        let data = deserialize_event_data(kind, original.clone());
        assert!(data.is_unsupported());
        assert_eq!(data.event_type(), "unsupported");
        match data {
            EventData::Unsupported { event_type, data } => {
                assert_eq!(event_type, kind);
                assert_eq!(data, original);
            }
            _ => panic!("Expected Unsupported variant"),
        }
    }
}

#[test]
fn forward_compat_optional_fields_absent() {
    // Optional fields can be absent without causing errors
    let json = r#"{
        "turn_id": "turn_00000000000000000000000000000002",
        "iterations": 3
    }"#;

    let data: TurnCompletedData = serde_json::from_str(json).unwrap();
    assert_eq!(data.iterations, 3);
    assert!(data.duration_ms.is_none());
    assert!(data.usage.is_none());
    assert!(data.input_content.is_none());
    assert!(data.final_message_id.is_none());
    assert!(data.final_answer_preview.is_none());
    assert!(data.time_to_first_token_ms.is_none());
    assert!(data.tool_call_count.is_none());
    assert!(data.llm_call_count.is_none());
    assert!(data.status.is_none());
}

// ========================================================================
// Round-Trip Serialization Tests
// ========================================================================
// These tests verify that events survive serialization/deserialization.

#[test]
fn representative_event_payloads_preserve_wire_identity() {
    // Independent wire literals catch identity changes; payload equality catches field loss.
    let test_cases: Vec<(&str, EventData)> = vec![
        (
            "input.message",
            InputMessageData::new(RuntimeMessage::user("test")).into(),
        ),
        (
            "output.message.started",
            OutputMessageStartedData {
                reasoning_state: None,
                turn_id: test_turn_id(),
                message_id: test_message_id(),
                model: None,
                iteration: None,
                phase: None,
            }
            .into(),
        ),
        (
            "output.message.delta",
            OutputMessageDeltaData {
                turn_id: test_turn_id(),
                message_id: test_message_id(),
                delta: "x".to_string(),
                accumulated: "x".to_string(),
                phase: None,
            }
            .into(),
        ),
        (
            "output.message.completed",
            OutputMessageCompletedData::new(RuntimeMessage::assistant("hi")).into(),
        ),
        (
            "turn.started",
            TurnStartedData {
                turn_id: test_turn_id(),
                input_message_id: test_message_id(),
                input_content: None,
                agent_id: None,
                agent_name: None,
                agent_description: None,
            }
            .into(),
        ),
        (
            "turn.completed",
            TurnCompletedData {
                turn_id: test_turn_id(),
                iterations: 1,
                duration_ms: None,
                usage: None,
                input_content: None,
                final_message_id: None,
                final_answer_preview: None,
                time_to_first_token_ms: None,
                tool_call_count: None,
                llm_call_count: None,
                status: None,
            }
            .into(),
        ),
        (
            "turn.failed",
            TurnFailedData {
                turn_id: test_turn_id(),
                error: "err".to_string(),
                error_code: None,
                error_fields: None,
                error_disclosure: None,
            }
            .into(),
        ),
        (
            "turn.cancelled",
            TurnCancelledData {
                turn_id: test_turn_id(),
                reason: None,
                usage: None,
            }
            .into(),
        ),
        (
            "turn.sealed",
            TurnSealedData {
                turn_id: test_turn_id(),
                reason: "no_progress".to_string(),
                detail: Some("sealed".to_string()),
                iterations: Some(3),
                usage: None,
            }
            .into(),
        ),
        (
            "reason.started",
            ReasonStartedData {
                harness_id: test_harness_id(),
                agent_id: Some(test_agent_id()),
                metadata: None,
            }
            .into(),
        ),
        (
            "reason.completed",
            ReasonCompletedData::success("", false, 0, None, None).into(),
        ),
        (
            "act.started",
            ActStartedData {
                tool_calls: vec![],
                headline: None,
            }
            .into(),
        ),
        (
            "act.completed",
            ActCompletedData {
                completed: true,
                success_count: 0,
                error_count: 0,
                duration_ms: None,
                headline: None,
            }
            .into(),
        ),
        (
            "session.started",
            SessionStartedData {
                harness_id: test_harness_id(),
                agent_id: Some(test_agent_id()),
                model_id: None,
            }
            .into(),
        ),
        (
            "session.activated",
            SessionActivatedData {
                turn_id: test_turn_id(),
                input_message_id: test_message_id(),
            }
            .into(),
        ),
        (
            "session.idled",
            SessionIdledData {
                turn_id: test_turn_id(),
                iterations: None,
                usage: None,
            }
            .into(),
        ),
        (
            "session.title.updated",
            SessionTitleUpdatedData {
                previous_title: Some("Old title".to_string()),
                title: "New title".to_string(),
            }
            .into(),
        ),
        (
            "session.model.changed",
            SessionModelChangedData {
                previous_model_id: Some(test_model_id()),
                previous_model_name: Some("GPT-5.6 Sol".to_string()),
                model_id: test_model_id(),
                model_name: "GPT-5.6 Terra".to_string(),
            }
            .into(),
        ),
        (
            "tool.started",
            ToolStartedData {
                tool_call: ToolCall {
                    id: "call_1".into(),
                    name: "lookup".into(),
                    arguments: json!({"q": "test"}),
                },
                tool_call_fingerprint: None,
                display_name: Some("Lookup".into()),
                narration: None,
            }
            .into(),
        ),
        (
            "tool.completed",
            ToolCompletedData::success(
                "call_1".into(),
                "lookup".into(),
                vec![ContentPart::text("result")],
                Some(7),
            )
            .into(),
        ),
        (
            "llm.generation",
            LlmGenerationData::success(
                vec![RuntimeMessage::user("prompt")],
                vec![],
                Some("answer".into()),
                vec![],
                "model".into(),
                None,
                None,
                Some(9),
                Some(2),
            )
            .into(),
        ),
        (
            "reason.thinking.started",
            ReasonThinkingStartedData {
                turn_id: test_turn_id(),
                model: Some("reasoner".into()),
            }
            .into(),
        ),
        (
            "reason.thinking.delta",
            ReasonThinkingDeltaData {
                turn_id: test_turn_id(),
                delta: "next".into(),
                accumulated: "first next".into(),
            }
            .into(),
        ),
        (
            "reason.thinking.completed",
            ReasonThinkingCompletedData {
                turn_id: test_turn_id(),
                thinking: "complete thought".into(),
            }
            .into(),
        ),
        (
            "reason.item",
            ReasonItemData {
                turn_id: test_turn_id(),
                provider: "openai".into(),
                model: Some("reasoner".into()),
                item_id: "rs_1".into(),
                summary: vec!["safe".into()],
                token_count: Some(42),
            }
            .into(),
        ),
    ];

    for (event_type, original) in test_cases {
        assert_eq!(original.event_type(), event_type);
        assert!(
            VALID_EVENT_TYPES.contains(&event_type),
            "filter must accept {event_type}"
        );
        assert!(!original.is_unsupported());
        let json = serde_json::to_value(&original).unwrap();
        let deserialized = deserialize_event_data(event_type, json.clone());
        assert_eq!(
            std::mem::discriminant(&original),
            std::mem::discriminant(&deserialized),
            "{event_type}"
        );
        assert_eq!(
            serde_json::to_value(deserialized).unwrap(),
            json,
            "{event_type}"
        );
        let request = EventRequest::new(test_session_id(), EventContext::empty(), original);
        assert_eq!(serde_json::to_value(request).unwrap()["type"], event_type);
    }
}

// ========================================================================
// Event Structure Tests
// ========================================================================
// Tests for the Event container structure

#[test]
fn event_structure_has_required_fields() {
    let ts = "2026-01-02T03:04:05Z".parse::<DateTime<Utc>>().unwrap();
    let message = RuntimeMessage::user("test").with_id(test_message_id());
    let message_json = serde_json::to_value(&message).unwrap();
    let event_id = EventId::from_uuid(Uuid::from_u128(7));
    let context = EventContext::turn(test_turn_id(), test_message_id());
    let mut event = Event::with_id(
        event_id,
        test_session_id(),
        context,
        InputMessageData::new(message),
    )
    .with_metadata(json!({"source": "replay"}))
    .with_tags(vec!["audit".into()])
    .with_sequence(12);
    event.ts = ts;
    let expected = json!({
        "id": "event_00000000000000000000000000000007",
        "type": "input.message", "ts": "2026-01-02T03:04:05Z",
        "session_id": "session_00000000000000000000000000000001",
        "context": {"turn_id": "turn_00000000000000000000000000000002",
            "input_message_id": "message_00000000000000000000000000000003"},
        "data": {"message": message_json}, "metadata": {"source": "replay"},
        "tags": ["audit"], "sequence": 12
    });
    assert_eq!(serde_json::to_value(&event).unwrap(), expected);
    let decoded: Event = serde_json::from_value(expected.clone()).unwrap();
    assert!(matches!(&decoded.data, EventData::InputMessage(_)));
    assert_eq!(serde_json::to_value(decoded).unwrap(), expected);

    let mut request_json = expected.clone();
    request_json.as_object_mut().unwrap().remove("id");
    request_json.as_object_mut().unwrap().remove("sequence");
    let request: EventRequest = serde_json::from_value(request_json.clone()).unwrap();
    assert_eq!(serde_json::to_value(&request).unwrap(), request_json);
    assert_eq!(
        serde_json::to_value(request.into_event(event_id, 12)).unwrap(),
        expected
    );
}

#[test]
fn event_context_span_fields() {
    let context = EventContext::empty().with_span(
        "trace123".to_string(),
        "span456".to_string(),
        Some("parent789".to_string()),
    );

    let json = serde_json::to_value(&context).unwrap();
    assert_eq!(
        json.get("trace_id").and_then(|v| v.as_str()),
        Some("trace123")
    );
    assert_eq!(
        json.get("span_id").and_then(|v| v.as_str()),
        Some("span456")
    );
    assert_eq!(
        json.get("parent_span_id").and_then(|v| v.as_str()),
        Some("parent789")
    );
}
