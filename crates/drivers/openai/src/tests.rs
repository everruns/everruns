#[cfg(test)]
mod driver_tests {
    use crate::{azure_provider, completions_provider, provider, register_driver};
    use everruns_provider::ProviderEndpoint;
    use everruns_provider::driver_registry::{
        DriverId, DriverRegistry, EmbedRequest, LlmCallConfig, LlmMessage, LlmMessageRole,
        ProviderConfig, ServiceKind,
    };
    use serde_json::{Value, json};
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};
    fn base_config(model: &str) -> LlmCallConfig {
        LlmCallConfig {
            speed: None,
            verbosity: None,
            model: model.to_string(),
            temperature: None,
            max_tokens: None,
            tools: vec![],
            reasoning_effort: None,
            reasoning_state: None,
            metadata: std::collections::HashMap::new(),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            driver_options: Default::default(),
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        }
    }

    #[tokio::test]
    async fn chat_factories_preserve_protocol_auth_and_stateful_continuations() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        assert!(!registry.has_driver(&DriverId::OpenRouter));
        assert_eq!(
            provider("default", "key")
                .endpoint()
                .url("responses")
                .as_deref(),
            Some("https://api.openai.com/v1/responses")
        );
        assert_eq!(
            completions_provider("default", "key")
                .endpoint()
                .url("chat/completions")
                .as_deref(),
            Some("https://api.openai.com/v1/chat/completions")
        );
        for id in [
            DriverId::OpenAI,
            DriverId::AzureOpenAI,
            DriverId::OpenAICompletions,
        ] {
            let completions = id == DriverId::OpenAICompletions;
            let azure = id == DriverId::AzureOpenAI;
            let base_path = if azure { "/openai/v1" } else { "/v1" };
            let operation = if completions {
                "chat/completions"
            } else {
                "responses"
            };
            for registered in [false, true] {
                for full in [false, true] {
                    let server = MockServer::builder().start().await;
                    let terminal = json!({"type":"response.completed","sequence_number":2,"response":{"id":"response-id","object":"response","created_at":0,"model":"gpt-5.4","status":"completed","output":[],"usage":{"input_tokens":10,"output_tokens":2,"total_tokens":12}}});
                    let wire = if completions {
                        "data: {\"id\":\"response-id\",\"choices\":[{\"delta\":{\"content\":\"answer\"},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2}}\n\ndata: [DONE]\n\n".to_string()
                    } else {
                        format!(
                            "data: {{\"type\":\"response.output_text.delta\",\"delta\":\"answer\"}}\n\ndata: {terminal}\n\n"
                        )
                    };
                    Mock::given(method("POST"))
                        .and(path(format!("{base_path}/{operation}")))
                        .and(query_param("version", "preview"))
                        .and(header(
                            if azure { "api-key" } else { "authorization" },
                            if azure {
                                "synthetic-key"
                            } else {
                                "Bearer synthetic-key"
                            },
                        ))
                        .respond_with(
                            ResponseTemplate::new(200)
                                .insert_header("content-type", "text/event-stream")
                                .set_body_string(wire),
                        )
                        .expect(1)
                        .mount(&server)
                        .await;
                    let url = format!(
                        "{}{base_path}{}?version=preview",
                        server.uri(),
                        if full {
                            format!("/{operation}")
                        } else {
                            "/".to_string()
                        }
                    );
                    let mut config = base_config("gpt-5.4");
                    config.parallel_tool_calls = Some(false);
                    config.previous_response_id = Some("prior-response".into());
                    let messages = vec![
                        LlmMessage::text(LlmMessageRole::User, "old"),
                        LlmMessage::text(LlmMessageRole::Assistant, "old answer"),
                        LlmMessage::text(LlmMessageRole::User, "new"),
                    ];
                    let response = if registered {
                        let driver = registry
                            .create_chat_driver(
                                &ProviderConfig::new(id.clone())
                                    .with_api_key("synthetic-key")
                                    .with_base_url(url),
                            )
                            .unwrap();
                        // Engine tool-exchange repair uses this capability to retain result-only continuations.
                        assert_eq!(driver.supports_stateful_responses(), !completions, "{id:?}");
                        driver
                            .chat_completion(&ProviderEndpoint::default(), messages, &config)
                            .await
                    } else {
                        let service = if azure {
                            azure_provider("direct", url, "synthetic-key")
                        } else if completions {
                            completions_provider("direct", "synthetic-key").base_url(url)
                        } else {
                            provider("direct", "synthetic-key").base_url(url)
                        };
                        service.chat_completion(messages, &config).await
                    }
                    .unwrap();
                    assert_eq!(response.text, "answer");
                    assert!(response.tool_calls.is_none());
                    assert!(response.reasoning.is_empty());
                    assert_eq!(
                        response.metadata.response_id.as_deref(),
                        Some("response-id")
                    );
                    assert_eq!(
                        (
                            response.metadata.prompt_tokens,
                            response.metadata.completion_tokens,
                            response.metadata.total_tokens
                        ),
                        (Some(10), Some(2), Some(12))
                    );
                    let requests = server.received_requests().await.unwrap();
                    assert_eq!(requests.len(), 1);
                    assert!(
                        requests[0]
                            .headers
                            .get(if azure { "authorization" } else { "api-key" })
                            .is_none()
                    );
                    let expected = if completions {
                        json!({"model":"gpt-5.4","messages":[{"role":"user","content":"old"},{"role":"assistant","content":"old answer"},{"role":"user","content":"new"}],"stream":true,"stream_options":{"include_usage":true},"parallel_tool_calls":false})
                    } else {
                        json!({"model":"gpt-5.4","input":[{"type":"message","role":"user","content":"new"}],"previous_response_id":"prior-response","stream":true,"parallel_tool_calls":false})
                    };
                    assert_eq!(requests[0].body_json::<Value>().unwrap(), expected);
                }
            }
        }
    }

    #[tokio::test]
    async fn descriptor_services_construct_real_embeddings_with_ordered_results() {
        let mut registry = DriverRegistry::new();
        register_driver(&mut registry);
        let openai = registry.descriptor(&DriverId::OpenAI).unwrap();
        assert_eq!(openai.display_name, "OpenAI");
        assert_eq!(
            openai.services,
            vec![
                ServiceKind::Chat,
                ServiceKind::Realtime,
                ServiceKind::Embeddings
            ]
        );
        assert_eq!(openai.credential_schema.fields[0].name, "api_key");
        for id in [DriverId::AzureOpenAI, DriverId::OpenAICompletions] {
            let descriptor = registry.descriptor(&id).unwrap();
            assert_eq!(descriptor.services, vec![ServiceKind::Chat]);
            assert_eq!(descriptor.credential_schema.fields[0].name, "api_key");
            assert!(
                registry
                    .create_embeddings_driver(&ProviderConfig::new(id).with_api_key("key"))
                    .is_err()
            );
        }
        let server = MockServer::builder().start().await;
        Mock::given(method("POST")).and(path("/v1/embeddings")).and(header("authorization","Bearer synthetic-key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"index":1,"embedding":[3.0,4.0]},{"index":0,"embedding":[1.0,2.0]}],"usage":{"total_tokens":7,"cost":0.0}}))).expect(1).mount(&server).await;
        let driver = registry
            .create_embeddings_driver(
                &ProviderConfig::new(DriverId::OpenAI)
                    .with_api_key("synthetic-key")
                    .with_base_url(format!("{}/v1", server.uri())),
            )
            .unwrap();
        let response = driver
            .embed(
                &ProviderEndpoint::default(),
                EmbedRequest {
                    model: "text-embedding-3-small".into(),
                    texts: vec!["first".into(), "second".into()],
                },
            )
            .await
            .unwrap();
        assert_eq!(response.embeddings, vec![vec![1.0, 2.0], vec![3.0, 4.0]]);
        assert_eq!(response.usage_tokens, Some(7));
        assert_eq!(response.actual_cost_usd, Some(0.0));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].body_json::<Value>().unwrap(),
            json!({"model":"text-embedding-3-small","input":["first","second"],"encoding_format":"float"})
        );
        assert!(
            registry
                .create_embeddings_driver(&ProviderConfig::new(DriverId::OpenAI))
                .is_err()
        );
    }
}

