use super::*;
use futures::StreamExt;

impl LlmSimDriver {
    async fn chat_completion(
        &self,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<everruns_contracts::driver_registry::LlmResponse> {
        ChatDriver::chat_completion(
            self,
            &everruns_contracts::runtime_provider::ProviderEndpoint::default(),
            messages,
            config,
        )
        .await
    }

    async fn chat_completion_stream(
        &self,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        ChatDriver::chat_completion_stream(
            self,
            &everruns_contracts::runtime_provider::ProviderEndpoint::default(),
            messages,
            config,
        )
        .await
    }
}

fn make_config() -> LlmCallConfig {
    LlmCallConfig::new("test-model")
}

fn user_message(content: &str) -> Message {
    Message::text(MessageRole::User, content)
}

fn system_message(content: &str) -> Message {
    Message::text(MessageRole::System, content)
}

#[tokio::test]
async fn test_fixed_response() {
    let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello, world!"));
    let messages = vec![user_message("Hi there")];

    let response = driver
        .chat_completion(messages, &make_config())
        .await
        .unwrap();

    assert_eq!(response.text, "Hello, world!");
    assert!(response.tool_calls.is_none());
}

#[tokio::test]
async fn test_echo_response() {
    let driver = LlmSimDriver::new(LlmSimConfig::echo());
    let messages = vec![
        system_message("You are a helpful assistant"),
        user_message("What is 2+2?"),
    ];

    let response = driver
        .chat_completion(messages, &make_config())
        .await
        .unwrap();

    assert_eq!(response.text, "Echo: What is 2+2?");
}

#[tokio::test]
async fn test_sequence_response() {
    let driver = LlmSimDriver::new(LlmSimConfig::sequence(vec![
        "First".to_string(),
        "Second".to_string(),
        "Third".to_string(),
    ]));

    let messages = vec![user_message("test")];

    // First call
    let r1 = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    assert_eq!(r1.text, "First");

    // Second call
    let r2 = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    assert_eq!(r2.text, "Second");

    // Third call
    let r3 = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    assert_eq!(r3.text, "Third");

    // Fourth call - cycles back to first
    let r4 = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    assert_eq!(r4.text, "First");
}

#[tokio::test]
async fn test_lorem_response() {
    let driver = LlmSimDriver::new(LlmSimConfig::lorem(50));
    let messages = vec![user_message("Generate text")];

    let response = driver
        .chat_completion(messages, &make_config())
        .await
        .unwrap();

    // Lorem response should have content
    assert!(!response.text.is_empty());
    // Should have multiple words
    assert!(response.text.split_whitespace().count() > 5);
}

#[tokio::test]
async fn test_fixed_tool_calls() {
    let tool_call = ToolCall {
        id: "call_123".to_string(),
        name: "get_weather".to_string(),
        arguments: serde_json::json!({"city": "NYC"}),
    };

    let driver = LlmSimDriver::new(
        LlmSimConfig::fixed("Let me check the weather.").with_tool_calls(vec![tool_call.clone()]),
    );

    let messages = vec![user_message("What's the weather?")];
    let response = driver
        .chat_completion(messages, &make_config())
        .await
        .unwrap();

    assert_eq!(response.text, "Let me check the weather.");
    let calls = response.tool_calls.expect("Expected tool calls");
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "get_weather");
    assert_eq!(calls[0].id, "call_123");
}

#[tokio::test]
async fn test_tool_call_sequence() {
    let call1 = ToolCall {
        id: "call_1".to_string(),
        name: "search".to_string(),
        arguments: serde_json::json!({"q": "rust"}),
    };
    let call2 = ToolCall {
        id: "call_2".to_string(),
        name: "fetch".to_string(),
        arguments: serde_json::json!({"url": "https://example.com"}),
    };

    let driver = LlmSimDriver::new(
        LlmSimConfig::fixed("Processing...").with_tool_call_sequence(vec![
            vec![call1.clone()],
            vec![call2.clone()],
            vec![],
        ]),
    );

    let messages = vec![user_message("test")];

    // First call - should get search
    let r1 = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    let calls1 = r1.tool_calls.expect("Expected tool calls");
    assert_eq!(calls1[0].name, "search");

    // Second call - should get fetch
    let r2 = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    let calls2 = r2.tool_calls.expect("Expected tool calls");
    assert_eq!(calls2[0].name, "fetch");

    // Third call - no tool calls
    let r3 = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    assert!(r3.tool_calls.is_none());
}

#[tokio::test]
async fn shared_driver_sequences_are_independent_for_each_session() {
    let driver = LlmSimDriver::new(LlmSimConfig::sequence(vec![
        "first".into(),
        "second".into(),
    ]));
    let mut first = make_config();
    first
        .metadata
        .insert("session_id".into(), "session-one".into());
    let mut second = first.clone();
    second
        .metadata
        .insert("session_id".into(), "session-two".into());
    let messages = vec![user_message("hello")];
    assert_eq!(
        driver
            .chat_completion(messages.clone(), &first)
            .await
            .unwrap()
            .text,
        "first"
    );
    assert_eq!(
        driver
            .clone()
            .chat_completion(messages.clone(), &second)
            .await
            .unwrap()
            .text,
        "first"
    );
    assert_eq!(
        driver.chat_completion(messages, &first).await.unwrap().text,
        "second"
    );
}

