use super::*;
use everruns_contracts::tool_types::ToolCall;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn driver() -> GeminiChatDriver {
    GeminiChatDriver {
        interactions: Some(true),
        ..GeminiChatDriver::new()
    }
}

fn endpoint(server: &MockServer) -> everruns_contracts::ProviderEndpoint {
    everruns_contracts::ProviderEndpoint::from_parts(
        format!("{}/v1beta", server.uri()),
        everruns_contracts::StaticHeaderAuth::new("x-goog-api-key", "synthetic-key"),
    )
}

fn sse(events: &[Value]) -> ResponseTemplate {
    let mut body = String::new();
    for event in events {
        body.push_str(&format!(
            "event: {}\ndata: {event}\n\n",
            event["event_type"].as_str().unwrap()
        ));
    }
    body.push_str("event: done\ndata: [DONE]\n\n");
    ResponseTemplate::new(200)
        .insert_header("content-type", "text/event-stream")
        .set_body_string(body)
}

fn weather_tool() -> ToolDefinition {
    ToolDefinition::function(
        "get_weather",
        "Weather for a city",
        json!({"type": "object", "properties": {"city": {"type": "string"}}, "additionalProperties": false}),
    )
}

/// The transcript a second turn sees: question, the model's call (with its
/// bound signature), and the tool's result.
fn tool_turn() -> Vec<Message> {
    let mut call = Message::text(MessageRole::Assistant, "");
    call.tool_calls = Some(vec![ToolCall {
        id: "call_7".into(),
        name: "get_weather".into(),
        arguments: json!({"city": "Paris"}),
    }]);
    call.reasoning = vec![
        ReasoningContentPart::opaque("google")
            .with_signature("sig-call")
            .with_bound_tool_call_id("call_7"),
    ];
    let mut result = Message::text(MessageRole::Tool, "18C, light rain");
    result.tool_call_id = Some("call_7".into());
    vec![
        Message::text(MessageRole::System, "Be terse."),
        Message::text(MessageRole::User, "Weather in Paris?"),
        call,
        result,
    ]
}

#[tokio::test]
async fn a_streamed_interaction_yields_reasoning_text_calls_and_its_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(header("x-goog-api-key", "synthetic-key"))
        .respond_with(sse(&[
            json!({"event_type":"interaction.created","interaction":{"id":"v1_abc","model":"gemini-4-hydrogen"}}),
            json!({"event_type":"step.start","index":0,"step":{"type":"thought"}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"thought_summary","content":{"type":"text","text":"Looking up "}}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"thought_summary","content":{"type":"text","text":"weather."}}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"thought_signature","signature":"sig-thought"}}),
            json!({"event_type":"step.stop","index":0}),
            json!({"event_type":"step.start","index":1,"step":{"type":"model_output"}}),
            json!({"event_type":"step.delta","index":1,"delta":{"type":"text","text":"Checking."}}),
            json!({"event_type":"step.stop","index":1}),
            json!({"event_type":"step.start","index":2,"step":{"type":"function_call","id":"call_9","name":"get_weather","arguments":{"city":"Paris"}}}),
            json!({"event_type":"step.delta","index":2,"delta":{"type":"thought_signature","signature":"sig-call"}}),
            json!({"event_type":"step.stop","index":2}),
            json!({"event_type":"interaction.completed","interaction":{"id":"v1_abc","status":"requires_action","usage":{"total_input_tokens":95,"total_cached_tokens":40,"total_output_tokens":22,"total_thought_tokens":50,"total_tool_use_tokens":0}}}),
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gemini-4-hydrogen");
    config.tools = vec![weather_tool()];
    config.reasoning_effort = Some(everruns_contracts::ReasoningEffort::Low);
    let response = driver()
        .chat_completion(
            &endpoint(&server),
            vec![Message::text(MessageRole::User, "Weather in Paris?")],
            &config,
        )
        .await
        .unwrap();

    assert_eq!(response.text, "Checking.");
    let calls = response.tool_calls.unwrap();
    assert_eq!(
        (calls[0].id.as_str(), calls[0].name.as_str()),
        ("call_9", "get_weather")
    );
    assert_eq!(calls[0].arguments, json!({"city": "Paris"}));
    let thought = response
        .reasoning
        .iter()
        .find(|r| r.bound_tool_call_id.is_none())
        .unwrap();
    assert_eq!(thought.signature.as_deref(), Some("sig-thought"));
    assert_eq!(
        thought.display_text().as_deref(),
        Some("Looking up weather.")
    );
    assert!(response.reasoning.iter().any(|r| {
        r.bound_tool_call_id.as_deref() == Some("call_9")
            && r.signature.as_deref() == Some("sig-call")
    }));
    let meta = &response.metadata;
    assert_eq!(meta.response_id.as_deref(), Some("v1_abc"));
    assert_eq!(meta.finish_reason.as_deref(), Some("stop"));
    assert_eq!(meta.cache_read_tokens, Some(40));
    assert_eq!(meta.prompt_tokens, Some(55));
    assert_eq!(meta.completion_tokens, Some(72));
    assert_eq!(meta.reasoning_tokens, Some(50));

    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["stream"], json!(true));
    assert_eq!(
        body["input"],
        json!([{"type":"user_input","content":[{"type":"text","text":"Weather in Paris?"}]}])
    );
    assert_eq!(body["tools"][0]["type"], json!("function"));
    // Gemini's schema subset has no additionalProperties.
    assert!(
        body["tools"][0]["parameters"]
            .get("additionalProperties")
            .is_none()
    );
    assert_eq!(body["generation_config"]["thinking_level"], json!("low"));
    assert_eq!(
        body["generation_config"]["thinking_summaries"],
        json!("auto")
    );
    assert!(body.get("previous_interaction_id").is_none());
}

