//! Native JSON wire contracts.
//!
//! Each Tokio test owns a dedicated mock server and closes response connections.
//! The process-wide HTTP client must not reuse a pooled mock port and a keep-alive
//! connection whose I/O task belongs to an earlier test runtime: a transport
//! reconnect can otherwise send the same POST twice. Keep exact request counts.
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
use everruns_contracts::{BearerAuth, LlmCallConfig, Message, MessageRole, Provider};
use everruns_drivers::{
    anthropic::AnthropicChatDriver, gemini::GeminiChatDriver, openai::OpenAIChatDriver,
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

#[path = "support/native_response.rs"]
mod native_response;
use native_response::response_resource;

#[tokio::test]
async fn responses_native_json_preserves_tools_reasoning_phase_usage_and_capture() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_partial_json(json!({"stream":false})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_json(response_resource("answer")),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = Provider::new("test", OpenAIChatDriver::new())
        .base_url(format!("{}/v1", server.uri()))
        .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("gpt-6.1-sol");
    config.capture_request = true;
    let answer = provider
        .chat_completion_non_streaming(vec![Message::text(MessageRole::User, "work")], &config)
        .await
        .unwrap();
    assert_eq!(answer.text, "answer");
    assert_eq!(answer.tool_calls.unwrap()[0].arguments, json!({"id":42}));
    assert!(!answer.reasoning.is_empty());
    assert_eq!(answer.metadata.response_id.as_deref(), Some("resp-native"));
    assert_eq!(answer.metadata.phase.as_deref(), Some("final_answer"));
    assert_eq!(answer.metadata.prompt_tokens, Some(70));
    assert_eq!(answer.metadata.cache_read_tokens, Some(30));
    assert_eq!(answer.metadata.completion_tokens, Some(20));
    assert_eq!(answer.metadata.request_body.unwrap()["stream"], false);
}

#[tokio::test]
async fn anthropic_native_json_preserves_signed_thinking_tool_inputs_and_cache_usage() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(path("/v1/messages")).and(body_partial_json(json!({"stream":false})))
        .respond_with(ResponseTemplate::new(200).insert_header("connection", "close").set_body_json(json!({"id":"msg-native","model":"claude-test","role":"assistant","type":"message","content":[
            {"type":"text","text":"answer"},
            {"type":"thinking","thinking":"checked","signature":"signed"},
            {"type":"tool_use","id":"call-native","name":"lookup","input":{"id":18446744073709551615_u64}}
        ],"stop_reason":"tool_use","stop_sequence":null,"usage":{"input_tokens":70,"output_tokens":20,"cache_read_input_tokens":30,"cache_creation_input_tokens":10}})))
        .expect(1).mount(&server).await;
    let provider = Provider::new("test", AnthropicChatDriver::new())
        .base_url(format!("{}/v1", server.uri()))
        .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("claude-test");
    config.max_tokens = Some(1024);
    config.capture_request = true;
    let answer = provider
        .chat_completion_non_streaming(vec![Message::text(MessageRole::User, "work")], &config)
        .await
        .unwrap();
    assert_eq!(answer.text, "answer");
    assert_eq!(
        answer.tool_calls.unwrap()[0].arguments,
        json!({"id":18446744073709551615_u64})
    );
    assert!(format!("{:?}", answer.reasoning).contains("signed"));
    assert_eq!(answer.metadata.prompt_tokens, Some(70));
    assert_eq!(answer.metadata.cache_read_tokens, Some(30));
    assert_eq!(answer.metadata.cache_creation_tokens, Some(10));
    assert_eq!(answer.metadata.completion_tokens, Some(20));
    assert_eq!(answer.metadata.request_body.unwrap()["stream"], false);
}

