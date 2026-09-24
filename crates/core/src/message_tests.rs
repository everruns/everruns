use super::*;
use crate::driver_registry::{LlmContentPart, MessageContent};
use crate::tool_types::ToolCall;
use serde_json::json;

fn calls() -> Vec<ToolCall> {
    vec![
        ToolCall {
            id: "call_search".into(),
            name: "search".into(),
            arguments: json!({"q": "rust"}),
        },
        ToolCall {
            id: "call_fetch".into(),
            name: "fetch".into(),
            arguments: json!({"url": "https://example.com"}),
        },
    ]
}

fn assert_messages(actual: &[RuntimeMessage], expected: &[RuntimeMessage]) {
    assert_eq!(
        serde_json::to_value(actual).unwrap(),
        serde_json::to_value(expected).unwrap()
    );
}

#[test]
fn native_custom_call_survives_transcript_serialization_and_conversion() {
    let native = everruns_provider::native_async::NativeToolCall::Custom {
        call_id: "original-call".into(),
        name: "lookup".into(),
        input: "raw\nquery: \"value\"".into(),
        asynchronous: true,
    };
    let mut message = RuntimeMessage::assistant("");
    message.content.push(ContentPart::ToolCall(
        ToolCallContentPart::from_native(native.clone()).unwrap(),
    ));
    let restored: RuntimeMessage =
        serde_json::from_slice(&serde_json::to_vec(&message).unwrap()).unwrap();
    assert_eq!(restored.tool_calls()[0].native.as_ref(), Some(&native));
    let llm = crate::llm_conversions::llm_message_from_message(&restored);
    assert_eq!(llm.native_tool_calls, vec![native]);
    assert_eq!(llm.tool_calls.unwrap()[0].id, "original-call");
}
#[test]
fn provider_opaque_content_persists_internally_and_is_removed_from_public_messages() {
    let opaque = ProviderOpaqueContent::new(
        "anthropic",
        json!([{"type": "thinking", "signature": "PRIVATE-SIGNATURE"}]),
    );
    let mut message = RuntimeMessage::assistant("answer");
    message
        .content
        .push(ContentPart::ProviderOpaque(opaque.clone()));

    let restored: RuntimeMessage =
        serde_json::from_slice(&serde_json::to_vec(&message).unwrap()).unwrap();
    assert!(
        restored
            .content
            .contains(&ContentPart::ProviderOpaque(opaque.clone()))
    );
    let llm = crate::llm_conversions::llm_message_from_message(&restored);
    let MessageContent::Parts(parts) = llm.content else {
        panic!("opaque replay content must use multipart provider content");
    };
    assert_eq!(
        parts,
        vec![
            LlmContentPart::ProviderOpaque(opaque),
            LlmContentPart::Text {
                text: "answer".into()
            }
        ]
    );

    let public = restored.into_public();
    assert_eq!(public.content, vec![ContentPart::text("answer")]);
    assert!(
        !serde_json::to_string(&public)
            .unwrap()
            .contains("PRIVATE-SIGNATURE")
    );
}

#[test]
fn settled_transcripts_are_preserved_without_synthetic_results() {
    for messages in [
        vec![],
        vec![
            RuntimeMessage::user("Hello"),
            RuntimeMessage::assistant("Hi"),
        ],
        vec![
            RuntimeMessage::assistant_with_tools("Searching", vec![calls()[0].clone()]),
            RuntimeMessage::tool_result("call_search", Some(json!({"found": 2})), None),
        ],
    ] {
        assert_messages(&patch_dangling_tool_calls(&messages), &messages);
    }
}

#[test]
fn dangling_calls_get_only_missing_cancellations_and_patching_is_idempotent() {
    let messages = vec![
        RuntimeMessage::user("Search then fetch"),
        RuntimeMessage::assistant_with_tools("Working", calls()),
        RuntimeMessage::user("Never mind"),
        RuntimeMessage::tool_result("call_search", Some(json!({"found": 2})), None),
    ];
    let patched = patch_dangling_tool_calls(&messages);
    assert_eq!(patched.len(), 5);
    assert_messages(&patched[..2], &messages[..2]);
    assert_messages(&patched[3..], &messages[2..]);
    assert_eq!(patched[2].role, RuntimeMessageRole::ToolResult);
    assert_eq!(
        serde_json::to_value(&patched[2].content).unwrap(),
        json!([{
            "type": "tool_result", "tool_call_id": "call_fetch",
            "error": "cancelled - another message came in before it could be completed"
        }])
    );
    assert_messages(&patch_dangling_tool_calls(&patched), &patched);
}