#[tokio::test]
async fn a_continued_turn_sends_only_what_followed_the_models_answer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .respond_with(sse(&[
            json!({"event_type":"step.start","index":0,"step":{"type":"model_output"}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"text","text":"18C and raining."}}),
            json!({"event_type":"step.stop","index":0}),
            json!({"event_type":"interaction.completed","interaction":{"id":"v1_next","status":"completed","usage":{"total_input_tokens":187,"total_output_tokens":19}}}),
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gemini-4-hydrogen");
    config.tools = vec![weather_tool()];
    config.previous_response_id = Some("v1_prev".into());
    let response = driver()
        .chat_completion(&endpoint(&server), tool_turn(), &config)
        .await
        .unwrap();
    assert_eq!(response.text, "18C and raining.");
    assert_eq!(response.metadata.response_id.as_deref(), Some("v1_next"));

    let body: Value = server.received_requests().await.unwrap()[0]
        .body_json()
        .unwrap();
    assert_eq!(body["previous_interaction_id"], json!("v1_prev"));
    assert_eq!(
        body["input"],
        json!([{"type":"function_result","call_id":"call_7","name":"get_weather","result":"18C, light rain"}])
    );
    // Interaction-scoped settings travel on every turn.
    assert_eq!(body["system_instruction"], json!("Be terse."));
    assert_eq!(body["tools"][0]["name"], json!("get_weather"));
}

#[tokio::test]
async fn a_lost_stored_interaction_falls_back_to_the_whole_transcript() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(body_partial_json(
            json!({"previous_interaction_id":"v1_gone"}),
        ))
        .respond_with(ResponseTemplate::new(404).set_body_json(
            json!({"error":{"message":"Requested entity was not found.","code":"not_found"}}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(|request: &Request| {
            request
                .body_json::<Value>()
                .is_ok_and(|body| body.get("previous_interaction_id").is_none())
        })
        .respond_with(sse(&[
            json!({"event_type":"step.start","index":0,"step":{"type":"model_output"}}),
            json!({"event_type":"step.delta","index":0,"delta":{"type":"text","text":"Raining."}}),
            json!({"event_type":"step.stop","index":0}),
            json!({"event_type":"interaction.completed","interaction":{"id":"v1_fresh","status":"completed"}}),
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gemini-4-hydrogen");
    config.tools = vec![weather_tool()];
    config.previous_response_id = Some("v1_gone".into());
    let response = driver()
        .chat_completion(&endpoint(&server), tool_turn(), &config)
        .await
        .unwrap();
    assert_eq!(response.text, "Raining.");
    assert_eq!(response.metadata.response_id.as_deref(), Some("v1_fresh"));

    let requests = server.received_requests().await.unwrap();
    let full: Value = requests[1].body_json().unwrap();
    assert_eq!(
        full["input"],
        json!([
            {"type":"user_input","content":[{"type":"text","text":"Weather in Paris?"}]},
            {"type":"function_call","id":"call_7","name":"get_weather","arguments":{"city":"Paris"},"signature":"sig-call"},
            {"type":"function_result","call_id":"call_7","name":"get_weather","result":"18C, light rain"},
        ])
    );
}

#[tokio::test]
async fn a_grounded_answer_reports_searches_and_citations() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(body_partial_json(json!({"stream": false, "tools":[{"type":"google_search"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "v1_g",
            "status": "completed",
            "usage": {"total_input_tokens": 221, "total_output_tokens": 78, "grounding_tool_count": [{"type":"google_search","count":2}]},
            "steps": [
                {"type":"google_search_call","id":"c1","signature":"s","arguments":{"queries":["f1 results 2026","latest f1 winner"]}},
                {"type":"google_search_result","call_id":"c1","signature":"s","result":[]},
                {"type":"model_output","content":[{"type":"text","text":"Norris won in Brazil.","annotations":[{"type":"url_citation","start_index":0,"end_index":21,"url":"https://example.com/f1","title":"example.com"}]}]}
            ]
        })))
        .expect(1)
        .mount(&server)
        .await;

    let mut config = LlmCallConfig::new("gemini-3.8-flash");
    config.tools = vec![ToolDefinition::function(
        everruns_contracts::native_web_search::WEB_SEARCH_FUNCTION_TOOL,
        "Search the web",
        json!({"type": "object"}),
    )];
    let (key, value) =
        everruns_contracts::native_web_search::NativeWebSearch::default().to_driver_option();
    config.driver_options.insert(key, value);
    config.prefer_non_streaming = true;
    let response = driver()
        .chat_completion(
            &endpoint(&server),
            vec![Message::text(MessageRole::User, "Who won?")],
            &config,
        )
        .await
        .unwrap();
    assert_eq!(response.text, "Norris won in Brazil.");
    assert_eq!(
        response.metadata.hosted_tool_calls.get("web_search_call"),
        Some(&2)
    );
    let citation = &response.metadata.web_citations[0];
    assert_eq!(citation.url, "https://example.com/f1");
    assert_eq!(
        citation.span.as_ref().unwrap().text,
        "Norris won in Brazil."
    );
}

#[test]
fn custom_base_urls_and_long_decodes_stay_on_generate_content() {
    let auto = GeminiChatDriver::new();
    let google = everruns_contracts::ProviderEndpoint::from_parts(
        DEFAULT_BASE_URL,
        everruns_contracts::StaticHeaderAuth::new("x-goog-api-key", "k"),
    );
    let proxy = everruns_contracts::ProviderEndpoint::from_parts(
        "https://proxy.example.com/v1beta",
        everruns_contracts::StaticHeaderAuth::new("x-goog-api-key", "k"),
    );
    let config = LlmCallConfig::new("gemini-4-hydrogen");
    assert!(auto.uses_interactions(&google, &config));
    assert!(!auto.uses_interactions(&proxy, &config));
    let mut long = config.clone();
    long.max_tokens = Some(100_000);
    assert!(!auto.uses_interactions(&google, &long));
}

fn computer_config(model: &str) -> LlmCallConfig {
    let mut config = LlmCallConfig::new(model);
    config.tools = vec![
        ToolDefinition::function(
            "computer",
            "Operate a display",
            json!({"type": "object", "properties": {"action": {"type": "string", "enum": ["screenshot", "left_click"]}}}),
        ),
        weather_tool(),
    ];
    let (key, value) = everruns_contracts::native_computer::NativeComputerUse {
        display_width: 1280,
        display_height: 800,
    }
    .to_driver_option();
    config.driver_options.insert(key, value);
    config
}

/// A tiny PNG, as the computer tool returns after an action.
const PIXEL: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAAAAYAAjCB0C8AAAAASUVORK5CYII=";

#[tokio::test]
async fn gemini_computer_actions_run_as_computer_calls_and_replay_natively() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1beta/interactions"))
        .and(body_partial_json(json!({"input": [{"type": "user_input"}]})))
        .respond_with(sse(&[
            json!({"event_type":"interaction.created","interaction":{"id":"v1_cu"}}),
            json!({"event_type":"step.start","index":0,"step":{"type":"function_call","id":"cu_1","name":"click","arguments":{"x":500,"y":250,"intent":"Open the menu"}}}),
            json!({"event_type":"step.stop","index":0}),
            json!({"event_type":"interaction.completed","interaction":{"id":"v1_cu","status":"requires_action"}}),
        ]))
        .expect(1)
        .mount(&server)
        .await;

    let config = computer_config("gemini-4-hydrogen");
    let question = Message::text(MessageRole::User, "Open the menu");
    let response = driver()
        .chat_completion(&endpoint(&server), vec![question.clone()], &config)
        .await
        .unwrap();
    let calls = response.tool_calls.clone().unwrap();
    assert_eq!(calls[0].name, "computer");
    assert_eq!(calls[0].arguments["action"], "left_click");
    assert_eq!(calls[0].arguments["coordinate"], json!([640, 200]));

    let requests = server.received_requests().await.unwrap();
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    let tools = body["tools"].as_array().unwrap();
    assert!(tools.iter().all(|tool| tool["name"] != "computer"));
    assert!(tools.iter().any(|tool| tool["name"] == "get_weather"));
    assert!(tools.contains(&json!({
        "type": "computer_use",
        "environment": "desktop",
        "excluded_predefined_functions": ["key_down", "key_up"],
    })));

    // The next turn, resent whole: Gemini sees its own `click`, answered in
    // its shape.
    let mut answer = Message::text(MessageRole::Assistant, "");
    answer.tool_calls = Some(calls);
    let mut result = Message::text(MessageRole::Tool, "");
    result.content = everruns_contracts::message::MessageContent::Parts(vec![
        everruns_contracts::message::LlmContentPart::text("{\"status\":\"ok\"}"),
        everruns_contracts::message::LlmContentPart::image(format!(
            "data:image/png;base64,{PIXEL}"
        )),
    ]);
    result.tool_call_id = Some("cu_1".into());
    let (request, _) = driver()
        .interaction_request(&[question, answer, result], &config, true, None)
        .unwrap();
    let input = serde_json::to_value(&request).unwrap()["input"].clone();
    assert_eq!(
        input[1],
        json!({"type": "function_call", "id": "cu_1", "name": "click",
               "arguments": {"x": 500, "y": 250, "intent": "Open the menu"}})
    );
    assert_eq!(input[2]["name"], "click");
    assert_eq!(
        input[2]["result"][0],
        json!({"type": "text", "text": "{\"status\":\"ok\"}"})
    );
    assert_eq!(input[2]["result"][1]["type"], "image");
}

#[test]
fn stateless_interactions_replay_signed_history_and_reject_invalid_storage_options() {
    let mut config = LlmCallConfig::new("gemini-4-hydrogen");
    config
        .driver_options
        .insert("gemini/interactions".into(), json!({"store":false}));
    let (request, _) = driver()
        .interaction_request(&tool_turn(), &config, false, Some("old-id"))
        .unwrap();
    let body = serde_json::to_value(request).unwrap();
    assert_eq!(body["store"], false);
    assert!(body.get("previous_interaction_id").is_none());
    assert_eq!(body["input"].as_array().unwrap().len(), 3);
    assert!(body["input"].to_string().contains("sig-call"));
    for invalid in [
        json!({"store":"false"}),
        json!({"stroe":false}),
        Value::Null,
    ] {
        config
            .driver_options
            .insert("gemini/interactions".into(), invalid);
        assert!(
            driver()
                .interaction_request(&tool_turn(), &config, false, None)
                .is_err()
        );
    }
}

#[tokio::test]
async fn stateless_storage_policy_reaches_both_transports() {
    for stream in [false, true] {
        let server = MockServer::start().await;
        let finished = json!({"id":"not-reused", "status":"completed", "steps":[{"type":"model_output","content":[{"type":"text","text":"OK"}]}]});
        Mock::given(method("POST"))
            .and(path("/v1beta/interactions"))
            .and(body_partial_json(json!({"store":false,"stream":stream})))
            .respond_with(if stream {
                sse(&[json!({"event_type":"interaction.completed","interaction":finished})])
            } else {
                ResponseTemplate::new(200).set_body_json(finished)
            })
            .expect(1)
            .mount(&server)
            .await;
        let mut config = LlmCallConfig::new("gemini-4-hydrogen");
        config.prefer_non_streaming = !stream;
        config.previous_response_id = Some("must-not-use".into());
        config
            .driver_options
            .insert("gemini/interactions".into(), json!({"store":false}));
        let response = driver()
            .chat_completion(&endpoint(&server), tool_turn(), &config)
            .await
            .unwrap();
        assert!(response.metadata.finish_reason.is_some());
        let body: Value = server.received_requests().await.unwrap()[0]
            .body_json()
            .unwrap();
        assert!(body.get("previous_interaction_id").is_none());
        assert_eq!(body["input"].as_array().unwrap().len(), 3);
    }
}

/// Gemini documents the array form as the *multimodal* function response and
/// only Gemini 3 and later models accept it, so a text-only tool result must go
/// back as a plain string. Media results keep the content-block array.
#[test]
fn a_function_result_is_a_plain_string_unless_it_carries_media() {
    assert_eq!(
        tool_result(vec![json!({"type": "text", "text": "18C, light rain"})]),
        json!("18C, light rain")
    );
    assert_eq!(
        tool_result(vec![
            json!({"type": "text", "text": "first"}),
            json!({"type": "text", "text": "second"}),
        ]),
        json!("first\nsecond")
    );
    assert_eq!(
        tool_result(vec![
            json!({"type": "text", "text": "ran"}),
            json!({"type": "image", "data": "AA==", "mime_type": "image/png"}),
        ]),
        json!([
            {"type": "text", "text": "ran"},
            {"type": "image", "data": "AA==", "mime_type": "image/png"},
        ])
    );
}