#[tokio::test]
async fn test_scripted_multi_turn_tool_call_agent_sequence() {
    let driver = LlmSimDriver::new(
        LlmSimConfig::scripted(vec![
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "bash".to_string(),
                arguments: serde_json::json!({"command": "echo hello > /tmp/x.txt"}),
                id: None,
            }]),
            SimTurn::ToolCalls(vec![SimToolCall {
                name: "bash".to_string(),
                arguments: serde_json::json!({"command": "sed -i s/hello/world/ /tmp/x.txt"}),
                id: None,
            }]),
            SimTurn::Assistant("done".to_string()),
        ])
        .with_on_exhausted(OnExhausted::Error),
    );

    let messages = vec![user_message("create /tmp/x.txt then change hello to world")];

    let first = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    let first_calls = first.tool_calls.expect("first turn should call bash");
    assert_eq!(first.text, "");
    assert_eq!(first_calls[0].name, "bash");
    assert_eq!(first_calls[0].id, "call_llmsim_0_0");

    let second = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    let second_calls = second.tool_calls.expect("second turn should call bash");
    assert_eq!(second_calls[0].name, "bash");
    assert_eq!(second_calls[0].id, "call_llmsim_1_0");

    let final_response = driver
        .chat_completion(messages.clone(), &make_config())
        .await
        .unwrap();
    assert_eq!(final_response.text, "done");
    assert!(final_response.tool_calls.is_none());

    let exhausted = driver
        .chat_completion(messages, &make_config())
        .await
        .unwrap_err();
    assert!(matches!(exhausted, AgentLoopError::Configuration(_)));
}

#[tokio::test]
async fn test_scripted_mixed_turn_streams_text_and_tool_calls() {
    let driver = LlmSimDriver::new(LlmSimConfig::scripted(vec![SimTurn::Mixed {
        text: "Let me check".to_string(),
        tool_calls: vec![SimToolCall {
            name: "search".to_string(),
            arguments: serde_json::json!({"q": "rust"}),
            id: Some("call_search".to_string()),
        }],
    }]));

    let mut stream = driver
        .chat_completion_stream(vec![user_message("find rust")], &make_config())
        .await
        .unwrap();

    let mut text_parts = Vec::new();
    let mut tool_calls = None;
    while let Some(event) = stream.next().await {
        match event.unwrap() {
            LlmStreamEvent::TextDelta(text) => text_parts.push(text),
            LlmStreamEvent::ToolCalls(calls) => tool_calls = Some(calls),
            LlmStreamEvent::Done(_) => {}
            _ => {}
        }
    }

    assert!(!text_parts.is_empty(), "scripted text should stream");
    assert_eq!(text_parts.join(""), "Let me check");
    let calls = tool_calls.expect("mixed turn should emit tool calls");
    assert_eq!(calls[0].id, "call_search");
    assert_eq!(calls[0].name, "search");
}

#[tokio::test]
async fn test_scripted_on_exhausted_modes() {
    let repeat = LlmSimDriver::new(LlmSimConfig::scripted(vec![
        SimTurn::Assistant("one".to_string()),
        SimTurn::Assistant("two".to_string()),
    ]));
    let messages = vec![user_message("test")];
    assert_eq!(
        repeat
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap()
            .text,
        "one"
    );
    assert_eq!(
        repeat
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap()
            .text,
        "two"
    );
    assert_eq!(
        repeat
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap()
            .text,
        "two"
    );

    let looping = LlmSimDriver::new(
        LlmSimConfig::scripted(vec![
            SimTurn::Assistant("a".to_string()),
            SimTurn::Assistant("b".to_string()),
        ])
        .with_on_exhausted(OnExhausted::Loop),
    );
    assert_eq!(
        looping
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap()
            .text,
        "a"
    );
    assert_eq!(
        looping
            .chat_completion(messages.clone(), &make_config())
            .await
            .unwrap()
            .text,
        "b"
    );
    assert_eq!(
        looping
            .chat_completion(messages, &make_config())
            .await
            .unwrap()
            .text,
        "a"
    );
}

#[tokio::test]
async fn test_scripted_error_turn() {
    let driver = LlmSimDriver::new(LlmSimConfig::scripted(vec![SimTurn::Error(
        SimError::RateLimit,
    )]));

    let err = driver
        .chat_completion(vec![user_message("test")], &make_config())
        .await
        .unwrap_err();

    assert!(err.is_rate_limited());
}