#[test]
fn plain_message_constructors_preserve_role_and_text() {
    for (message, role, text) in [
        (
            RuntimeMessage::user("question"),
            RuntimeMessageRole::User,
            "question",
        ),
        (
            RuntimeMessage::assistant("answer"),
            RuntimeMessageRole::Agent,
            "answer",
        ),
        (
            RuntimeMessage::system("instruction"),
            RuntimeMessageRole::System,
            "instruction",
        ),
    ] {
        assert_eq!(message.role, role);
        assert_eq!(message.text(), Some(text));
        assert_eq!(message.content, vec![ContentPart::text(text)]);
        assert!(!message.has_tool_calls());
    }
}

#[test]
fn tool_result_constructor_preserves_result_and_error_fields() {
    for (result, error) in [
        (Some(json!({"count": 2})), None),
        (None, Some("timeout".to_owned())),
        (Some(json!(false)), Some("partial".to_owned())),
    ] {
        let message = RuntimeMessage::tool_result("call_result", result.clone(), error.clone());
        assert_eq!(message.role, RuntimeMessageRole::ToolResult);
        assert_eq!(message.tool_call_id(), Some("call_result"));
        assert_eq!(
            message.content,
            vec![ContentPart::tool_result("call_result", result, error)]
        );
    }
}

#[test]
fn assistant_tool_messages_preserve_calls_and_distinguish_empty_from_whitespace_text() {
    for text in ["", "   ", "Working"] {
        let message = RuntimeMessage::assistant_with_tools(text, calls());
        let tool_parts: Vec<_> = calls()
            .into_iter()
            .map(|c| ContentPart::tool_call(c.id, c.name, c.arguments))
            .collect();
        let mut expected = vec![];
        if !text.is_empty() {
            expected.push(ContentPart::text(text));
        }
        expected.extend(tool_parts);
        assert_eq!(message.role, RuntimeMessageRole::Agent);
        assert_eq!(message.text(), (!text.is_empty()).then_some(text));
        assert_eq!(message.content, expected);
        assert!(message.has_tool_calls());
        assert_eq!(
            serde_json::to_value(message.tool_calls()).unwrap(),
            serde_json::to_value(calls()).unwrap()
        );
    }
}

#[test]
fn openai_plain_messages_map_internal_roles_and_preserve_text() {
    for (message, expected) in [
        (
            RuntimeMessage::user("question"),
            json!({"role": "user", "content": "question"}),
        ),
        (
            RuntimeMessage::system("instruction"),
            json!({"role": "system", "content": "instruction"}),
        ),
        (
            RuntimeMessage::assistant("answer"),
            json!({"role": "assistant", "content": "answer"}),
        ),
    ] {
        assert_eq!(message.to_openai_format(), expected);
    }
}

#[test]
fn openai_tool_calls_preserve_ids_arguments_and_optional_text() {
    for text in ["", "Working"] {
        let message = RuntimeMessage::assistant_with_tools(text, calls());
        let mut expected = json!({"role": "assistant", "tool_calls": [
            {"id": "call_search", "type": "function", "function": {"name": "search", "arguments": "{\"q\":\"rust\"}"}},
            {"id": "call_fetch", "type": "function", "function": {"name": "fetch", "arguments": "{\"url\":\"https://example.com\"}"}}
        ]});
        if !text.is_empty() {
            expected["content"] = text.into();
        }
        assert_eq!(message.to_openai_format(), expected);
    }
}

#[test]
fn openai_tool_results_prefer_errors_and_preserve_call_identity() {
    for (result, error, content) in [
        (
            Some(json!({"temperature":72})),
            None,
            "{\"temperature\":72}",
        ),
        (None, Some("timeout"), "Error: timeout"),
        (
            Some(json!({"partial":true})),
            Some("partial failure"),
            "Error: partial failure",
        ),
        (None, None, "{}"),
    ] {
        let message = RuntimeMessage::tool_result("call_result", result, error.map(str::to_owned));
        assert_eq!(
            message.to_openai_format(),
            json!({"role":"tool", "tool_call_id":"call_result", "content":content})
        );
    }
}

