#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "http")]

use everruns_contracts::{
    LlmCallConfig, OpenAIProtocolChatDriver, OpenResponsesProtocolChatDriver, Provider,
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

// Local wire fixtures only: exercise both parsers and both HTTP delivery modes.
#[tokio::test]
async fn request_id_is_distinct_from_response_id_and_optional() {
    for responses in [false, true] {
        for streaming in [false, true] {
            for request_id in [Some("req_correlation"), None, Some("")] {
                let server = MockServer::start().await;
                let resource = if responses {
                    json!({"id":"resp_resource", "object":"response", "status":"completed", "output":[],
                        "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}})
                } else {
                    json!({"id":"resp_resource", "model":"model", "choices":[{
                        "index":0, "message":{"role":"assistant","content":"hello"},
                        "delta":{"content":"hello"}, "finish_reason":"stop"}],
                        "usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}})
                };
                let body = if !streaming {
                    resource.to_string()
                } else if responses {
                    format!(
                        "data: {}\n\n",
                        json!({"type":"response.completed","response":resource})
                    )
                } else {
                    format!("data: {resource}\n\ndata: [DONE]\n\n")
                };
                let mut reply = ResponseTemplate::new(200).set_body_raw(
                    body,
                    if streaming {
                        "text/event-stream"
                    } else {
                        "application/json"
                    },
                );
                if let Some(id) = request_id {
                    reply = reply.insert_header("x-request-id", id);
                }
                Mock::given(method("POST"))
                    .and(path(if responses {
                        "/responses"
                    } else {
                        "/chat/completions"
                    }))
                    .respond_with(reply)
                    .expect(1)
                    .mount(&server)
                    .await;
                let provider = if responses {
                    Provider::new("openai", OpenResponsesProtocolChatDriver::new())
                } else {
                    Provider::new("openai", OpenAIProtocolChatDriver::new())
                }
                .base_url(server.uri());
                let config = LlmCallConfig::new("model");
                let response = if streaming {
                    provider.chat_completion(vec![], &config).await
                } else {
                    provider
                        .chat_completion_non_streaming(vec![], &config)
                        .await
                }
                .unwrap();
                assert_eq!(
                    response.metadata.provider_request_id.as_deref(),
                    request_id.filter(|s| !s.is_empty()),
                    "responses={responses} streaming={streaming}"
                );
                assert_eq!(
                    response.metadata.response_id.as_deref(),
                    Some("resp_resource")
                );
            }
        }
    }
}
