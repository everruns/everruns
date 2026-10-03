use super::*;
use serde_json::json;

#[test]
fn input_constructors_emit_complete_literal_payloads() {
    for (item, expected) in [
        (
            InputItem::Message(MessageItem::user("Hello")),
            json!({"type":"message","role":"user","content":"Hello"}),
        ),
        (
            InputItem::Message(MessageItem::developer("Rules")),
            json!({"type":"message","role":"developer","content":"Rules"}),
        ),
        (
            InputItem::Message(MessageItem::assistant("Answer")),
            json!({"type":"message","role":"assistant","content":"Answer"}),
        ),
        (
            InputItem::FunctionCall(FunctionCallItem::new(
                "call_123",
                "get_weather",
                r#"{"location":"NYC"}"#,
            )),
            json!({"type":"function_call","call_id":"call_123","name":"get_weather","arguments":"{\"location\":\"NYC\"}"}),
        ),
        (
            InputItem::FunctionCallOutput(FunctionCallOutputItem::new(
                "call_123",
                r#"{"temp": 72}"#,
            )),
            json!({"type":"function_call_output","call_id":"call_123","output":"{\"temp\": 72}"}),
        ),
    ] {
        assert_eq!(serde_json::to_value(item).unwrap(), expected);
    }
}

#[test]
fn request_items_use_discriminators_and_preserve_complete_payloads() {
    let items = json!([
        {"type":"message","id":"msg","status":"completed","role":"assistant","content":[{"type":"output_text","text":"Answer"}]},
        {"type":"function_call","id":"fc","status":"in_progress","call_id":"call","name":"weather","arguments":"{\"city\":\"NYC\"}"},
        {"type":"function_call_output","id":"out","status":"completed","call_id":"call","output":[{"type":"input_text","text":"72"}]},
        {"type":"reasoning","id":"rs","summary":[{"type":"summary_text","text":"Summary"}],"content":[{"type":"reasoning_text","text":"Reasoning"}],"encrypted_content":"opaque"},
        {"type":"item_reference","id":"msg_123"}
    ]);
    let request = json!({"model":"model","input":items});
    let parsed: CreateResponseBody = serde_json::from_value(request.clone()).unwrap();
    let Some(Input::Items(items)) = &parsed.input else {
        panic!("expected item array")
    };
    assert!(matches!(
        items.as_slice(),
        [
            InputItem::Message(_),
            InputItem::FunctionCall(_),
            InputItem::FunctionCallOutput(_),
            InputItem::Reasoning(_),
            InputItem::Reference(_)
        ]
    ));
    assert_eq!(serde_json::to_value(parsed).unwrap(), request);
    for invalid in [
        json!({"type":"function_call","id":"fc"}),
        json!({"type":"unknown","id":"id"}),
        json!({"id":"id"}),
        json!({"type":"message","id":"msg","content":"missing role"}),
    ] {
        assert!(
            serde_json::from_value::<InputItem>(invalid.clone()).is_err(),
            "{invalid}"
        );
    }
    let simple = json!({"input":"Hello"});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<CreateResponseBody>(simple.clone()).unwrap())
            .unwrap(),
        simple
    );
}

#[test]
fn lifecycle_statuses_decode_from_response_snapshots_and_emit_protocol_values() {
    for (wire, expected) in [
        ("in_progress", ResponseStatus::InProgress),
        ("completed", ResponseStatus::Completed),
        ("failed", ResponseStatus::Failed),
        ("cancelled", ResponseStatus::Cancelled),
        ("queued", ResponseStatus::Queued),
        ("incomplete", ResponseStatus::Incomplete),
        ("provider_extension", ResponseStatus::Unknown),
    ] {
        let value = json!({"type":"response.in_progress","sequence_number":7,"response":{"id":"resp","object":"response","created_at":1,"model":"model","status":wire,"output":[],"tools":[]}});
        let event: StreamingEvent = serde_json::from_value(value.clone()).unwrap();
        let StreamingEvent::ResponseInProgress {
            sequence_number,
            response,
        } = &event
        else {
            panic!("wrong event")
        };
        assert_eq!(*sequence_number, 7);
        assert_eq!(response.status, expected);
        let mut encoded = value;
        if expected == ResponseStatus::Unknown {
            encoded["response"]["status"] = json!("unknown");
        }
        assert_eq!(serde_json::to_value(event).unwrap(), encoded);
    }
}

