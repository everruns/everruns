use super::*;
use everruns_provider::driver_registry::ChatDriver;
use everruns_provider::{BuiltinTool, DeferrablePolicy, ToolHints, ToolPolicy};

fn contract_config(model: &str, max_tokens: Option<u32>) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.max_tokens = max_tokens;
    config
}

fn contract_tool(name: &str, deferrable: DeferrablePolicy) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: name.into(),
        display_name: None,
        description: "Search records".into(),
        parameters: json!({"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable,
        hints: ToolHints::default(),
        full_parameters: None,
    })
}

async fn assert_contract_request(config: LlmCallConfig, registered: bool, expected: Value) {
    use everruns_provider::{Provider, StaticHeaderAuth};
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("x-api-key", "synthetic-key"))
        .and(header("anthropic-version", "2023-06-01"))
        .and(body_json(expected))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"message":"request captured"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let base = format!("{}/v1", server.uri());
    let driver = if registered {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        registry
            .create_chat_driver(
                &everruns_provider::driver_registry::ProviderConfig::new(DriverId::Anthropic)
                    .with_api_key("synthetic-key")
                    .with_base_url(base),
            )
            .unwrap()
    } else {
        Provider::new("test", AnthropicChatDriver::new())
            .base_url(base)
            .auth(StaticHeaderAuth::new("x-api-key", "synthetic-key"))
            .into_boxed_driver()
    };
    let error = match driver
        .chat_completion_stream(
            &everruns_provider::ProviderEndpoint::default(),
            vec![Message::text(MessageRole::User, "hello")],
            &config,
        )
        .await
    {
        Ok(_) => panic!("expected capture response"),
        Err(error) => error,
    };
    assert_eq!(
        error.llm_error_kind(),
        Some(LlmErrorKind::InvalidRequest),
        "model={}, max_tokens={:?}, effort={:?}: {error}",
        config.model,
        config.max_tokens,
        config.reasoning_effort
    );
    assert!(error.to_string().contains("request captured"), "{error}");
    server.verify().await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].headers.contains_key("authorization"));
}

