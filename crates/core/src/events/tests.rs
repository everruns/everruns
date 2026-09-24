//! Tests moved out of events.rs: tests.

use crate::typed_id::{AgentId, EventId, ExecId, HarnessId, MessageId, SessionId, TurnId};
use chrono::Utc;
use std::collections::HashMap;
use uuid::Uuid;

use super::*;
use crate::driver_registry::PromptCacheStrategy;
use serde_json::json;

/// Build an assistant message whose reasoning part carries every kind of
/// opaque replay state, plus readable text that must survive.
fn message_with_replay_state() -> RuntimeMessage {
    let mut message = RuntimeMessage::assistant("the answer");
    message.content.push(ContentPart::reasoning(
        everruns_provider::reasoning::ReasoningContentPart::opaque("anthropic")
            .with_item_id("rs_abc")
            .with_signature("sig-do-not-publish")
            .with_encrypted("enc-do-not-publish")
            .with_text(everruns_provider::reasoning::ReasoningText::Plain {
                text: "visible reasoning".to_string(),
            }),
    ));
    message.content.push(ContentPart::ProviderOpaque(
        everruns_provider::ProviderOpaqueContent::new(
            "anthropic",
            json!([{"type": "thinking", "signature": "OPAQUE-SIGNATURE"}]),
        ),
    ));
    message
}

fn generation_metadata() -> LlmGenerationMetadata {
    LlmGenerationMetadata {
        model: "test-model".to_string(),
        provider: Some("anthropic".to_string()),
        response_model: None,
        usage: None,
        duration_ms: None,
        time_to_first_token_ms: None,
        success: true,
        error: None,
        finish_reasons: None,
        response_id: None,
        retry: None,
        compaction: None,
        request_options: None,
    }
}

fn reasoning_part(message: &RuntimeMessage) -> &everruns_provider::reasoning::ReasoningContentPart {
    message
        .content
        .iter()
        .find_map(|part| match part {
            ContentPart::Reasoning(r) => Some(r),
            _ => None,
        })
        .expect("reasoning part")
}
fn assert_no_provider_opaque(message: &RuntimeMessage) {
    assert!(
        !message
            .content
            .iter()
            .any(|part| matches!(part, ContentPart::ProviderOpaque(_)))
    );
}

/// The events read path must strip the same replay state the message read
/// path strips, or `GET .../events` republishes what `GET .../messages`
/// withholds (EVE-933).
#[test]
fn into_public_strips_replay_state_from_completed_messages() {
    let data = EventData::OutputMessageCompleted(OutputMessageCompletedData::new(
        message_with_replay_state(),
    ));

    let EventData::OutputMessageCompleted(public) = data.into_public() else {
        panic!("variant must be preserved");
    };
    let part = reasoning_part(&public.message);
    assert_no_provider_opaque(&public.message);

    assert_eq!(part.signature, None, "signature must not be published");
    assert_eq!(
        part.encrypted, None,
        "encrypted payload must not be published"
    );
    assert_eq!(
        part.item_id.as_deref(),
        Some("rs_abc"),
        "the provider id is an identifier, not replay state"
    );
    assert_eq!(
        part.display_text().as_deref(),
        Some("visible reasoning"),
        "readable reasoning is content and must survive"
    );
}

#[test]
fn into_public_strips_replay_state_from_input_and_generation_messages() {
    let EventData::InputMessage(input) =
        EventData::InputMessage(InputMessageData::new(message_with_replay_state())).into_public()
    else {
        panic!("variant must be preserved");
    };
    assert_eq!(reasoning_part(&input.message).signature, None);
    assert_no_provider_opaque(&input.message);
    assert_eq!(reasoning_part(&input.message).encrypted, None);
    assert_eq!(
        reasoning_part(&input.message).display_text().as_deref(),
        Some("visible reasoning")
    );

    // The prompt replayed to the provider carries the same artifacts.
    let generation = LlmGenerationData {
        messages: vec![message_with_replay_state()],
        tools: Vec::new(),
        output: LlmGenerationOutput {
            text: Some("the answer".to_string()),
            tool_calls: Vec::new(),
        },
        metadata: generation_metadata(),
    };
    let EventData::LlmGeneration(public) = EventData::LlmGeneration(generation).into_public()
    else {
        panic!("variant must be preserved");
    };
    assert_eq!(reasoning_part(&public.messages[0]).signature, None);
    assert_no_provider_opaque(&public.messages[0]);
    assert_eq!(reasoning_part(&public.messages[0]).encrypted, None);
    assert_eq!(
        reasoning_part(&public.messages[0])
            .display_text()
            .as_deref(),
        Some("visible reasoning")
    );
    assert_eq!(public.output.text.as_deref(), Some("the answer"));
}

