#![allow(clippy::unwrap_used, clippy::expect_used)]
#![cfg(feature = "http")]

use everruns_contracts::call_ledger_jsonl::JsonlCallObserver;
use everruns_contracts::driver_registry::{DriverId, DriverRegistry, ProviderConfig};
use everruns_contracts::runtime_provider::ProviderEndpoint;
use everruns_contracts::{
    LlmCallConfig, OpenAIProtocolChatDriver, OpenResponsesProtocolChatDriver, Provider,
};
use serde_json::json;
use std::sync::Arc;
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
                let temp = tempfile::tempdir().unwrap();
                let ledger_path = temp.path().join("calls.jsonl");
                let observer = Arc::new(JsonlCallObserver::create(&ledger_path).unwrap());
                let mut registry = DriverRegistry::new().with_call_observer(observer.clone());
                registry.register_provider(provider).unwrap();
                let driver = registry
                    .create_chat_driver(&ProviderConfig::for_provider(
                        "openai",
                        DriverId::external("unused"),
                    ))
                    .unwrap();
                let mut config = LlmCallConfig::new("model");
                config
                    .metadata
                    .insert("purpose".into(), "completion_review".into());
                let response = if streaming {
                    driver
                        .chat_completion(&ProviderEndpoint::default(), vec![], &config)
                        .await
                } else {
                    driver
                        .chat_completion_non_streaming(
                            &ProviderEndpoint::default(),
                            vec![],
                            &config,
                        )
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
                assert!(!observer.has_write_error());
                let records: Vec<serde_json::Value> = std::fs::read_to_string(&ledger_path)
                    .unwrap()
                    .lines()
                    .map(|line| serde_json::from_str(line).unwrap())
                    .collect();
                assert_eq!(records.len(), 2);
                assert_eq!(records[1]["metadata"]["purpose"], "completion_review");
                assert_eq!(
                    records[1]["provider_request_id"],
                    json!(request_id.filter(|s| !s.is_empty()))
                );
                assert_eq!(records[1]["response_id"], "resp_resource");
                assert_eq!(records[1]["prompt_tokens"], 2);
            }
        }
    }
}