#[test]
fn streaming_deltas_preserve_routing_indexes_text_and_optional_fields() {
    let logprobs = json!([{"token":"hi","logprob":-0.25,"bytes":[104,105],"top_logprobs":[{"token":"hey","logprob":-0.5,"bytes":[104,101,121]}]}]);
    for wire in [
        json!({"type":"response.output_text.delta","sequence_number":5,"item_id":"msg_123","output_index":2,"content_index":3,"delta":"Hello","logprobs":logprobs,"obfuscation":"opaque"}),
        json!({"type":"response.reasoning_text.delta","sequence_number":3,"item_id":"rs_tmp","output_index":4,"content_index":2,"delta":"User asks"}),
    ] {
        let parsed: StreamingEvent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(parsed).unwrap(), wire);
    }
    let minimal: StreamingEvent =
        serde_json::from_value(json!({"type":"response.reasoning_text.delta","delta":"text"}))
            .unwrap();
    assert_eq!(
        serde_json::to_value(minimal).unwrap(),
        json!({"type":"response.reasoning_text.delta","sequence_number":0,"item_id":"","output_index":0,"content_index":0,"delta":"text"})
    );
    let no_logprobs: StreamingEvent = serde_json::from_value(json!({"type":"response.output_text.delta","sequence_number":5,"item_id":"msg","output_index":0,"content_index":0,"delta":"text"})).unwrap();
    assert_eq!(
        serde_json::to_value(no_logprobs).unwrap(),
        json!({"type":"response.output_text.delta","sequence_number":5,"item_id":"msg","output_index":0,"content_index":0,"delta":"text","logprobs":[]})
    );
}

#[test]
fn errors_preserve_full_payload_and_omit_absent_options() {
    let error = ErrorPayload {
        type_: "invalid_request_error".into(),
        code: Some("model_not_found".into()),
        message: "Model not found".into(),
        param: Some("model".into()),
        headers: Some(HashMap::from([("retry-after".into(), "5".into())])),
    };
    let expected = json!({"type":"invalid_request_error","code":"model_not_found","message":"Model not found","param":"model","headers":{"retry-after":"5"}});
    assert_eq!(serde_json::to_value(error).unwrap(), expected);
    let wire = json!({"type":"error","sequence_number":8,"error":expected});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<StreamingEvent>(wire.clone()).unwrap())
            .unwrap(),
        wire
    );
    let minimal = json!({"type":"server_error","message":"Failed"});
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<ErrorPayload>(minimal.clone()).unwrap())
            .unwrap(),
        minimal
    );
    assert_eq!(
        Error {
            code: "failed".into(),
            message: "Problem".into()
        }
        .to_string(),
        "failed: Problem"
    );
}

#[test]
fn function_names_enforce_literal_length_and_ascii_boundaries() {
    for name in [
        "a".into(),
        "a".repeat(64),
        "get_weather".into(),
        "get-weather".into(),
        "getWeather123".into(),
        "_-09AZaz".into(),
    ] {
        assert!(validate_function_name(&name), "{name}");
    }
    for name in [
        "".into(),
        "a".repeat(65),
        "get weather".into(),
        "get.weather".into(),
        "é".into(),
        "a\n".into(),
        "a/b".into(),
    ] {
        assert!(!validate_function_name(&name), "{name:?}");
    }
}

#[test]
fn metadata_enforces_pair_and_unicode_character_boundaries() {
    for count in [0, 1, 16, 17] {
        let values = (0..count)
            .map(|i| (format!("key{i}"), "value".into()))
            .collect();
        assert_eq!(validate_metadata(&values), count <= 16, "{count}");
    }
    for unit in ["a", "é", "🦀"] {
        for (key_len, value_len, valid) in [
            (0, 0, true),
            (64, 512, true),
            (65, 512, false),
            (64, 513, false),
        ] {
            assert_eq!(
                validate_metadata(&HashMap::from([(
                    unit.repeat(key_len),
                    unit.repeat(value_len)
                )])),
                valid,
                "{unit}: {key_len}/{value_len}"
            );
        }
    }
}

#[test]
fn content_variants_preserve_full_multimodal_payloads() {
    for wire in [
        json!({"type":"input_text","text":"Hello"}),
        json!({"type":"input_image","image_url":"data:image/png;base64,aGk=","detail":"high"}),
        json!({"type":"input_image"}),
        json!({"type":"input_audio","input_audio":{"data":"aGk=","format":"wav"}}),
        json!({"type":"input_file","filename":"note.txt","file_data":"aGk=","file_url":"https://example.com/file"}),
        json!({"type":"input_video","video_url":"https://example.com/video"}),
        json!({"type":"output_text","text":"answer","annotations":[{"type":"url_citation","url":"https://example.com","title":"Source","start_index":0,"end_index":6}],"logprobs":[{"token":"answer","logprob":-0.5,"bytes":[97],"top_logprobs":[]}]}),
        json!({"type":"text","text":"plain"}),
        json!({"type":"reasoning_text","text":"thought"}),
        json!({"type":"summary_text","text":"summary"}),
        json!({"type":"refusal","refusal":"Cannot"}),
    ] {
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<ContentPart>(wire.clone()).unwrap())
                .unwrap(),
            wire
        );
    }
    assert_eq!(
        serde_json::to_value(InputTextContent::new("Hello")).unwrap(),
        json!({"type":"input_text","text":"Hello"})
    );
    assert_eq!(
        serde_json::to_value(InputImageContent::new("https://example.com/image")).unwrap(),
        json!({"type":"input_image","image_url":"https://example.com/image"})
    );
}