/// `needs_public_projection` gates the SSE clone, so a variant it misses is
/// a variant that leaks.
#[test]
fn needs_public_projection_covers_every_message_bearing_variant() {
    assert!(
        EventData::InputMessage(InputMessageData::new(RuntimeMessage::user("hi")))
            .needs_public_projection()
    );
    assert!(
        EventData::OutputMessageCompleted(OutputMessageCompletedData::new(
            RuntimeMessage::assistant("hi")
        ))
        .needs_public_projection()
    );
    assert!(
        EventData::LlmGeneration(LlmGenerationData {
            messages: Vec::new(),
            tools: Vec::new(),
            output: LlmGenerationOutput {
                text: None,
                tool_calls: Vec::new(),
            },
            metadata: generation_metadata(),
        })
        .needs_public_projection()
    );
    assert!(
        !EventData::ReasonItem(ReasonItemData {
            turn_id: TurnId::new(),
            provider: "openai".to_string(),
            model: None,
            item_id: "rs_abc".to_string(),
            summary: Vec::new(),
            token_count: None,
        })
        .needs_public_projection(),
        "a message-free variant must not pay for a clone"
    );
}

#[test]
fn test_event_creation() {
    let session_id = SessionId::new();
    let context = EventContext::empty();
    let data = InputMessageData::new(RuntimeMessage::user("test"));

    let event = Event::new(session_id, context, data);

    assert_eq!(event.event_type, "input.message");
    assert_eq!(event.session_uuid(), session_id.uuid());
    assert!(event.is_input_event());
    assert!(event.is_message_event());
}

#[test]
fn test_event_context_from_execution_context() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();

    let atom_ctx = ExecutionContext::new(session_id, turn_id, input_message_id);
    let context = EventContext::from_execution_context(&atom_ctx);

    assert_eq!(context.turn_id, Some(turn_id));
    assert_eq!(context.input_message_id, Some(input_message_id));
    assert_eq!(context.exec_id, Some(atom_ctx.exec_id));
}

#[test]
fn transcript_repaired_is_valid_filter_event_type() {
    assert!(VALID_EVENT_TYPES.contains(&TRANSCRIPT_REPAIRED));
}

/// `capability.usage` is emitted (`EventData::CapabilityUsage`) and documented
/// as streamable, so the public `types`/`exclude` filter allowlist must accept
/// it instead of rejecting it as an unknown event type.
#[test]
fn capability_usage_is_valid_filter_event_type() {
    assert!(VALID_EVENT_TYPES.contains(&CAPABILITY_USAGE));
}

#[test]
fn test_event_builder() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();
    let input_message_id = MessageId::new();
    let exec_id = ExecId::new();

    let event = EventBuilder::new(session_id)
        .with_turn(turn_id, input_message_id)
        .with_exec(exec_id)
        .build(ReasonStartedData {
            harness_id: HarnessId::from_seed(1),
            agent_id: Some(AgentId::new()),
            metadata: Some(ModelMetadata {
                model: "gpt-5.2".to_string(),
                model_id: None,
                provider_id: None,
            }),
        });

    assert_eq!(event.event_type, "reason.started");
    assert_eq!(event.session_id, session_id);
    assert_eq!(event.context.turn_id, Some(turn_id));
    assert_eq!(event.context.input_message_id, Some(input_message_id));
    assert_eq!(event.context.exec_id, Some(exec_id));
}