#[tokio::test]
async fn test_conditional_tool_calls() {
    let weather_call = ToolCall {
        id: "call_w".to_string(),
        name: "get_weather".to_string(),
        arguments: serde_json::json!({}),
    };
    let search_call = ToolCall {
        id: "call_s".to_string(),
        name: "search".to_string(),
        arguments: serde_json::json!({}),
    };

    let config = LlmSimConfig {
        response: ResponseConfig::Fixed("Response".to_string()),
        tool_calls: Some(ToolCallConfig::Conditional {
            patterns: vec![
                ToolCallPattern::new("weather", vec![weather_call]),
                ToolCallPattern::new("search", vec![search_call]),
            ],
        }),
        simulate_latency: false,
        model_name: "test".to_string(),
        response_delay: None,
        response_id: None,
        effort_capture: None,
        message_capture: None,
    };

    let driver = LlmSimDriver::new(config);

    // Weather query - should trigger weather tool
    let r1 = driver
        .chat_completion(vec![user_message("What's the weather?")], &make_config())
        .await
        .unwrap();
    let calls1 = r1.tool_calls.expect("Expected weather tool");
    assert_eq!(calls1[0].name, "get_weather");

    // Search query - should trigger search tool
    let r2 = driver
        .chat_completion(vec![user_message("search for rust")], &make_config())
        .await
        .unwrap();
    let calls2 = r2.tool_calls.expect("Expected search tool");
    assert_eq!(calls2[0].name, "search");

    // No matching pattern - no tool calls
    let r3 = driver
        .chat_completion(vec![user_message("hello world")], &make_config())
        .await
        .unwrap();
    assert!(r3.tool_calls.is_none());
}

#[tokio::test]
async fn test_streaming() {
    let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hello world test"));
    let messages = vec![user_message("test")];

    let mut stream = driver
        .chat_completion_stream(messages, &make_config())
        .await
        .unwrap();

    let mut text_parts = Vec::new();
    let mut got_done = false;

    while let Some(event) = stream.next().await {
        match event.unwrap() {
            LlmStreamEvent::TextDelta(text) => text_parts.push(text),
            LlmStreamEvent::Done(meta) => {
                got_done = true;
                assert!(meta.total_tokens.is_some());
                assert!(meta.model.is_some());
            }
            _ => {}
        }
    }

    assert!(got_done);
    // llmsim's TokenStream handles chunking; verify full text is correct
    assert!(!text_parts.is_empty());
    assert_eq!(text_parts.join(""), "Hello world test");
}

#[tokio::test]
async fn test_metadata() {
    let driver = LlmSimDriver::new(LlmSimConfig::fixed("Hi").with_model("custom-model"));
    let messages = vec![user_message("test")];

    let mut config = make_config();
    config.model = "request-model".to_string();

    let response = driver.chat_completion(messages, &config).await.unwrap();

    assert_eq!(response.metadata.model, Some("request-model".to_string()));
    assert!(response.metadata.prompt_tokens.is_some());
    assert!(response.metadata.completion_tokens.is_some());
}

#[tokio::test]
async fn test_register_driver() {
    let mut registry = DriverRegistry::new();
    register_driver(&mut registry);

    assert!(registry.has_driver(&DriverId::LlmSim));

    // Creating a driver should work (with any API key since it's simulated)
    let config = everruns_contracts::driver_registry::ProviderConfig::new(DriverId::LlmSim)
        .with_api_key("fake-key");
    let driver = registry.create_chat_driver(&config);
    assert!(driver.is_ok());
}

#[test]
fn test_recommended_models_cover_all_roles() {
    let mut registry = DriverRegistry::new();
    register_driver(&mut registry);

    let entry = registry
        .recommended_models(&DriverId::LlmSim)
        .expect("llmsim declares recommended models");
    assert_eq!(entry.provider, DriverId::LlmSim.as_str());
    assert!(!entry.recommended_models.is_empty());
    for role in [
        entry.coding_model,
        entry.research_model,
        entry.chat_model,
        entry.small_model,
        entry.decision_model,
    ] {
        assert!(
            entry.recommended_models.contains(&role),
            "role default {role} not in recommended list"
        );
    }
}

#[tokio::test]
async fn test_empty_response() {
    let config = LlmSimConfig {
        response: ResponseConfig::Empty,
        tool_calls: None,
        simulate_latency: false,
        model_name: "test".to_string(),
        response_delay: None,
        response_id: None,
        effort_capture: None,
        message_capture: None,
    };

    let driver = LlmSimDriver::new(config);
    let messages = vec![user_message("test")];

    let response = driver
        .chat_completion(messages, &make_config())
        .await
        .unwrap();

    assert!(response.text.is_empty());
}

#[test]
fn test_resolve_latency_profile_from_model_name() {
    let driver = LlmSimDriver::new(LlmSimConfig::fixed("test"));

    // "-latency" in model name -> fast profile (non-instant)
    let profile = driver.resolve_latency_profile("llmsim-latency");
    assert!(profile.sample_ttft().as_nanos() > 0);

    // default model name -> instant profile
    let profile = driver.resolve_latency_profile("llmsim-default");
    assert_eq!(profile.sample_ttft().as_nanos(), 0);

    // config flag also enables fast profile
    let driver = LlmSimDriver::new(LlmSimConfig::fixed("test").with_latency());
    let profile = driver.resolve_latency_profile("llmsim-default");
    assert!(profile.sample_ttft().as_nanos() > 0);
}