#[tokio::test]
async fn gemini_native_json_preserves_signed_parts_tools_and_usage() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(path("/v1beta/models/gemini-test:generateContent"))
        .respond_with(ResponseTemplate::new(200).insert_header("connection", "close").set_body_json(json!({"modelVersion":"gemini-test","candidates":[{"content":{"role":"model","parts":[{"text":"answer"},{"functionCall":{"name":"lookup","args":{"id":42}},"thoughtSignature":"signed"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":20,"cachedContentTokenCount":30,"thoughtsTokenCount":5}})))
        .expect(1).mount(&server).await;
    let provider = Provider::new("test", GeminiChatDriver::new())
        .base_url(format!("{}/v1beta", server.uri()))
        .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("gemini-test");
    config.capture_request = true;
    let answer = provider
        .chat_completion_non_streaming(vec![Message::text(MessageRole::User, "work")], &config)
        .await
        .unwrap();
    assert_eq!(answer.text, "answer");
    assert_eq!(answer.tool_calls.unwrap()[0].arguments, json!({"id":42}));
    assert!(format!("{:?}", answer.reasoning).contains("signed"));
    assert_eq!(answer.metadata.prompt_tokens, Some(70));
    assert_eq!(answer.metadata.cache_read_tokens, Some(30));
    assert_eq!(answer.metadata.completion_tokens, Some(25));
}

#[tokio::test]
async fn malformed_native_json_is_an_error_without_replaying_the_post() {
    for body in ["{", "{}", "{\"status\":\"in_progress\",\"output\":[]}"] {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("connection", "close")
                    .set_body_string(body),
            )
            .expect(1)
            .mount(&server)
            .await;
        let provider = Provider::new("test", OpenAIChatDriver::new())
            .base_url(server.uri())
            .auth(BearerAuth::new("synthetic-key"));
        assert!(
            provider
                .chat_completion_non_streaming(vec![], &LlmCallConfig::new("gpt-6.1-sol"))
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn long_anthropic_and_gemini_decodes_keep_streaming_transport() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(path("/messages")).and(body_partial_json(json!({"stream":true})))
        .respond_with(ResponseTemplate::new(200).insert_header("connection", "close").insert_header("content-type","text/event-stream").set_body_string("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":0}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"))
        .expect(1).mount(&server).await;
    let provider = Provider::new("test", AnthropicChatDriver::new())
        .base_url(server.uri())
        .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("claude-test");
    config.max_tokens = Some(32_768);
    provider
        .chat_completion_non_streaming(vec![Message::text(MessageRole::User, "work")], &config)
        .await
        .unwrap();
    Mock::given(method("POST")).and(path("/models/gemini-4-argon:streamGenerateContent"))
        .respond_with(ResponseTemplate::new(200).insert_header("connection", "close").insert_header("content-type","text/event-stream").set_body_string("data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"done\"}]},\"finishReason\":\"STOP\"}]}\n\n"))
        .expect(1).mount(&server).await;
    let provider = Provider::new("test", GeminiChatDriver::new())
        .base_url(server.uri())
        .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("gemini-4-argon");
    config.max_tokens = Some(131_072);
    assert_eq!(
        provider
            .chat_completion_non_streaming(vec![], &config)
            .await
            .unwrap()
            .text,
        "done"
    );
}

#[tokio::test]
async fn bounded_responses_calls_keep_the_incremental_limit_enforcement() {
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(body_partial_json(json!({"stream":true})))
        .respond_with(ResponseTemplate::new(200).insert_header("connection", "close").insert_header("content-type","text/event-stream").set_body_string("data: {\"type\":\"response.output_text.delta\",\"delta\":\"done\"}\n\ndata: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-limit\",\"status\":\"completed\",\"output\":[]}}\n\n"))
        .expect(1).mount(&server).await;
    let provider = Provider::new("test", OpenAIChatDriver::new())
        .base_url(server.uri())
        .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("gpt-6.1-sol");
    config.limits.max_response_bytes = Some(100);
    assert_eq!(
        provider
            .chat_completion_non_streaming(vec![], &config)
            .await
            .unwrap()
            .text,
        "done"
    );
}

#[tokio::test]
async fn responses_native_json_drops_incomplete_calls_and_surfaces_failures() {
    for failed in [false, true] {
        let server = MockServer::builder().start().await;
        let mut resource = response_resource("partial");
        resource["output"][2]["status"] = json!("incomplete");
        resource["status"] = json!(if failed { "failed" } else { "incomplete" });
        resource["incomplete_details"] = json!({"reason":"max_output_tokens"});
        resource["error"] = json!({"code":"server_error","message":"failed"});
        Mock::given(method("POST"))
            .and(body_partial_json(json!({"stream":false})))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("connection", "close")
                    .set_body_json(resource),
            )
            .expect(1)
            .mount(&server)
            .await;
        let provider = Provider::new("test", OpenAIChatDriver::new())
            .base_url(server.uri())
            .auth(BearerAuth::new("synthetic-key"));
        let response = provider
            .chat_completion_non_streaming(vec![], &LlmCallConfig::new("gpt-6.1-sol"))
            .await;
        if failed {
            assert!(response.is_err());
        } else {
            let response = response.unwrap();
            assert!(response.tool_calls.is_none_or(|calls| calls.is_empty()));
            assert_eq!(response.metadata.tool_calls_dropped, 1);
            assert_eq!(response.metadata.finish_reason.as_deref(), Some("length"));
        }
    }
}

#[tokio::test]
async fn total_deadline_uses_native_transport_and_bounds_the_whole_response() {
    use std::time::Duration;
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(body_partial_json(json!({"stream":false})))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_json(response_resource("late"))
                .set_delay(Duration::from_millis(500)),
        )
        .expect(1)
        .mount(&server)
        .await;
    let provider = Provider::new("test", OpenAIChatDriver::new())
        .base_url(server.uri())
        .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("gpt-6.1-sol");
    config.limits.total = Some(Duration::from_millis(100));
    assert!(
        provider
            .chat_completion_non_streaming(vec![], &config)
            .await
            .unwrap_err()
            .to_string()
            .contains("did not finish")
    );
}

#[tokio::test]
async fn background_responses_keep_resumable_streaming_when_completion_is_requested() {
    use everruns_contracts::OpenResponsesProtocolChatDriver;
    let server = MockServer::builder().start().await;
    Mock::given(method("POST")).and(body_partial_json(json!({"stream":true,"background":true,"store":true})))
        .respond_with(ResponseTemplate::new(200).insert_header("connection", "close").insert_header("content-type","text/event-stream").set_body_string("data: {\"type\":\"response.completed\",\"response\":{\"id\":\"resp-background\",\"status\":\"completed\",\"output\":[]}}\n\n"))
        .expect(1).mount(&server).await;
    let provider = Provider::new(
        "test",
        OpenResponsesProtocolChatDriver::new().with_background_mode(true),
    )
    .base_url(server.uri())
    .auth(BearerAuth::new("synthetic-key"));
    let mut config = LlmCallConfig::new("model");
    config
        .driver_options
        .insert("openai/background".into(), json!(true));
    let response = provider
        .chat_completion_non_streaming(vec![], &config)
        .await
        .unwrap();
    assert_eq!(
        response.metadata.response_id.as_deref(),
        Some("resp-background")
    );
}

#[tokio::test]
async fn chat_completions_native_keeps_local_validation_and_drops_orphaned_results() {
    use everruns_contracts::OpenAIProtocolChatDriver;
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .and(path("/v1/chat/completions"))
        .and(body_partial_json(json!({"stream":false,"messages":[{"role":"user","content":"work"}]})))
        .respond_with(ResponseTemplate::new(200).insert_header("connection", "close").set_body_json(json!({
            "model":"gpt-6.1-sol","choices":[{"message":{"role":"assistant","content":"answer"},"finish_reason":"stop"}],
            "usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}
        })))
        .expect(1).mount(&server).await;
    let provider = Provider::new("test", OpenAIProtocolChatDriver::new())
        .base_url(format!("{}/v1", server.uri()));
    let mut config = LlmCallConfig::new("gpt-6.1-sol");
    config.speed = Some("unsupported-tier".into());
    let error = provider
        .chat_completion_non_streaming(vec![Message::text(MessageRole::User, "work")], &config)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("unsupported"));
    assert!(server.received_requests().await.unwrap().is_empty());
    config.speed = None;
    let mut orphan = Message::text(MessageRole::Tool, "orphaned result");
    orphan.tool_call_id = Some("missing".into());
    let answer = provider
        .chat_completion_non_streaming(
            vec![orphan, Message::text(MessageRole::User, "work")],
            &config,
        )
        .await
        .unwrap();
    assert_eq!(answer.text, "answer");
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn malformed_native_bodies_are_not_safe_to_retry() {
    use everruns_contracts::{OpenAIProtocolChatDriver, llm_error::LlmErrorKind};
    let server = MockServer::builder().start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_string("{"),
        )
        .expect(4)
        .mount(&server)
        .await;
    let drivers: Vec<Box<dyn everruns_contracts::ChatDriver>> = vec![
        Box::new(OpenAIChatDriver::new()),
        Box::new(OpenAIProtocolChatDriver::new()),
        Box::new(AnthropicChatDriver::new()),
        Box::new(GeminiChatDriver::new()),
    ];
    for driver in drivers {
        let endpoint =
            everruns_contracts::Provider::new("native-malformed", driver).base_url(server.uri());
        let mut config = LlmCallConfig::new("test-model");
        config.max_tokens = Some(1024);
        let error = endpoint
            .chat_completion_non_streaming(vec![], &config)
            .await
            .unwrap_err();
        assert_eq!(
            error.llm_error_kind(),
            Some(LlmErrorKind::MalformedResponse)
        );
        assert!(!error.is_transient_llm_error());
    }
}