#[test]
fn openai_content_parts_preserve_text_and_image_sources() {
    for (part, expected) in [
        (
            ContentPart::text("Hello"),
            json!({"type":"text", "text":"Hello"}),
        ),
        (
            ContentPart::image_url("https://example.com/img.png"),
            json!({"type":"image_url", "image_url":{"url":"https://example.com/img.png"}}),
        ),
        (
            ContentPart::Image(ImageContentPart::from_base64("YWJj", "image/jpeg")),
            json!({"type":"image_url", "image_url":{"url":"data:image/jpeg;base64,YWJj"}}),
        ),
        (
            ContentPart::Image(ImageContentPart {
                url: None,
                base64: Some("YWJj".into()),
                media_type: None,
            }),
            json!({"type":"image_url", "image_url":{"url":"data:image/png;base64,YWJj"}}),
        ),
        (
            ContentPart::Image(ImageContentPart {
                url: Some("https://example.com/preferred".into()),
                base64: Some("YWJj".into()),
                media_type: Some("image/jpeg".into()),
            }),
            json!({"type":"image_url", "image_url":{"url":"https://example.com/preferred"}}),
        ),
    ] {
        assert_eq!(part.to_openai_format(), Some(expected));
    }
    assert!(
        ContentPart::Image(ImageContentPart {
            url: None,
            base64: None,
            media_type: None
        })
        .to_openai_format()
        .is_none()
    );
}

#[test]
fn openai_content_parts_exclude_tool_file_and_reasoning_artifacts() {
    for part in [
        ContentPart::tool_call("call_1", "lookup", json!({})),
        ContentPart::tool_result("call_1", Some(json!(42)), None),
        ContentPart::image_file(ImageId::new()),
        ContentPart::reasoning(
            ReasoningContentPart::opaque("test").with_signature("private-signature"),
        ),
    ] {
        assert!(part.to_openai_format().is_none());
    }
}

#[test]
fn openai_message_content_preserves_multimodal_order_and_filters_unsupported_parts() {
    let mut message = RuntimeMessage::user("before");
    message
        .content
        .push(ContentPart::image_url("https://example.com/image"));
    message.content.push(ContentPart::text("after"));
    assert_eq!(
        message.to_openai_format(),
        json!({"role":"user", "content":[
            {"type":"text", "text":"before"}, {"type":"image_url", "image_url":{"url":"https://example.com/image"}},
            {"type":"text", "text":"after"}
        ]})
    );
    message.content = vec![
        ContentPart::tool_call("ignored", "tool", json!({})),
        ContentPart::text("kept"),
    ];
    assert_eq!(
        message.to_openai_format(),
        json!({"role":"user", "content":"kept"})
    );
    message.content.remove(1);
    assert_eq!(
        message.to_openai_format(),
        json!({"role":"user", "content":""})
    );
    let mut assistant = RuntimeMessage::assistant("first");
    assistant.content.push(ContentPart::text("second"));
    assert_eq!(
        assistant.to_openai_format(),
        json!({"role":"assistant", "content":"first\nsecond"})
    );
}

#[test]
fn message_phase_wire_contract_preserves_optional_source() {
    for (phase, wire) in [
        (None, None),
        (Some(ExecutionPhase::Commentary), Some("commentary")),
        (Some(ExecutionPhase::FinalAnswer), Some("final_answer")),
    ] {
        for source in [
            None,
            Some(PhaseSource::Provider),
            Some(PhaseSource::Derived),
        ] {
            if phase.is_none() && source.is_some() {
                continue;
            }
            let message = match (phase, source) {
                (Some(phase), Some(source)) => {
                    RuntimeMessage::assistant("answer").with_phase_from(phase, source)
                }
                (Some(phase), None) => RuntimeMessage::assistant("answer").with_phase(phase),
                _ => RuntimeMessage::assistant("answer"),
            };
            let json = serde_json::to_value(&message).unwrap();
            assert_eq!(
                json.get("phase"),
                wire.map(serde_json::Value::from).as_ref()
            );
            let source_wire = match source {
                Some(PhaseSource::Provider) => Some("provider"),
                Some(PhaseSource::Derived) => Some("derived"),
                None => None,
            };
            assert_eq!(
                json.get("phase_source"),
                source_wire.map(serde_json::Value::from).as_ref()
            );
            let decoded: RuntimeMessage = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(decoded.phase, phase);
            assert_eq!(decoded.phase_source, source);
            assert_eq!(decoded.text(), Some("answer"));
            assert_eq!(serde_json::to_value(decoded).unwrap(), json);
        }
    }
}