#[test]
fn test_reason_completed_data() {
    let data = ReasonCompletedData::success("Hello world", true, 2, Some(1000), None);
    assert!(data.success);
    assert_eq!(data.text_preview, Some("Hello world".to_string()));
    assert!(data.has_tool_calls);
    assert_eq!(data.tool_call_count, 2);
    assert_eq!(data.duration_ms, Some(1000));
    assert!(data.usage.is_none());

    let data = ReasonCompletedData::failure("Network error".to_string(), Some(500));
    assert!(!data.success);
    assert_eq!(data.error, Some("Network error".to_string()));
    assert_eq!(data.duration_ms, Some(500));
}

#[test]
fn test_llm_generation_data_success() {
    let messages = vec![
        RuntimeMessage::user("Hello"),
        RuntimeMessage::assistant("Hi there!"),
    ];
    let tools = vec![ToolDefinitionSummary {
        name: "get_weather".to_string(),
        display_name: None,
        category: None,
        capability_id: None,
        capability_name: None,
        description: "Get weather for a city".to_string(),
    }];
    let tool_calls = vec![ToolCall {
        id: "call_weather".into(),
        name: "get_weather".into(),
        arguments: json!({"city": "Kyiv"}),
    }];
    let data = LlmGenerationData::success(
        messages.clone(),
        tools.clone(),
        Some("Hi there!".to_string()),
        tool_calls.clone(),
        "gpt-5.2".to_string(),
        Some("openai".to_string()),
        Some(TokenUsage {
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            actual_cost_usd: None,
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }),
        Some(100),
        Some(25), // time_to_first_token_ms
    );

    assert_eq!(
        serde_json::to_value(&data.messages).unwrap(),
        serde_json::to_value(messages).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&data.tools).unwrap(),
        serde_json::to_value(tools).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&data.output.tool_calls).unwrap(),
        serde_json::to_value(tool_calls).unwrap()
    );
    assert_eq!(data.output.text.as_deref(), Some("Hi there!"));
    assert_eq!(
        serde_json::to_value(&data.metadata).unwrap(),
        json!({
            "model": "gpt-5.2", "provider": "openai", "success": true,
            "usage": {"input_tokens": 10, "output_tokens": 5},
            "duration_ms": 100, "time_to_first_token_ms": 25, "finish_reasons": ["tool_calls"]
        })
    );
}

#[test]
fn test_llm_generation_data_with_full_metadata() {
    let messages = vec![RuntimeMessage::user("Hello")];
    let data = LlmGenerationData::success_with_metadata(
        messages.clone(),
        vec![],
        Some("Hi!".to_string()),
        vec![],
        "claude-opus-5".to_string(),
        Some("anthropic".to_string()),
        Some(TokenUsage {
            input_tokens: 5,
            output_tokens: 3,
            cache_read_tokens: None,
            cache_creation_tokens: None,
            actual_cost_usd: None,
            estimated_cost_usd: None,
            effective_cost_usd: None,
        }),
        Some(50),
        Some(25), // time_to_first_token_ms
        Some(vec!["end_turn".to_string()]),
        Some("msg_12345".to_string()),
    );

    assert_eq!(
        serde_json::to_value(&data.messages).unwrap(),
        serde_json::to_value(&messages).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&data.metadata).unwrap(),
        json!({
            "model": "claude-opus-5", "provider": "anthropic",
            "usage": {"input_tokens": 5, "output_tokens": 3},
            "duration_ms": 50, "time_to_first_token_ms": 25,
            "success": true, "finish_reasons": ["end_turn"], "response_id": "msg_12345"
        })
    );
    assert_eq!(
        serde_json::to_value(&data.output).unwrap(),
        json!({"text": "Hi!"})
    );
    assert!(data.tools.is_empty());
}

#[test]
fn test_llm_generation_data_failure() {
    let messages = vec![RuntimeMessage::user("Hello")];
    let data = LlmGenerationData::failure(
        messages.clone(),
        vec![],
        "gpt-5.2".to_string(),
        Some("openai".to_string()),
        "Rate limit exceeded".to_string(),
        Some(50),
        None, // time_to_first_token_ms
    );

    assert_eq!(
        serde_json::to_value(&data.messages).unwrap(),
        serde_json::to_value(messages).unwrap()
    );
    assert_eq!(
        serde_json::to_value(&data.metadata).unwrap(),
        json!({
            "model": "gpt-5.2", "provider": "openai", "success": false,
            "error": "Rate limit exceeded", "duration_ms": 50, "finish_reasons": ["error"]
        })
    );
    assert_eq!(serde_json::to_value(&data.output).unwrap(), json!({}));
    assert!(data.tools.is_empty());
}

