#![cfg(feature = "http")]
#![allow(clippy::unwrap_used, clippy::expect_used)]

use async_trait::async_trait;
use bytes::Bytes;
use everruns_contracts::driver_registry::{
    LlmCallConfig, Message, MessageRole, PromptCacheConfig, PromptCacheRetention,
    PromptCacheStrategy,
};
use everruns_contracts::http_transport::{
    HeaderMap, HttpError, HttpRequest, HttpResponse, HttpTransport, StatusCode,
};
use everruns_contracts::{OpenResponsesProtocolChatDriver, Provider};
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

// Exercises real encoding and both decoding paths without network or paid calls.
struct Capture(Mutex<Vec<Value>>);
#[async_trait]
impl HttpTransport for Capture {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        let body: Value = serde_json::from_slice(request.body()).unwrap();
        let streaming = body["stream"] == true;
        self.0.lock().unwrap().push(body);
        let response = json!({"id":"resp_cache", "object":"response", "created_at":1780000000,
            "status":"completed", "model":"gpt-6.1-sol", "output":[],
            "usage":{"input_tokens":12000,"output_tokens":0,"total_tokens":12000,
                "input_tokens_details":{"cached_tokens":4000,"cache_write_tokens":7000,
                    "cache_write_tokens_details":{"ttl_30m_tokens":3000,"ttl_12h_tokens":4000}}}});
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            if streaming {
                "text/event-stream"
            } else {
                "application/json"
            }
            .parse()
            .unwrap(),
        );
        let bytes = if streaming {
            format!(
                "data: {}\n\n",
                json!({"type":"response.completed","sequence_number":1,"response":response})
            )
        } else {
            response.to_string()
        };
        Ok(HttpResponse::new(
            StatusCode::OK,
            headers,
            Box::pin(futures::stream::once(async move { Ok(Bytes::from(bytes)) })),
        ))
    }
}

#[tokio::test]
async fn twelve_hour_retention_round_trips_through_qualified_responses_endpoint() {
    for base in [
        "https://api.openai.com/v1",
        "https://account.openai.azure.com/openai/v1",
        "https://gateway.example/v1",
    ] {
        for non_streaming in [false, true] {
            for strategy in [PromptCacheStrategy::Auto, PromptCacheStrategy::Explicit] {
                for retention in [
                    PromptCacheRetention::Default,
                    PromptCacheRetention::TwelveHours,
                    PromptCacheRetention::ThirtyMinutes,
                ] {
                    let capture = Arc::new(Capture(Mutex::new(Vec::new())));
                    let driver = OpenResponsesProtocolChatDriver::new()
                        .with_prompt_cache_options(true)
                        .with_transport(capture.clone());
                    let provider = Provider::new("test", driver).base_url(base);
                    let mut config = LlmCallConfig::new("gpt-6.1-sol");
                    config.prefer_non_streaming = non_streaming;
                    config.prompt_cache = Some(PromptCacheConfig {
                        enabled: true,
                        retention,
                        strategy,
                        ..Default::default()
                    });
                    let result = provider
                        .chat_completion(
                            vec![
                                Message::text(MessageRole::System, "Stable policy"),
                                Message::text(MessageRole::User, "Question"),
                            ],
                            &config,
                        )
                        .await
                        .unwrap();
                    assert_eq!(result.metadata.prompt_tokens, Some(1000));
                    assert_eq!(result.metadata.cache_read_tokens, Some(4000));
                    assert_eq!(result.metadata.cache_creation_tokens, Some(7000));
                    assert_eq!(result.metadata.cache_creation_12h_tokens, Some(4000));
                    assert_eq!(result.metadata.total_tokens, Some(12000));
                    let requests = capture.0.lock().unwrap();
                    assert_eq!(requests.len(), 1);
                    assert_eq!(
                        requests[0]["prompt_cache_options"]["ttl"],
                        if base == "https://api.openai.com/v1"
                            && retention != PromptCacheRetention::ThirtyMinutes
                        {
                            "12h"
                        } else {
                            "30m"
                        }
                    );
                    assert_eq!(requests[0]["stream"], !non_streaming);
                    if strategy == PromptCacheStrategy::Explicit {
                        assert_eq!(
                            requests[0]["input"][0]["content"][0]["prompt_cache_breakpoint"]["mode"],
                            "explicit"
                        );
                    }
                }
            }
        }
    }
}

#[tokio::test]
async fn automatic_defaults_and_disable_are_endpoint_and_model_scoped() {
    for (base, model, native, expected) in [
        ("https://api.openai.com/v1", "gpt-6.1-sol", true, true),
        ("https://api.openai.com/v1", "gpt-5.6-sol", true, true),
        ("https://api.openai.com/v1", "gpt-5.5", true, false),
        ("https://api.openai.com/v1", "gpt-unknown", true, false),
        ("https://api.openai.com/v1", "gpt-6.1-sol", false, false),
        (
            "https://account.openai.azure.com/openai/v1",
            "gpt-6.1-sol",
            true,
            false,
        ),
        (
            "https://api.openai.com.example/v1",
            "gpt-6.1-sol",
            true,
            false,
        ),
        (
            "https://chatgpt.com/backend-api/codex",
            "gpt-6.1-sol",
            true,
            false,
        ),
    ] {
        for disabled in [false, true] {
            for non_streaming in [false, true] {
                let capture = Arc::new(Capture(Mutex::new(Vec::new())));
                let provider = Provider::new(
                    "test",
                    OpenResponsesProtocolChatDriver::new()
                        .with_prompt_cache_options(native)
                        .with_transport(capture.clone()),
                )
                .base_url(base);
                let mut config = LlmCallConfig::new(model);
                config.prefer_non_streaming = non_streaming;
                if disabled {
                    config.prompt_cache = Some(PromptCacheConfig {
                        enabled: false,
                        ..Default::default()
                    });
                }
                provider
                    .chat_completion(vec![Message::text(MessageRole::User, "Question")], &config)
                    .await
                    .unwrap();
                let requests = capture.0.lock().unwrap();
                assert_eq!(requests.len(), 1);
                if expected && !disabled {
                    assert_eq!(
                        requests[0]["prompt_cache_options"],
                        json!({"ttl":"12h", "mode":"implicit"})
                    );
                } else {
                    assert!(
                        requests[0].get("prompt_cache_options").is_none(),
                        "{base}, {model}"
                    );
                }
                if disabled {
                    assert!(requests[0].get("prompt_cache_key").is_none());
                }
            }
        }
    }
}