#[test]
fn hints_merge_shallowly_with_message_precedence() {
    let session = std::collections::HashMap::from([
        ("shared".into(), json!({"old":1})),
        ("session_only".into(), json!(42)),
    ]);
    let message = std::collections::HashMap::from([
        ("shared".into(), json!({"new":2})),
        ("message_only".into(), json!(null)),
    ]);
    for (left, right, expected) in [
        (None, None, json!({})),
        (
            Some(&session),
            None,
            json!({"shared":{"old":1},"session_only":42}),
        ),
        (
            None,
            Some(&message),
            json!({"shared":{"new":2},"message_only":null}),
        ),
        (
            Some(&session),
            Some(&message),
            json!({"shared":{"new":2},"session_only":42,"message_only":null}),
        ),
    ] {
        assert_eq!(
            serde_json::to_value(Controls::resolve_hints(left, right)).unwrap(),
            expected
        );
    }
}

#[test]
fn controls_wire_contract_preserves_all_overrides_and_legacy_defaults() {
    let expected = json!({"model_id":"model_00000000000000000000000000000006", "locale":"uk-UA",
        "reasoning":{"effort":"high"}, "speed":"priority", "verbosity":"low", "error_disclosure":"generic",
        "hints":{"setup_connection":true,"theme":"dark"}});
    let controls = Controls {
        model_id: Some(ModelId::from_uuid(uuid::Uuid::from_u128(6))),
        locale: Some("uk-UA".into()),
        reasoning: Some(ReasoningConfig {
            effort: Some(everruns_provider::model::ReasoningEffort::High),
        }),
        speed: Some("priority".into()),
        verbosity: Some("low".into()),
        error_disclosure: Some("generic".into()),
        hints: Some(std::collections::HashMap::from([
            ("setup_connection".into(), json!(true)),
            ("theme".into(), json!("dark")),
        ])),
    };
    assert_eq!(serde_json::to_value(&controls).unwrap(), expected);
    assert_eq!(
        serde_json::from_value::<Controls>(expected).unwrap(),
        controls
    );
    let legacy: Controls = serde_json::from_value(json!({})).unwrap();
    assert_eq!(serde_json::to_value(legacy).unwrap(), json!({}));
}

#[test]
fn tool_result_text_preserves_strings_without_json_escaping() {
    let value = serde_json::json!("{\n  \"count\": 1\n}");
    assert_eq!(
        ContentPart::tool_result_text(&value).as_text(),
        Some("{\n  \"count\": 1\n}")
    );
}

#[test]
fn tool_result_text_serializes_structured_values() {
    for (value, expected) in [
        (json!({"count":1}), "{\"count\":1}"),
        (json!([true, 2]), "[true,2]"),
        (json!(null), "null"),
    ] {
        assert_eq!(
            ContentPart::tool_result_text(&value).as_text(),
            Some(expected)
        );
    }
}
#[test]
fn file_content_part_serde_roundtrip() {
    let part = ContentPart::File(FileContentPart::with_filename(FileId::new(), "report.pdf"));
    let v = serde_json::to_value(&part).unwrap();
    assert_eq!(v["type"], serde_json::json!("file"));
    assert_eq!(v["filename"], serde_json::json!("report.pdf"));
    let back: ContentPart = serde_json::from_value(v).unwrap();
    assert_eq!(back, part);
    assert!(back.is_file());
    assert_eq!(back.content_type(), ContentType::File);
    assert_eq!(ContentType::File.to_string(), "file");
}