#[test]
fn test_delta_events_are_ephemeral() {
    let session_id = SessionId::new();
    let turn_id = TurnId::new();

    let voice = VoiceTranscriptData {
        voice_connection_id: "voice_1".into(),
        item_id: Some("item_2".into()),
        response_id: None,
        phase: None,
        delta: "world".into(),
        accumulated: "Hello world".into(),
    };
    let cases: Vec<(EventData, bool)> = vec![
        (
            OutputMessageDeltaData {
                turn_id,
                message_id: MessageId::new(),
                delta: "world".into(),
                accumulated: "Hello world".into(),
                phase: None,
            }
            .into(),
            true,
        ),
        (
            ReasonThinkingDeltaData {
                turn_id,
                delta: "next".into(),
                accumulated: "first next".into(),
            }
            .into(),
            true,
        ),
        (
            ToolOutputDeltaData {
                tool_call_id: "call_123".into(),
                tool_name: "bash".into(),
                delta: "line".into(),
                stream: "stdout".into(),
            }
            .into(),
            true,
        ),
        (EventData::VoiceInputTranscriptDelta(voice.clone()), true),
        (EventData::VoiceOutputTranscriptDelta(voice.clone()), true),
        (
            EventData::VoiceInputTranscriptCompleted(voice.clone()),
            false,
        ),
        (EventData::VoiceOutputTranscriptCompleted(voice), false),
        (
            OutputMessageCompletedData::new(RuntimeMessage::assistant("done")).into(),
            false,
        ),
        (
            LlmGenerationData::success(
                vec![],
                vec![],
                Some("done".into()),
                vec![],
                "model".into(),
                None,
                None,
                None,
                None,
            )
            .into(),
            false,
        ),
    ];
    for (data, expected) in cases {
        let request = EventRequest::new(session_id, EventContext::empty(), data);
        assert_eq!(
            request.is_ephemeral(),
            expected,
            "{} request",
            request.event_type
        );
        let event = request.into_event(EventId::new(), 7);
        assert_eq!(event.is_ephemeral(), expected, "{} event", event.event_type);
    }
}

#[test]
fn test_llm_generation_data_with_request_options() {
    let mut provider_options = HashMap::new();
    provider_options.insert(
        "openai".to_string(),
        json!({ "previous_response_id": true }),
    );

    let data = LlmGenerationData::success(
        vec![RuntimeMessage::user("Hello")],
        vec![],
        Some("Hi".to_string()),
        vec![],
        "gpt-5.4".to_string(),
        Some("openai".to_string()),
        None,
        Some(42),
        Some(12),
    )
    .with_request_options(LlmRequestOptions {
        temperature: Some(0.5),
        max_tokens: Some(256),
        reasoning_effort: Some("high".into()),
        stream: Some(false),
        prompt_cache: Some(LlmPromptCacheInfo {
            enabled: true,
            strategy: PromptCacheStrategy::Auto,
            provider_mode: Some("prompt_cache_key".to_string()),
        }),
        tool_search: Some(LlmToolSearchInfo {
            enabled: true,
            threshold: 8,
        }),
        provider_options,
        metadata: HashMap::from([("project".into(), "test".into())]),
    });

    let json = serde_json::to_value(&data).unwrap();
    assert_eq!(
        json["metadata"]["request_options"],
        json!({
            "temperature": 0.5, "max_tokens": 256, "reasoning_effort": "high", "stream": false,
            "prompt_cache": {"enabled": true, "strategy": "auto", "provider_mode": "prompt_cache_key"},
            "tool_search": {"enabled": true, "threshold": 8},
            "provider_options": {"openai": {"previous_response_id": true}}, "metadata": {"project": "test"}
        })
    );
    let empty = LlmGenerationData::success(
        vec![],
        vec![],
        None,
        vec![],
        "model".into(),
        None,
        None,
        None,
        None,
    )
    .with_request_options(LlmRequestOptions::default());
    assert!(
        serde_json::to_value(empty).unwrap()["metadata"]
            .get("request_options")
            .is_none()
    );
}