#[cfg(test)]
mod provider_tests {
    use crate::types::{ChatMessage, MessageRole};
    use everruns_provider::ToolCall;
    use serde_json::json;
    #[test]
    fn message_conversion_preserves_roles_empty_text_and_complete_tool_exchange() {
        let messages = [
            ChatMessage {
                role: MessageRole::System,
                content: "rules".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: MessageRole::User,
                content: "question".into(),
                tool_calls: None,
                tool_call_id: None,
            },
            ChatMessage {
                role: MessageRole::Assistant,
                content: String::new(),
                tool_calls: Some(vec![ToolCall {
                    id: "call-id".into(),
                    name: "lookup".into(),
                    arguments: json!({"query":"hello"}),
                }]),
                tool_call_id: None,
            },
            ChatMessage {
                role: MessageRole::Tool,
                content: "result".into(),
                tool_calls: None,
                tool_call_id: Some("call-id".into()),
            },
        ];
        let wire: Vec<_> = messages.iter().map(ChatMessage::to_openai).collect();
        assert_eq!(
            serde_json::to_value(wire).unwrap(),
            json!([
                {"role":"system","content":"rules"},{"role":"user","content":"question"},
                {"role":"assistant","content":"","tool_calls":[{"id":"call-id","type":"function","function":{"name":"lookup","arguments":"{\"query\":\"hello\"}"}}]},
                {"role":"tool","content":"result","tool_call_id":"call-id"}
            ])
        );
    }
}