#[test]
fn function_tools_and_choices_preserve_schema_strictness_and_omission() {
    let schema = json!({"type":"object","properties":{"city":{"type":"string"}},"required":["city"],"additionalProperties":false});
    let tool = Tool::Function {
        name: "get_weather".into(),
        description: Some("Get weather".into()),
        parameters: Some(schema.clone()),
        strict: Some(true),
    };
    let wire = json!({"type":"function","name":"get_weather","description":"Get weather","parameters":schema,"strict":true});
    assert_eq!(serde_json::to_value(tool).unwrap(), wire);
    assert_eq!(
        serde_json::to_value(serde_json::from_value::<Tool>(wire.clone()).unwrap()).unwrap(),
        wire
    );
    let built = FunctionTool::new("get_weather")
        .with_description("Get weather")
        .with_parameters(schema);
    let mut expected = wire;
    expected.as_object_mut().unwrap().remove("strict");
    assert_eq!(serde_json::to_value(built).unwrap(), expected);
    assert_eq!(
        serde_json::to_value(FunctionTool::new("minimal")).unwrap(),
        json!({"type":"function","name":"minimal"})
    );
    for wire in [
        json!("auto"),
        json!("none"),
        json!("required"),
        json!({"type":"function","name":"get_weather"}),
        json!({"type":"allowed_tools","mode":"required","tools":[{"type":"function","name":"get_weather"}]}),
    ] {
        assert_eq!(
            serde_json::to_value(serde_json::from_value::<ToolChoice>(wire.clone()).unwrap())
                .unwrap(),
            wire
        );
    }
}

#[test]
fn usage_preserves_complete_counts_details_and_optional_gateway_cost() {
    for wire in [
        json!({"input_tokens":194,"output_tokens":2,"total_tokens":196,"cost":0.00095,"input_tokens_details":{"cached_tokens":180},"output_tokens_details":{"reasoning_tokens":1}}),
        json!({"input_tokens":10,"output_tokens":5,"total_tokens":15}),
        json!({"input_tokens":0,"output_tokens":0,"total_tokens":0,"cost":0.0}),
    ] {
        let usage: Usage = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(usage.cost, wire.get("cost").and_then(Value::as_f64));
        assert_eq!(serde_json::to_value(usage).unwrap(), wire);
    }
}
#[test]
fn hosted_tool_search_preserves_complete_namespace_and_call_payloads() {
    let json = r#"{
            "id": "resp_123",
            "object": "response",
            "created_at": 1780000000,
            "status": "completed",
            "model": "gpt-5.5",
            "output": [
                {
                    "type": "tool_search_call",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "arguments": { "paths": ["Math"] }
                },
                {
                    "type": "tool_search_output",
                    "execution": "server",
                    "call_id": null,
                    "status": "completed",
                    "tools": [
                        {
                            "type": "namespace",
                            "name": "Math",
                            "description": "Tools for Math",
                            "tools": [
                                {
                                    "type": "function",
                                    "name": "add",
                                    "description": "Add numbers.",
                                    "defer_loading": true,
                                    "parameters": {
                                        "type": "object",
                                        "properties": {
                                            "a": { "type": "number" },
                                            "b": { "type": "number" }
                                        },
                                        "required": ["a", "b"],
                                        "additionalProperties": false
                                    }
                                }
                            ]
                        }
                    ]
                },
                {
                    "type": "function_call",
                    "id": "fc_123",
                    "call_id": "call_123",
                    "name": "add",
                    "namespace": "Math",
                    "arguments": "{\"a\":7,\"b\":3}",
                    "status": "completed"
                }
            ],
            "usage": {
                "input_tokens": 10,
                "output_tokens": 5,
                "total_tokens": 15
            }
        }"#;

    let response: ResponseResource = serde_json::from_str(json).unwrap();
    let mut expected: Value = serde_json::from_str(json).unwrap();
    expected["output"][0]
        .as_object_mut()
        .unwrap()
        .remove("call_id");
    expected["output"][1]
        .as_object_mut()
        .unwrap()
        .remove("call_id");
    expected["tools"] = json!([]);
    assert_eq!(serde_json::to_value(response).unwrap(), expected);
}