#[test]
fn test_output_message_started_data_without_model() {
    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let data = OutputMessageStartedData {
        reasoning_state: None,
        turn_id,
        message_id: MessageId::new(),
        model: None,
        iteration: None,
        phase: None,
    };

    // Model should be skipped when None
    let json = serde_json::to_value(&data).unwrap();
    assert!(json.get("model").is_none());
}

#[test]
fn test_output_message_lifecycle_shares_message_id() {
    let turn_id = TurnId::new();
    let message_id = MessageId::from_uuid(Uuid::from_u128(3));
    let next_message_id = MessageId::from_uuid(Uuid::from_u128(4));

    let started = OutputMessageStartedData {
        reasoning_state: None,
        turn_id,
        message_id,
        model: None,
        iteration: Some(1),
        phase: None,
    };
    let next_started = OutputMessageStartedData {
        reasoning_state: None,
        turn_id,
        message_id: next_message_id,
        model: None,
        iteration: Some(2),
        phase: None,
    };
    let delta = OutputMessageDeltaData {
        turn_id,
        message_id,
        delta: "Hello".to_string(),
        accumulated: "Hello".to_string(),
        phase: None,
    };
    let replaced = OutputMessageReplacedData {
        turn_id,
        message_id,
        guardrail_capability_id: "guardrails".to_string(),
        guardrail_id: "example".to_string(),
        reason_code: "blocked".to_string(),
        replacement: "Safe response".to_string(),
    };
    let completed = OutputMessageCompletedData::new(
        RuntimeMessage::assistant("Safe response").with_id(message_id),
    );

    for value in [
        serde_json::to_value(started).unwrap(),
        serde_json::to_value(delta).unwrap(),
        serde_json::to_value(replaced).unwrap(),
    ] {
        assert_eq!(value["message_id"], message_id.to_string());
    }
    let completed = serde_json::to_value(completed).unwrap();
    assert_eq!(completed["message"]["id"], message_id.to_string());
    assert_eq!(
        completed["message"]["content"],
        json!([{"type": "text", "text": "Safe response"}])
    );
    let next = serde_json::to_value(next_started).unwrap();
    assert_eq!(next["message_id"], next_message_id.to_string());
    assert_eq!(next["iteration"], 2);
}

#[test]
fn test_output_message_phase_hint_serde() {
    let turn_id = TurnId::new();
    let message_id = MessageId::new();
    for (phase, wire) in [
        (None, None),
        (Some(ExecutionPhase::Commentary), Some("commentary")),
        (Some(ExecutionPhase::FinalAnswer), Some("final_answer")),
    ] {
        let cases: Vec<(&str, EventData)> = vec![
            (
                "output.message.started",
                OutputMessageStartedData {
                    reasoning_state: None,
                    turn_id,
                    message_id,
                    model: None,
                    iteration: None,
                    phase,
                }
                .into(),
            ),
            (
                "output.message.delta",
                OutputMessageDeltaData {
                    turn_id,
                    message_id,
                    delta: "done".into(),
                    accumulated: "nearly done".into(),
                    phase,
                }
                .into(),
            ),
        ];
        for (kind, data) in cases {
            let json = serde_json::to_value(&data).unwrap();
            assert_eq!(
                json.get("phase"),
                wire.map(serde_json::Value::from).as_ref(),
                "{kind}"
            );
            let decoded = deserialize_event_data(kind, json.clone());
            assert!(!decoded.is_unsupported(), "{kind}");
            assert_eq!(serde_json::to_value(decoded).unwrap(), json);
        }
    }
}

#[test]
fn test_output_message_delta_deserialization_preserves_fields() {
    let json = serde_json::json!({
        "turn_id": TurnId::new(), "message_id": MessageId::new(),
        "delta": "world", "accumulated": "Hello world", "phase": "final_answer"
    });
    let deserialized = deserialize_event_data("output.message.delta", json.clone());
    assert!(matches!(&deserialized, EventData::OutputMessageDelta(_)));
    assert_eq!(serde_json::to_value(deserialized).unwrap(), json);
}