#[tokio::test]
async fn registered_and_direct_requests_apply_parallel_preferences_only_with_tools() {
    for registered in [false, true] {
        for tools in [false, true] {
            for preference in [None, Some(true), Some(false)] {
                let mut config = contract_config("claude-test", Some(32));
                config.parallel_tool_calls = preference;
                let mut expected = json!({"model":"claude-test","max_tokens":32,"stream":true,
                    "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]});
                if tools {
                    config
                        .tools
                        .push(contract_tool("lookup", DeferrablePolicy::Never));
                    expected["tools"] = json!([{"name":"lookup","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}}]);
                    if let Some(allow) = preference {
                        expected["tool_choice"] =
                            json!({"type":"auto","disable_parallel_tool_use":!allow});
                    }
                }
                assert_contract_request(config, registered, expected).await;
            }
        }
    }
}

#[test]
fn server_compaction_requires_direct_eligible_anthropic_models() {
    use everruns_provider::Provider;

    let driver = AnthropicChatDriver::new();
    let direct = Provider::new("anthropic", AnthropicChatDriver::new())
        .base_url(DEFAULT_BASE_URL)
        .endpoint()
        .clone();
    let custom = Provider::new("anthropic", AnthropicChatDriver::new())
        .base_url("https://proxy.example.test/v1")
        .endpoint()
        .clone();

    assert_eq!(
        driver.provider_managed_reduction_option(&direct, "claude-opus-4-8", 120_000),
        Some((
            SERVER_COMPACTION_OPTION.to_string(),
            json!({"trigger_tokens":72_000})
        ))
    );
    assert_eq!(
        driver.provider_managed_reduction_option(&direct, "claude-opus-4-8-20260101[1m]", 1_000),
        Some((
            SERVER_COMPACTION_OPTION.to_string(),
            json!({"trigger_tokens":SERVER_COMPACTION_MIN_TOKENS})
        ))
    );
    assert!(
        driver
            .provider_managed_reduction_option(&custom, "claude-opus-4-8", 120_000)
            .is_none()
    );
    for model in [
        "claude-opus-5-5",
        "claude-sonnet-4-5",
        "claude-haiku-4-5",
        "unlisted-model",
    ] {
        assert!(
            driver
                .provider_managed_reduction_option(&direct, model, 120_000)
                .is_none(),
            "{model}"
        );
    }
}

#[test]
fn server_compaction_rejects_a_configured_output_budget_below_the_minimum_trigger() {
    use everruns_provider::Provider;

    let driver = Provider::new("anthropic", AnthropicChatDriver::new())
        .base_url(DEFAULT_BASE_URL)
        .into_boxed_driver();
    let endpoint = everruns_provider::ProviderEndpoint::default();
    let mut config = LlmCallConfig::new("claude-opus-4-8");
    config.max_tokens = Some(160_000);

    assert_eq!(
        driver.provider_managed_reduction_fallback_reason(&endpoint, &config),
        Some("configured_output_budget")
    );

    config.max_tokens = Some(150_000);
    assert_eq!(
        driver.provider_managed_reduction_fallback_reason(&endpoint, &config),
        None
    );
}
#[test]
fn server_compaction_request_contract_uses_top_level_cache_control() {
    let request = AnthropicRequest {
        model: "claude-opus-4-8".to_string(),
        messages: vec![json!({
            "role":"user",
            "content":[{"type":"text","text":"hello"}]
        })],
        max_tokens: 32,
        temperature: None,
        system: None,
        stream: true,
        tools: None,
        tool_choice: None,
        thinking: None,
        output_config: None,
        diagnostics: None,
        context_management: Some(AnthropicContextManagement {
            edits: vec![AnthropicContextEdit::Compact {
                trigger: AnthropicCompactionTrigger {
                    r#type: "input_tokens",
                    value: 120_000usize,
                },
                pause_after_compaction: false,
            }],
        }),
        cache_control: Some(AnthropicCacheControl::ephemeral()),
    };

    let value = serde_json::to_value(request).unwrap();
    assert_eq!(
        value["context_management"],
        json!({"edits":[{
            "type":"compact_20260112",
            "trigger":{"type":"input_tokens","value":120_000},
            "pause_after_compaction":false
        }]})
    );
    assert_eq!(value["cache_control"], json!({"type":"ephemeral"}));
    assert!(
        !serde_json::to_string(&value["messages"])
            .unwrap()
            .contains("cache_control")
    );
}

#[test]
fn split_compaction_deltas_build_an_exact_replay_checkpoint() {
    let blocks = Mutex::new(BTreeMap::from([(
        0,
        json!({
            "type":"compaction",
            "content":"",
            "encrypted_content":"cipher-v1"
        }),
    )]));
    append_raw_block_field(&blocks, 0, "content", "first ");
    set_raw_block_field(
        &blocks,
        0,
        "encrypted_content",
        Value::String("cipher-v2".to_string()),
    );
    append_raw_block_field(&blocks, 0, "content", "second");
    let response = vec![
        blocks.lock().unwrap().get(&0).unwrap().clone(),
        json!({"type":"text","text":"answer","future_field":{"kept":true}}),
    ];
    let prior = vec![json!({
        "role":"user",
        "content":[{"type":"text","text":"original"}]
    })];

    let candidate = anthropic_checkpoint_candidate(true, &prior, &response)
        .unwrap()
        .expect("complete compaction should produce a checkpoint");
    assert_eq!(
        candidate.format_version,
        SERVER_COMPACTION_CHECKPOINT_FORMAT
    );
    let ProviderOpaqueContext::AnthropicMessagesPrefix { messages_json } = candidate.context else {
        panic!("expected Anthropic message-prefix checkpoint");
    };
    assert_eq!(
        serde_json::from_str::<Value>(&messages_json).unwrap(),
        json!([
            {
                "role":"user",
                "content":[{"type":"text","text":"original"}]
            },
            {
                "role":"assistant",
                "content":[
                    {
                        "type":"compaction",
                        "content":"first second",
                        "encrypted_content":"cipher-v2"
                    },
                    {
                        "type":"text",
                        "text":"answer",
                        "future_field":{"kept":true}
                    }
                ]
            }
        ])
    );
}

#[test]
fn incomplete_compaction_never_produces_a_checkpoint() {
    assert!(
        anthropic_checkpoint_candidate(
            true,
            &[],
            &[json!({"type":"text","text":"ordinary response"})]
        )
        .unwrap()
        .is_none()
    );
    let response = vec![json!({
        "type":"compaction",
        "content":"summary",
        "encrypted_content":""
    })];
    assert!(anthropic_checkpoint_candidate(true, &[], &response).is_err());
    assert!(
        anthropic_checkpoint_candidate(false, &[], &response)
            .unwrap()
            .is_none()
    );
}

#[test]
fn three_turn_messages_keep_each_prior_wire_array_as_an_exact_prefix() {
    let request_one = vec![json!({
        "role":"user",
        "content":[{"type":"text","text":"turn one"}]
    })];
    let response_one = vec![
        json!({
            "type":"compaction",
            "content":"summary",
            "encrypted_content":"ciphertext",
            "future_field":{"preserved":true}
        }),
        json!({"type":"text","text":"answer one"}),
    ];
    let checkpoint = anthropic_checkpoint_candidate(true, &request_one, &response_one)
        .unwrap()
        .unwrap();
    let ProviderOpaqueContext::AnthropicMessagesPrefix { messages_json } = checkpoint.context
    else {
        panic!("expected Anthropic prefix");
    };
    let mut request_two: Vec<Value> = serde_json::from_str(&messages_json).unwrap();
    request_two.push(json!({
        "role":"user",
        "content":[{"type":"text","text":"turn two"}]
    }));
    assert_eq!(&request_two[..request_one.len()], request_one.as_slice());

    let mut request_three = request_two.clone();
    request_three.push(json!({
        "role":"assistant",
        "content":[{"type":"text","text":"answer two","provider_field":"unchanged"}]
    }));
    request_three.push(json!({
        "role":"user",
        "content":[{"type":"text","text":"turn three"}]
    }));
    assert_eq!(&request_three[..request_two.len()], request_two.as_slice());
    assert_eq!(
        request_three[1]["content"][0],
        json!({
            "type":"compaction",
            "content":"summary",
            "encrypted_content":"ciphertext",
            "future_field":{"preserved":true}
        })
    );
}

#[tokio::test]
async fn requests_resolve_model_limits_and_complete_reasoning_policies() {
    for (model, requested, expected_limit) in [
        ("claude-sonnet-4-5-20250514", None, 64000),
        ("claude-test", None, 16384),
        ("claude-sonnet-4-5", Some(99), 99),
        ("claude-test", Some(99), 99),
    ] {
        assert_contract_request(
            contract_config(model, requested),
            false,
            json!({"model":model,"max_tokens":expected_limit,"stream":true,
            "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}),
        )
        .await;
    }
    for (effort, budget, adaptive) in [
        (ReasoningEffort::None, None, None),
        (ReasoningEffort::Minimal, Some(1024), Some("low")),
        (ReasoningEffort::Low, Some(1024), Some("low")),
        (ReasoningEffort::Medium, Some(4096), Some("medium")),
        (ReasoningEffort::High, Some(16384), Some("high")),
        (ReasoningEffort::Xhigh, Some(32768), Some("max")),
        (ReasoningEffort::Max, Some(32768), Some("max")),
    ] {
        for model in ["claude-sonnet-4-5", "claude-opus-4-8"] {
            // A one-token cap cannot accommodate thinking in either form, so the
            // driver keeps the cap and omits it. Covering only the budget-based
            // form would leave an adaptive model thinking with a one-token
            // ceiling and returning nothing.
            let mut capped = contract_config(model, Some(1));
            capped.reasoning_effort = Some(effort);
            assert_contract_request(
                capped,
                false,
                json!({"model":model,"max_tokens":1,"stream":true,
                "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]}),
            )
            .await;

            // Given a cap with room for it, thinking is configured as usual and
            // the cap is still honoured exactly rather than grown to fit.
            let mut roomy = contract_config(model, Some(64_000));
            roomy.reasoning_effort = Some(effort);
            let mut expected = json!({"model":model,"max_tokens":64_000,"stream":true,
                "messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]});
            if model == "claude-opus-4-8" {
                if let Some(level) = adaptive {
                    expected["thinking"] = json!({"type":"adaptive","display":"summarized"});
                    expected["output_config"] = json!({"effort":level});
                }
            } else if let Some(budget) = budget {
                expected["thinking"] = json!({"type":"enabled","budget_tokens":budget});
            }
            assert_contract_request(roomy, false, expected).await;
        }
    }
}

#[test]
fn cache_markers_preserve_system_text_and_target_stable_message_blocks() {
    for (text, enabled, expected) in [
        (None, true, Value::Null),
        (Some(""), true, json!("")),
        (Some("prompt"), false, json!("prompt")),
        (
            Some("prompt"),
            true,
            json!([{"type":"text","text":"prompt","cache_control":{"type":"ephemeral"}}]),
        ),
    ] {
        assert_eq!(
            serde_json::to_value(AnthropicChatDriver::system_prompt_for_request(
                text.map(str::to_owned),
                enabled
            ))
            .unwrap(),
            expected
        );
    }
    let mut first = Message::text(MessageRole::User, "");
    first.content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: "first".into(),
        },
        LlmContentPart::Text {
            text: "last".into(),
        },
        LlmContentPart::Image {
            url: "https://example.com/a.png".into(),
        },
    ]);
    let messages = [
        first,
        Message::text(MessageRole::Assistant, "reply"),
        Message::text(MessageRole::User, "question"),
        Message::text(MessageRole::Assistant, "volatile"),
    ];
    for (enabled, volatile, positions) in [
        (false, 0, vec![]),
        (true, 0, vec![(2, 0), (3, 0)]),
        (true, 1, vec![(1, 0), (2, 0)]),
        (true, 2, vec![(0, 1), (1, 0)]),
        (true, 3, vec![(0, 1)]),
        (true, 4, vec![]),
        (true, usize::MAX, vec![]),
    ] {
        let mut expected = json!([
            {"role":"user","content":[{"type":"text","text":"first"},{"type":"text","text":"last"},{"type":"image","source":{"type":"url","url":"https://example.com/a.png"}}]},
            {"role":"assistant","content":[{"type":"text","text":"reply"}]},
            {"role":"user","content":[{"type":"text","text":"question"}]},
            {"role":"assistant","content":[{"type":"text","text":"volatile"}]}
        ]);
        for (message, block) in positions {
            expected[message]["content"][block]["cache_control"] = json!({"type":"ephemeral"});
        }
        let (_, actual) = AnthropicChatDriver::convert_messages(&messages, enabled, volatile);
        assert_eq!(
            serde_json::to_value(actual).unwrap(),
            expected,
            "enabled={enabled} volatile={volatile}"
        );
    }
    let (_, empty) = AnthropicChatDriver::convert_messages(&[], true, 0);
    assert!(empty.is_empty());
}

#[test]
fn preserved_assistant_content_replays_verbatim_in_original_order() {
    let preserved = json!([
        {"type":"thinking","thinking":"first","signature":"sig-1","future_metadata":{"opaque":true}},
        {"type":"server_tool_use","id":"srv_1","name":"tool_search_tool_bm25","input":{"query":"weather"}},
        {"type":"tool_search_tool_result","tool_use_id":"srv_1","content":{"type":"tool_search_tool_result_error","error_code":"too_many_requests"}},
        {"type":"future_server_content","payload":{"must":"survive"}},
        {"type":"thinking","thinking":"second","signature":"sig-2"},
        {"type":"text","text":"Calling the selected tool."},
        {"type":"tool_use","id":"toolu_1","name":"get_weather","input":{"city":"Paris"}}
    ]);
    let mut assistant = Message::parts(
        MessageRole::Assistant,
        vec![
            LlmContentPart::ProviderOpaque(ProviderOpaqueContent::new(
                "anthropic",
                preserved.clone(),
            )),
            LlmContentPart::Text {
                text: "reconstructed text".into(),
            },
        ],
    );
    assistant.reasoning.push(
        ReasoningContentPart::opaque("anthropic")
            .with_text(ReasoningText::Plain {
                text: "reconstructed thinking".into(),
            })
            .with_signature("wrong-signature"),
    );
    assistant.tool_calls = Some(vec![ToolCall {
        id: "wrong-id".into(),
        name: "wrong-tool".into(),
        arguments: json!({"wrong":true}),
    }]);

    let (_, converted) = AnthropicChatDriver::convert_messages(&[assistant], true, 0);

    assert_eq!(
        serde_json::to_value(converted).unwrap(),
        json!([{"role":"assistant","content":preserved}])
    );
}

#[test]
fn tool_search_preserves_complete_schemas_and_cache_thresholds() {
    let tools = [
        contract_tool("automatic", DeferrablePolicy::Automatic),
        contract_tool("always", DeferrablePolicy::Always),
        contract_tool("never", DeferrablePolicy::Never),
    ];
    for enabled in [false, true] {
        assert!(AnthropicChatDriver::convert_tools(&[], enabled).is_empty());
        for threshold in [2, 3, 4] {
            let mut expected = json!([
                {"name":"automatic","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}},
                {"name":"always","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}},
                {"name":"never","description":"Search records","input_schema":{"type":"object","properties":{"q":{"type":"string"}},"required":["q"]}}
            ]);
            if threshold <= 3 {
                expected[0]["defer_loading"] = json!(true);
                expected[1]["defer_loading"] = json!(true);
                expected.as_array_mut().unwrap().insert(
                    0,
                    json!({"type":"tool_search_tool_bm25_20251119","name":"tool_search_tool_bm25"}),
                );
            } else if enabled {
                expected[2]["cache_control"] = json!({"type":"ephemeral"});
            }
            assert_eq!(
                serde_json::to_value(AnthropicChatDriver::convert_tools_with_search(
                    &tools, threshold, enabled
                ))
                .unwrap(),
                expected,
                "threshold={threshold} cache={enabled}"
            );
        }
    }
}

#[test]
fn message_conversion_folds_system_text_without_losing_the_transcript() {
    for with_system in [false, true] {
        let mut messages = vec![Message::text(MessageRole::User, "hello")];
        if with_system {
            messages.insert(0, Message::text(MessageRole::System, "first instruction"));
            messages.push(Message::text(MessageRole::System, "later summary"));
        }
        messages.push(Message::text(MessageRole::Assistant, "reply"));
        let (system, converted) = AnthropicChatDriver::convert_messages(&messages, false, 0);
        assert_eq!(
            system.as_deref(),
            with_system.then_some("first instruction\n\nlater summary")
        );
        assert_eq!(
            serde_json::to_value(converted).unwrap(),
            json!([
                {"role":"user","content":[{"type":"text","text":"hello"}]},
                {"role":"assistant","content":[{"type":"text","text":"reply"}]}
            ])
        );
    }
}

#[test]
fn tool_exchanges_preserve_identity_and_filter_only_orphan_results() {
    let mut assistant = Message::text(MessageRole::Assistant, "");
    assistant.tool_calls = Some(vec![ToolCall {
        id: "call_123".into(),
        name: "get_weather".into(),
        arguments: json!({"city":"London"}),
    }]);
    let mut valid = Message::text(MessageRole::Tool, "{\"temp\":20}");
    valid.tool_call_id = Some("call_123".into());
    let mut orphan = Message::text(MessageRole::Tool, "orphan result");
    orphan.tool_call_id = Some("trimmed_call".into());
    let missing_id = Message::text(MessageRole::Tool, "missing ID");
    let (system, converted) =
        AnthropicChatDriver::convert_messages(&[orphan, assistant, missing_id, valid], false, 0);
    assert!(system.is_none());
    assert_eq!(
        serde_json::to_value(converted).unwrap(),
        json!([
            {"role":"assistant","content":[{"type":"tool_use","id":"call_123","name":"get_weather","input":{"city":"London"}}]},
            {"role":"user","content":[{"type":"tool_result","tool_use_id":"call_123","content":"{\"temp\":20}"}]}
        ])
    );
}

#[test]
fn model_family_normalization_requires_a_nonempty_base_and_eight_ascii_digits() {
    for (input, expected) in [
        ("claude-opus-4-5-20251101", "claude-opus-4-5"),
        ("claude-sonnet-4-6-20260217", "claude-sonnet-4-6"),
        ("claude-opus-4-5", "claude-opus-4-5"),
        ("claude-sonnet-4-6", "claude-sonnet-4-6"),
        ("claudé-opus-20251101", "claudé-opus"),
        ("claudé-opus-experimental", "claudé-opus-experimental"),
        ("", ""),
        ("-20251101", "-20251101"),
        ("claude-2025110", "claude-2025110"),
        ("claude-202511011", "claude-202511011"),
        ("claude-2025x101", "claude-2025x101"),
        ("claude-２０２５１１０１", "claude-２０２５１１０１"),
    ] {
        assert_eq!(normalize_anthropic_id(input), expected, "{input}");
    }
}

#[test]
fn fragmented_tool_arguments_preserve_payloads_and_handle_incomplete_json() {
    let payload = r#"{"path":"src/main.rs","contents":"fn main() { println!(\"hello — 世界\"); }","count":1234567}"#;
    let mut call = ToolCall {
        id: "call".into(),
        name: "write_file".into(),
        arguments: json!(""),
    };
    for (offset, ch) in payload.char_indices() {
        append_tool_input_delta(&mut call, &ch.to_string());
        assert_eq!(
            call.arguments.as_str(),
            Some(&payload[..offset + ch.len_utf8()])
        );
    }
    finalize_tool_arguments(&mut call);
    assert_eq!(
        serde_json::to_value(&call).unwrap(),
        json!({
            "id":"call","name":"write_file","arguments":{"path":"src/main.rs","contents":"fn main() { println!(\"hello — 世界\"); }","count":1234567}
        })
    );
    for incomplete in ["", "{", r#"{"x":"unterminated"#] {
        call.arguments = json!(incomplete);
        finalize_tool_arguments(&mut call);
        assert_eq!(call.arguments, json!({}), "{incomplete}");
    }
}

// These tests verify that empty text blocks are filtered out to avoid
// Anthropic API error: "text content blocks must be non-empty"

#[test]
fn content_conversion_preserves_text_and_filters_only_empty_blocks() {
    for text in ["", "Hello, world!", "   ", "\n\t", "héllo 世界"] {
        let expected = if text.is_empty() {
            json!([])
        } else {
            json!([{"type":"text","text":text}])
        };
        for content in [
            MessageContent::Text(text.into()),
            MessageContent::Parts(vec![
                LlmContentPart::Text {
                    text: String::new(),
                },
                LlmContentPart::Text { text: text.into() },
                LlmContentPart::Text {
                    text: String::new(),
                },
            ]),
        ] {
            assert_eq!(
                serde_json::to_value(AnthropicChatDriver::convert_content(&content)).unwrap(),
                expected,
                "content={content:?}"
            );
        }
    }
    assert_eq!(
        serde_json::to_value(AnthropicChatDriver::convert_content(
            &MessageContent::Parts(vec![])
        ))
        .unwrap(),
        json!([])
    );
}

#[test]
fn content_conversion_preserves_order_and_complete_media_payloads() {
    let content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: String::new(),
        },
        LlmContentPart::Text {
            text: "caption".into(),
        },
        LlmContentPart::Image {
            url: "data:image/png;base64,iVBORw0KGgo=".into(),
        },
        LlmContentPart::Text {
            text: String::new(),
        },
        LlmContentPart::Image {
            url: "https://example.com/photo.jpg?size=large".into(),
        },
        LlmContentPart::Audio {
            url: "data:audio/wav;base64,AAAA".into(),
        },
        LlmContentPart::Text { text: "  ".into() },
    ]);
    assert_eq!(
        serde_json::to_value(AnthropicChatDriver::convert_content(&content)).unwrap(),
        json!([
            {"type":"text","text":"caption"},
            {"type":"image","source":{"type":"base64","media_type":"image/png","data":"iVBORw0KGgo="}},
            {"type":"image","source":{"type":"url","url":"https://example.com/photo.jpg?size=large"}},
            {"type":"text","text":"[Audio content not supported]"},
            {"type":"text","text":"  "}
        ])
    );
}

#[test]
fn file_pdf_serializes_to_document_block() {
    let content = MessageContent::Parts(vec![
        LlmContentPart::Text {
            text: "summarize".into(),
        },
        LlmContentPart::File {
            url: "data:application/pdf;base64,JVBERi0=".into(),
            filename: Some("report.pdf".into()),
        },
        LlmContentPart::File {
            url: "https://example.com/report.pdf".into(),
            filename: None,
        },
    ]);
    assert_eq!(
        serde_json::to_value(AnthropicChatDriver::convert_content(&content)).unwrap(),
        json!([
            {"type":"text","text":"summarize"},
            {"type":"document","source":{"type":"base64","media_type":"application/pdf","data":"JVBERi0="}},
            {"type":"document","source":{"type":"url","url":"https://example.com/report.pdf"}},
        ])
    );
}

#[test]
fn test_thinking_config_serialization() {
    // Adaptive must not carry budget_tokens (400 on Fable 5.x / Opus 4.8 /
    // 4.7); display:"summarized" opts back into visible thinking text,
    // which those models omit by default.
    let adaptive = serde_json::to_value(AnthropicThinking::adaptive("claude-test")).unwrap();
    assert_eq!(
        adaptive,
        json!({"type": "adaptive", "display": "summarized"})
    );

    let enabled = serde_json::to_value(AnthropicThinking::Enabled {
        budget_tokens: 4096,
    })
    .unwrap();
    assert_eq!(enabled, json!({"type": "enabled", "budget_tokens": 4096}));
}

#[test]
fn test_convert_tools_strips_top_level_composition_keywords() {
    let make_tool = |name: &str, parameters: Value| {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: "test tool".to_string(),
            parameters,
            policy: ToolPolicy::Auto,
            category: None,
            deferrable: DeferrablePolicy::default(),
            hints: ToolHints::default(),
            full_parameters: None,
        })
    };
    let tools = vec![
        make_tool(
            "top_level_one_of",
            json!({
                "type": "object",
                "properties": {
                    "target": {
                        "type": "object",
                        // Nested composition is accepted by Anthropic and
                        // must survive.
                        "oneOf": [{"required": ["id"]}]
                    }
                },
                "required": ["target"],
                "oneOf": [{"required": ["name"]}],
                "anyOf": [{"required": ["name"]}],
                "allOf": [{"required": ["name"]}]
            }),
        ),
        // Degenerate caller-supplied schema: nothing but composition.
        make_tool("bare_any_of", json!({"anyOf": [{"type": "object"}]})),
    ];

    let converted = AnthropicChatDriver::convert_tools(&tools, false);
    let json = serde_json::to_value(&converted).unwrap();

    let schema = &json[0]["input_schema"];
    assert!(schema.get("oneOf").is_none());
    assert!(schema.get("anyOf").is_none());
    assert!(schema.get("allOf").is_none());
    assert_eq!(schema["required"], json!(["target"]));
    assert_eq!(
        schema["properties"]["target"]["oneOf"],
        json!([{"required": ["id"]}])
    );

    let bare = &json[1]["input_schema"];
    assert!(bare.get("anyOf").is_none());
    assert_eq!(bare["type"], "object");
}

#[test]
fn test_tool_result_with_images_conversion() {
    // Tool result with text + image content
    let msg = Message {
        native_tool_calls: Vec::new(),
        role: MessageRole::Tool,
        content: MessageContent::Parts(vec![
            LlmContentPart::Text {
                text: "{\"status\": \"ok\"}".to_string(),
            },
            LlmContentPart::Image {
                url: "data:image/png;base64,AAAA".to_string(),
            },
        ]),
        tool_calls: None,
        tool_call_id: Some("call_img".to_string()),
        phase: None,
        reasoning: Vec::new(),
        configuration_update: None,
    };

    let assistant = Message {
        native_tool_calls: Vec::new(),
        role: MessageRole::Assistant,
        content: MessageContent::Text(String::new()),
        tool_calls: Some(vec![ToolCall {
            id: "call_img".to_string(),
            name: "capture".to_string(),
            arguments: json!({}),
        }]),
        tool_call_id: None,
        phase: None,
        reasoning: Vec::new(),
        configuration_update: None,
    };
    let (_, converted) = AnthropicChatDriver::convert_messages(&[assistant, msg], false, 0);

    assert_eq!(converted.len(), 2);
    assert_eq!(converted[1].role, "user");
    assert_eq!(converted[1].content.len(), 1);

    match &converted[1].content[0] {
        AnthropicContentBlock::ToolResult {
            tool_use_id,
            content,
            ..
        } => {
            assert_eq!(tool_use_id, "call_img");
            match content {
                AnthropicToolResultContent::Blocks(blocks) => {
                    assert_eq!(blocks.len(), 2);
                    match &blocks[0] {
                        AnthropicToolResultBlock::Text { text } => {
                            assert_eq!(text, "{\"status\": \"ok\"}");
                        }
                        _ => panic!("Expected text block"),
                    }
                    match &blocks[1] {
                        AnthropicToolResultBlock::Image { source } => match source {
                            AnthropicImageSource::Base64 { media_type, data } => {
                                assert_eq!(media_type, "image/png");
                                assert_eq!(data, "AAAA");
                            }
                            _ => panic!("Expected base64 image source"),
                        },
                        _ => panic!("Expected image block"),
                    }
                }
                _ => panic!("Expected Blocks content for multimodal tool result"),
            }
        }
        _ => panic!("Expected ToolResult block"),
    }
}

// ========================================================================
// HTTP error classification
// ========================================================================

#[tokio::test]
async fn http_errors_preserve_semantic_classification_without_retrying() {
    use everruns_provider::{Provider, StaticHeaderAuth};
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    let mut config = LlmCallConfig::new("claude-test");
    config.max_tokens = Some(32);
    for (status, message, category) in [
        (413, "Request too large", "size"),
        (
            400,
            "prompt is too long: 250000 tokens > 200000 maximum",
            "size",
        ),
        (400, "request size exceeded maximum", "size"),
        (400, "too many tokens in request", "size"),
        (401, "Invalid API key", "auth"),
        (429, "Rate limit exceeded", "rate"),
        (500, "Internal server error", "unavailable"),
        (404, "not_found_error: model: claude-test", "model"),
        (404, "Model not found", "model"),
        (404, "Endpoint not found", "invalid"),
        (400, "not_found_error: model: claude-test", "invalid"),
        (500, "prompt is too long", "unavailable"),
    ] {
        let server = MockServer::builder().start().await;
        let body = json!({"error":{"message":message}});
        Mock::given(method("POST")).and(path("/v1/messages"))
            .and(header("x-api-key","synthetic-key"))
            .and(header("anthropic-version","2023-06-01"))
            .and(body_json(json!({"model":"claude-test","max_tokens":32,"stream":true,"messages":[{"role":"user","content":[{"type":"text","text":"hello"}]}]})))
            .respond_with(ResponseTemplate::new(status).set_body_json(body.clone())).expect(1).mount(&server).await;
        let provider = Provider::new(
            "test",
            AnthropicChatDriver::new().with_retry_config(LlmRetryConfig::no_retry()),
        )
        .base_url(format!("{}/v1", server.uri()))
        .auth(StaticHeaderAuth::new("x-api-key", "synthetic-key"));
        let error = match provider
            .chat_completion_stream(vec![Message::text(MessageRole::User, "hello")], &config)
            .await
        {
            Ok(_) => panic!("expected HTTP error for {status}: {message}"),
            Err(error) => error,
        };
        match category {
            "size" => assert!(error.is_request_too_large(), "{error:?}"),
            "model" => assert_eq!(error.model_not_available_id(), Some("claude-test")),
            kind => {
                let expected = match kind {
                    "auth" => LlmErrorKind::Authentication,
                    "rate" => LlmErrorKind::RateLimited,
                    "unavailable" => LlmErrorKind::Unavailable,
                    _ => LlmErrorKind::InvalidRequest,
                };
                assert_eq!(
                    error.llm_error_kind(),
                    Some(expected),
                    "{status}: {message}"
                );
                assert!(!error.is_request_too_large());
                assert!(!error.is_model_not_available());
            }
        }
        if category != "model" {
            assert!(error.to_string().contains(message), "{error:?}");
        }
        server.verify().await;
    }
}

// ========================================================================
// Discovered profile construction tests
// ========================================================================

#[test]
fn discovered_profile_preserves_complete_catalog_metadata() {
    let info: AnthropicModelInfo = serde_json::from_value(json!({
        "id":"claude-sonnet-4-6-20260217", "display_name":"Claude Sonnet 4.6",
        "created_at":"2026-02-17T00:00:00Z", "max_input_tokens":200000,
        "max_tokens":64000
    }))
    .unwrap();
    assert_eq!(
        serde_json::to_value(info.to_discovered_profile()).unwrap(),
        json!({
            "name":"Claude Sonnet 4.6", "family":"claude-sonnet-4-6", "release_date":"2026-02-17",
            "attachment":false, "reasoning":false, "temperature":true, "tool_call":true,
            "structured_output":false, "open_weights":false,
            "limits":{"context":200000,"output":64000},
            "modalities":{"input":["text"],"output":["text"]},
            "tool_search":false, "supports_phases":false, "supports_server_compaction":false
        })
    );
}

#[test]
fn discovered_limits_do_not_wrap_and_require_both_bounds() {
    for (input, output, expected) in [
        (Some(0_u32), Some(0_u32), Some((0, 0))),
        (Some(2147483647), Some(64000), Some((2147483647, 64000))),
        (
            Some(2147483648),
            Some(u32::MAX),
            Some((2147483647, 2147483647)),
        ),
        (Some(u32::MAX), Some(1), Some((2147483647, 1))),
        (None, Some(64000), None),
        (Some(200000), None, None),
        (None, None, None),
    ] {
        let info: AnthropicModelInfo = serde_json::from_value(json!({
            "id":"claude-test", "display_name":"Test", "max_input_tokens":input, "max_tokens":output
        }))
        .unwrap();
        let profile = info.to_discovered_profile();
        assert_eq!(
            profile.limits.map(|l| (l.context, l.output)),
            expected,
            "input={input:?} output={output:?}"
        );
    }
}

#[test]
fn discovered_capabilities_and_effort_defaults_match_supported_choices() {
    for capabilities in [
        json!({"thinking":{"supported":false},"effort":{"supported":true,"high":{"supported":true}}}),
        json!({"thinking":{"supported":true},"effort":{"supported":true}}),
        json!({"thinking":{"supported":true},"effort":{"supported":true,"low":{"supported":false}}}),
    ] {
        let info: AnthropicModelInfo = serde_json::from_value(json!({
            "id":"claude-test","display_name":"Test","capabilities":capabilities
        }))
        .unwrap();
        assert!(info.to_discovered_profile().reasoning_effort.is_none());
    }
    for effort in [
        Value::Null,
        json!({"supported":false,"high":{"supported":true}}),
    ] {
        let info: AnthropicModelInfo = serde_json::from_value(json!({
            "id":"claude-test","display_name":"Test","capabilities":{
                "thinking":{"supported":true},"effort":effort
            }
        }))
        .unwrap();
        assert_eq!(
            serde_json::to_value(info.to_discovered_profile().reasoning_effort).unwrap(),
            json!({
                "values":[
                    {"value":"low","name":"Low (1K tokens)"},
                    {"value":"medium","name":"Medium (4K tokens)"},
                    {"value":"high","name":"High (16K tokens)"},
                    {"value":"xhigh","name":"Extra High (32K tokens)"}
                ],"default":"medium"
            })
        );
    }
    for (image, pdf, expected) in [
        (false, false, json!(["text"])),
        (true, false, json!(["text", "image"])),
        (false, true, json!(["text", "pdf"])),
        (true, true, json!(["text", "image", "pdf"])),
    ] {
        let info: AnthropicModelInfo = serde_json::from_value(json!({
            "id":"claude-test", "display_name":"Test", "capabilities":{
                "image_input":{"supported":image},"pdf_input":{"supported":pdf},
                "structured_outputs":{"supported":true}
            }
        }))
        .unwrap();
        let profile = info.to_discovered_profile();
        assert_eq!(profile.attachment, image || pdf);
        assert!(profile.structured_output);
        assert_eq!(
            serde_json::to_value(profile.modalities).unwrap(),
            json!({"input":expected,"output":["text"]})
        );
        assert!(!profile.reasoning);
        assert!(profile.reasoning_effort.is_none());
    }
    for adaptive in [false, true] {
        for (levels, default) in [
            (vec!["low"], "low"),
            (vec!["medium"], "medium"),
            (vec!["high"], "high"),
            (vec!["max"], "xhigh"),
            (
                vec!["low", "medium", "high", "max"],
                if adaptive { "high" } else { "medium" },
            ),
        ] {
            let mut effort = json!({"supported":true});
            for level in &levels {
                effort[*level] = json!({"supported":true});
            }
            let info: AnthropicModelInfo = serde_json::from_value(json!({
                "id":"claude-test", "display_name":"Test", "capabilities":{
                    "thinking":{"supported":true,"types":{"adaptive":{"supported":adaptive}}},
                    "effort":effort
                }
            }))
            .unwrap();
            let profile = info.to_discovered_profile();
            assert!(profile.reasoning);
            let expected:Vec<Value>=levels.iter().map(|level|json!({
                "value":if *level=="max" {"xhigh"} else {level},
                "name":match (*level,adaptive) {
                    ("low",true)=>"Low",("medium",true)=>"Medium",("high",true)=>"High",("max",true)=>"Max",
                    ("low",false)=>"Low (1K tokens)",("medium",false)=>"Medium (4K tokens)",("high",false)=>"High (16K tokens)",_=>"Extra High (32K tokens)"
                }
            })).collect();
            assert_eq!(
                serde_json::to_value(profile.reasoning_effort).unwrap(),
                json!({"values":expected,"default":default}),
                "adaptive={adaptive} levels={levels:?}"
            );
        }
    }
}