#[test]
fn test_output_message_started_deserialization() {
    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let data = OutputMessageStartedData {
        reasoning_state: None,
        turn_id,
        message_id: MessageId::new(),
        model: Some("claude-opus-5".to_string()),
        iteration: Some(3),
        phase: Some(ExecutionPhase::FinalAnswer),
    };

    // Serialize to JSON
    let json = serde_json::to_value(EventData::OutputMessageStarted(data.clone())).unwrap();

    // Deserialize back through the real (type-driven) decode path
    let deserialized = deserialize_event_data("output.message.started", json.clone());

    assert!(matches!(&deserialized, EventData::OutputMessageStarted(_)));
    assert_eq!(serde_json::to_value(deserialized).unwrap(), json);
}

#[test]
fn test_event_deserialize_reason_item_uses_event_type_dispatch() {
    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let payload = serde_json::json!({
        "id": EventId::new().to_string(),
        "type": REASON_ITEM,
        "ts": Utc::now().to_rfc3339(),
        "session_id": SessionId::from_uuid(Uuid::now_v7()).to_string(),
        "context": {"trace_id": "t", "span_id": "s", "parent_span_id": null},
        "data": {
            "turn_id": turn_id.to_string(),
            "provider": "openai",
            "model": "gpt-5",
            "item_id": "rs_event",
            "summary": ["safe"],
            "token_count": 9
        }
    });

    let event: Event = serde_json::from_value(payload.clone()).expect("event deserializes");
    assert_eq!(serde_json::to_value(&event.data).unwrap(), payload["data"]);
    match event.data {
        EventData::ReasonItem(data) => {
            assert_eq!(data.turn_id, turn_id);
            assert_eq!(data.provider, "openai");
            assert_eq!(data.item_id, "rs_event");
            assert_eq!(data.token_count, Some(9));
        }
        other => panic!("expected reason.item data, got {}", other.event_type()),
    }
}

#[test]
fn test_event_request_deserialize_reason_item_uses_event_type_dispatch() {
    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let payload = serde_json::json!({
        "type": REASON_ITEM,
        "ts": Utc::now().to_rfc3339(),
        "session_id": SessionId::from_uuid(Uuid::now_v7()).to_string(),
        "context": {"trace_id": "t", "span_id": "s", "parent_span_id": null},
        "data": {
            "turn_id": turn_id.to_string(),
            "provider": "openai",
            "item_id": "rs_request",
                "summary": ["safe"]
        }
    });

    let req: EventRequest = serde_json::from_value(payload.clone()).expect("request deserializes");
    assert_eq!(serde_json::to_value(&req.data).unwrap(), payload["data"]);
    match req.data {
        EventData::ReasonItem(data) => {
            assert_eq!(data.turn_id, turn_id);
            assert_eq!(data.provider, "openai");
            assert_eq!(data.item_id, "rs_request");
        }
        other => panic!("expected reason.item data, got {}", other.event_type()),
    }
}

/// `ReasonThinkingStartedData` only requires `turn_id`, so a richer
/// `reason.item` payload also satisfies it structurally. Decoding does not
/// rely on serde to disambiguate (`EventData` has no `Deserialize` impl, so
/// variant declaration order is irrelevant); `deserialize_event_data`
/// selects the variant from the outer `type` string. Guard that the overlap
/// exists and that type dispatch resolves `reason.item` to `ReasonItem`
/// (keeping `provider`, `item_id`, …) rather than the looser
/// `ReasonThinkingStarted`.
#[test]
fn test_reason_item_resolves_via_type_dispatch_despite_overlap() {
    // The two reasoning variants overlap structurally: ReasonThinkingStarted
    // (turn_id + optional model) accepts any superset, while ReasonItem
    // (turn_id + provider + item_id + …) is richer. Confirm both parse in
    // isolation, then that type dispatch picks ReasonItem.
    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let json = serde_json::json!({
        "turn_id": turn_id.to_string(),
        "provider": "openai",
        "model": "gpt-5",
        "item_id": "rs_keep",
        "summary": ["s"],
        "token_count": 7,
    });

    // Both candidate structs accept the payload in isolation
    // (ReasonThinkingStarted ignores the extra fields), proving the overlap.
    // The canonical path disambiguates via the event_type, not via any
    // declaration order.
    let as_thinking: ReasonThinkingStartedData =
        serde_json::from_value(json.clone()).expect("thinking ignores extra fields");
    assert_eq!(as_thinking.turn_id, turn_id);
    assert_eq!(as_thinking.model.as_deref(), Some("gpt-5"));

    let as_item: ReasonItemData =
        serde_json::from_value(json.clone()).expect("ReasonItem accepts payload");
    assert_eq!(as_item.item_id, "rs_keep");
    assert_eq!(as_item.provider, "openai");

    // Canonical parse via type dispatch: this is the path used by Event
    // and EventRequest deserialization (see `deserialize_event_data`).
    let event_data = deserialize_event_data(REASON_ITEM, json);
    match event_data {
        EventData::ReasonItem(out) => {
            assert_eq!(out.item_id, "rs_keep");
            assert_eq!(out.provider, "openai");
        }
        other => panic!(
            "Typed dispatcher must select ReasonItem for {REASON_ITEM}, got {}",
            other.event_type()
        ),
    }
}

/// Regression guard for EVE-485: the persisted `reason.item` event must
/// never carry plaintext hidden reasoning content, and must not carry the
/// opaque replay state either — signatures and encrypted context are what
/// the driver hands back to the provider, not event data.
/// Assert structurally on parsed JSON keys rather than substrings so a
/// payload value that happens to contain "content"/"thinking" cannot mask
/// the guard.
#[test]
fn test_reason_item_data_excludes_plaintext_reasoning() {
    let turn_id = TurnId::from_uuid(Uuid::now_v7());
    let data = ReasonItemData {
        turn_id,
        provider: "openai".to_string(),
        model: Some("gpt-5".to_string()),
        item_id: "rs_secret".to_string(),
        // Deliberately stuff the substrings the old guard checked into a
        // legitimate value to prove the structural check still rejects
        // them when present only as values.
        summary: vec!["safe summary mentioning content and thinking".to_string()],
        token_count: Some(1),
    };

    let value = serde_json::to_value(&data).expect("serializable");
    let object = value.as_object().expect("data serializes to JSON object");
    for forbidden in [
        "content",
        "reasoning_text",
        "thinking",
        "reasoning_content",
        "raw_reasoning",
        // Opaque replay state: carried on the message's reasoning parts and
        // handed back to the provider, never published as event data.
        "encrypted_content",
        "signature",
        "encrypted",
    ] {
        assert!(
            !object.contains_key(forbidden),
            "ReasonItemData JSON must not expose `{forbidden}` key, got: {object:?}",
        );
    }
    // The only sanctioned field that carries reasoning text.
    assert!(object.contains_key("summary"));
}

#[test]
fn test_llm_generation_ttft_omitted_when_none() {
    let messages = vec![RuntimeMessage::user("test")];
    let data = LlmGenerationData::success(
        messages,
        vec![],
        Some("response".to_string()),
        vec![],
        "model".to_string(),
        None,
        None,
        None,
        None, // time_to_first_token_ms
    );

    // TTFT should be None when passed as None
    assert!(data.metadata.time_to_first_token_ms.is_none());

    // Should not appear in JSON when None
    let json = serde_json::to_value(&data).unwrap();
    assert!(json["metadata"].get("time_to_first_token_ms").is_none());
}

/// The serving model is carried on the wire and omitted when the provider did
/// not report one. Downstream exporters read this JSON directly, so an
/// always-present `"response_model": null` would make "unreported" and
/// "reported as nothing" indistinguishable to them.
#[test]
fn response_model_serializes_only_when_the_provider_reported_one() {
    let unreported = serde_json::to_value(generation_metadata()).unwrap();
    assert!(
        unreported.get("response_model").is_none(),
        "an unreported serving model must not appear at all: {unreported}"
    );

    let mut meta = generation_metadata();
    meta.response_model = Some("test-model-20250929".to_string());
    let reported = serde_json::to_value(&meta).unwrap();
    assert_eq!(
        reported.get("response_model").and_then(|v| v.as_str()),
        Some("test-model-20250929")
    );

    let round_tripped: LlmGenerationMetadata = serde_json::from_value(reported).unwrap();
    assert_eq!(
        round_tripped.response_model.as_deref(),
        Some("test-model-20250929")
    );
    assert_eq!(round_tripped.model, "test-model");
}
