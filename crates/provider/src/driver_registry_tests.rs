use super::*;
use crate::runtime_provider::ProviderEndpoint;

#[test]
fn test_disjoint_prompt_tokens_subtracts_cached_subset() {
    // Inclusive providers report a prompt count that includes cached reads;
    // normalization yields the non-cached remainder.
    assert_eq!(disjoint_prompt_tokens(1000, Some(800)), 200);
    // No cache reported => prompt count passes through unchanged.
    assert_eq!(disjoint_prompt_tokens(1000, None), 1000);
    assert_eq!(disjoint_prompt_tokens(1000, Some(0)), 1000);
    // Saturating: a provider reporting cache > input never underflows.
    assert_eq!(disjoint_prompt_tokens(800, Some(1000)), 0);
}

/// A call config with nothing set beyond the model, so a test can assert on
/// exactly what a wrapper adds.
fn bare_call_config() -> LlmCallConfig {
    LlmCallConfig {
        model: "claude-opus-4-8".to_string(),
        temperature: None,
        max_tokens: None,
        tools: vec![],
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        metadata: HashMap::new(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        driver_options: Default::default(),
        parallel_tool_calls: None,
        volatile_suffix_len: 0,
        extra_headers: Vec::new(),
        cache_diagnostics: None,
        capture_request: false,
        limits: Default::default(),
        reasoning_state: None,
    }
}

#[test]
fn provider_config_debug_redacts_runtime_values() {
    let config = ProviderConfig::new(DriverId::OpenAI)
        .with_api_key("secret-key")
        .with_base_url("https://user:password@example.test/v1?token=secret")
        .with_metadata(ProviderMetadata {
            refresh_token: Some("refresh-secret".into()),
            account_id: Some("account-1".into()),
            extra: Some(serde_json::json!({ "client_secret": "metadata-secret" })),
        });
    let debug = format!("{config:?}");
    assert!(debug.contains("ProviderConfig"));
    assert!(debug.contains("openai"));
    assert!(debug.contains("<configured>"));
    for secret in [
        "secret-key",
        "password",
        "token=secret",
        "refresh-secret",
        "metadata-secret",
    ] {
        assert!(!debug.contains(secret), "debug output exposed {secret}");
    }
}

#[test]
fn system_messages_fold_only_system_text_in_transcript_order() {
    use MessageRole::{Assistant, System, Tool, User};
    for (messages, expected) in [
        (vec![], None),
        (
            vec![
                Message::text(User, "user"),
                Message::text(Assistant, "answer"),
                Message::text(Tool, "result"),
            ],
            None,
        ),
        (vec![Message::text(System, "")], Some("")),
        (
            vec![
                Message::text(System, "rules"),
                Message::text(User, "question"),
            ],
            Some("rules"),
        ),
        (
            vec![
                Message::text(System, "first"),
                Message::text(User, "question"),
                Message::text(System, "second"),
                Message::text(Assistant, "answer"),
                Message::text(System, "third"),
            ],
            Some("first\n\nsecond\n\nthird"),
        ),
        (
            vec![
                Message::parts(
                    System,
                    vec![
                        LlmContentPart::text("foo"),
                        LlmContentPart::image("image"),
                        LlmContentPart::audio("audio"),
                        LlmContentPart::text("bar"),
                    ],
                ),
                Message::text(System, "next"),
            ],
            Some("foobar\n\nnext"),
        ),
    ] {
        assert_eq!(fold_system_messages(&messages).as_deref(), expected);
    }
}

#[test]
fn prefix_preserves_all_media_and_changes_only_the_first_text_part() {
    let mut plain = Message::text(MessageRole::User, "Hello");
    plain.prepend_text_prefix("[Alice] ");
    assert!(matches!(plain.content, MessageContent::Text(ref text) if text == "[Alice] Hello"));
    for (parts, expected) in [
        (vec![], vec![("text", "[Alice] ")]),
        (
            vec![
                LlmContentPart::image("image"),
                LlmContentPart::audio("audio"),
            ],
            vec![("text", "[Alice] "), ("image", "image"), ("audio", "audio")],
        ),
        (
            vec![
                LlmContentPart::text("Hello"),
                LlmContentPart::image("image"),
            ],
            vec![("text", "[Alice] Hello"), ("image", "image")],
        ),
        (
            vec![
                LlmContentPart::image("image"),
                LlmContentPart::text("Hello"),
                LlmContentPart::audio("audio"),
                LlmContentPart::text("later"),
            ],
            vec![
                ("image", "image"),
                ("text", "[Alice] Hello"),
                ("audio", "audio"),
                ("text", "later"),
            ],
        ),
        (
            vec![LlmContentPart::text(""), LlmContentPart::text("later")],
            vec![("text", "[Alice] "), ("text", "later")],
        ),
    ] {
        let mut message = Message::parts(MessageRole::Tool, parts);
        message.tool_call_id = Some("call-1".into());
        message.prepend_text_prefix("[Alice] ");
        let MessageContent::Parts(parts) = &message.content else {
            panic!("parts must remain parts")
        };
        let actual: Vec<_> = parts
            .iter()
            .map(|part| match part {
                LlmContentPart::Text { text } => ("text", text.as_str()),
                LlmContentPart::Image { url } => ("image", url.as_str()),
                LlmContentPart::Audio { url } => ("audio", url.as_str()),
                LlmContentPart::File { url, .. } => ("file", url.as_str()),
                LlmContentPart::ProviderOpaque(opaque) => {
                    ("provider_opaque", opaque.provider.as_str())
                }
            })
            .collect();
        assert_eq!(actual, expected);
        assert_eq!(message.role, MessageRole::Tool);
        assert_eq!(message.tool_call_id.as_deref(), Some("call-1"));
    }
}
struct FixtureDriver(&'static str);

#[async_trait]
impl ChatDriver for FixtureDriver {
    async fn chat_completion_stream(
        &self,
        _: &ProviderEndpoint,
        _: Vec<Message>,
        _: &LlmCallConfig,
    ) -> Result<LlmResponseStream> {
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta(self.0.into())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
    async fn list_models(&self, _: &ProviderEndpoint) -> Result<Option<Vec<DiscoveredModel>>> {
        Ok(Some(vec![DiscoveredModel {
            model_id: self.0.into(),
            display_name: None,
            created_at: None,
            owned_by: None,
            capabilities: vec!["chat".into()],
            discovered_profile: None,
        }]))
    }
    async fn compact(
        &self,
        _: &ProviderEndpoint,
        request: CompactRequest,
    ) -> Result<Option<CompactResponse>> {
        Ok(Some(CompactResponse {
            output: vec![crate::compact::CompactOutputItem::Compaction {
                encrypted_content: request.model,
            }],
            usage: None,
        }))
    }
    fn supports_compact(&self) -> bool {
        true
    }
    fn supports_stateful_responses(&self) -> bool {
        true
    }
    fn effective_context_window(&self, model: &str) -> Option<usize> {
        (model == "known").then_some(12345)
    }
    fn supports_parallel_tool_calls(&self, model: &str) -> bool {
        model == "known"
    }
}

fn compact_fixture() -> CompactRequest {
    CompactRequest {
        reasoning_state: None,
        model: "compact-model".into(),
        input: vec![],
        previous_response_id: None,
        instructions: None,
    }
}

#[tokio::test]
async fn default_and_boxed_drivers_preserve_optional_operations_and_model_capabilities() {
    struct DefaultDriver;
    #[async_trait]
    impl ChatDriver for DefaultDriver {
        async fn chat_completion_stream(
            &self,
            _: &ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    let endpoint = ProviderEndpoint::default();
    assert!(!DefaultDriver.supports_compact());
    assert!(!DefaultDriver.supports_stateful_responses());
    assert!(!DefaultDriver.supports_parallel_tool_calls("known"));
    assert_eq!(DefaultDriver.effective_context_window("known"), None);
    assert!(
        DefaultDriver
            .list_models(&endpoint)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        DefaultDriver
            .compact(&endpoint, compact_fixture())
            .await
            .unwrap()
            .is_none()
    );
    let boxed: BoxedChatDriver = Box::new(FixtureDriver("boxed"));
    assert!(boxed.supports_compact());
    assert!(boxed.supports_stateful_responses());
    for (model, expected) in [("known", true), ("unknown", false)] {
        assert_eq!(boxed.supports_parallel_tool_calls(model), expected);
        assert_eq!(
            boxed.effective_context_window(model),
            expected.then_some(12345)
        );
    }
    assert_eq!(
        boxed
            .chat_completion(&endpoint, vec![], &bare_call_config())
            .await
            .unwrap()
            .text,
        "boxed"
    );
}

#[tokio::test]
async fn registry_replacement_changes_factory_and_preserves_other_descriptors() {
    let mut registry = DriverRegistry::new();
    assert!(registry.registered_providers().is_empty());
    registry.register(DriverId::LlmSim, |_| Box::new(FixtureDriver("first")));
    registry.register_descriptor(DriverDescriptor {
        display_name: "OpenAI custom".into(),
        services: vec![ServiceKind::Chat, ServiceKind::Realtime],
        ..DriverDescriptor::chat_only(DriverId::OpenAI, |_| Box::new(FixtureDriver("other")))
    });
    let config = ProviderConfig::new(DriverId::LlmSim);
    let endpoint = ProviderEndpoint::default();
    assert_eq!(
        registry
            .create_chat_driver(&config)
            .unwrap()
            .chat_completion(&endpoint, vec![], &bare_call_config())
            .await
            .unwrap()
            .text,
        "first"
    );
    registry.register_or_replace(DriverId::LlmSim, |_| Box::new(FixtureDriver("replacement")));
    assert_eq!(
        registry
            .create_chat_driver(&config)
            .unwrap()
            .chat_completion(&endpoint, vec![], &bare_call_config())
            .await
            .unwrap()
            .text,
        "replacement"
    );
    assert!(registry.has_driver(&DriverId::LlmSim));
    assert!(!registry.has_driver(&DriverId::Anthropic));
    assert_eq!(
        registry.providers_for(ServiceKind::Realtime),
        vec![DriverId::OpenAI]
    );
    let mut chat = registry.providers_for(ServiceKind::Chat);
    chat.sort_by_key(|id| id.to_string());
    assert_eq!(chat, vec![DriverId::LlmSim, DriverId::OpenAI]);
    assert!(registry.supports(&DriverId::OpenAI, ServiceKind::Realtime));
    assert!(!registry.supports(&DriverId::LlmSim, ServiceKind::Realtime));
    assert!(!registry.supports(&DriverId::Gemini, ServiceKind::Chat));
    assert_eq!(
        registry.descriptor(&DriverId::OpenAI).unwrap().display_name,
        "OpenAI custom"
    );
    assert_eq!(
        registry
            .create_chat_driver(
                &ProviderConfig::new(DriverId::OpenAI).with_api_key("synthetic-key")
            )
            .unwrap()
            .chat_completion(&endpoint, vec![], &bare_call_config())
            .await
            .unwrap()
            .text,
        "other"
    );
    let defaults =
        DriverDescriptor::chat_only(DriverId::Anthropic, |_| Box::new(FixtureDriver("default")));
    assert_eq!(defaults.display_name, "anthropic");
    let sim = registry.descriptor(&DriverId::LlmSim).unwrap();
    assert!(sim.credential_schema.fields.is_empty());
    assert_eq!(sim.services, vec![ServiceKind::Chat]);
    assert!(sim.chat.is_some());
    let real = registry.descriptor(&DriverId::OpenAI).unwrap();
    assert_eq!(real.credential_schema.fields.len(), 1);
    assert_eq!(real.credential_schema.fields[0].name, "api_key");
    assert!(real.credential_schema.fields[0].required);
    assert!(registry.descriptor(&DriverId::Gemini).is_none());
}

#[test]
#[should_panic(expected = "already registered")]
fn duplicate_registration_rejects_an_existing_driver() {
    let mut registry = DriverRegistry::new();
    registry.register(DriverId::OpenAI, |_| Box::new(FixtureDriver("first")));
    registry.register(DriverId::OpenAI, |_| Box::new(FixtureDriver("second")));
}

#[tokio::test]
async fn factory_receives_complete_config_and_external_metadata_auth_remains_keyless() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = seen.clone();
    let mut registry = DriverRegistry::new();
    registry.register_external("CUSTOM", move |config| {
        capture.lock().unwrap().push(config.clone());
        Box::new(FixtureDriver("external"))
    });
    let metadata = ProviderMetadata {
        refresh_token: Some("refresh".into()),
        account_id: Some("account".into()),
        extra: Some(serde_json::json!({"region":"west"})),
    };
    for key in [None, Some("synthetic-key")] {
        let mut config = ProviderConfig::for_provider("connection", DriverId::external("custom"))
            .with_base_url("https://gateway.example/v1")
            .with_metadata(metadata.clone());
        if let Some(key) = key {
            config = config.with_api_key(key);
        }
        let response = registry
            .create_chat_driver(&config)
            .unwrap()
            .chat_completion(&ProviderEndpoint::default(), vec![], &bare_call_config())
            .await
            .unwrap();
        assert_eq!(response.text, "external");
        let received = seen.lock().unwrap().pop().unwrap();
        assert_eq!(received.provider.as_str(), "connection");
        assert_eq!(received.provider_type, DriverId::external("custom"));
        assert_eq!(received.api_key.as_deref(), key);
        assert_eq!(received.credential("api_key"), key);
        assert_eq!(received.credentials.len(), usize::from(key.is_some()));
        assert_eq!(
            received.base_url.as_deref(),
            Some("https://gateway.example/v1")
        );
        assert_eq!(received.metadata, metadata);
    }
    assert!(
        registry
            .descriptor(&DriverId::external("custom"))
            .unwrap()
            .credential_schema
            .fields
            .is_empty()
    );
}

#[test]
fn registry_distinguishes_missing_driver_from_missing_chat_service() {
    let mut registry = DriverRegistry::new();
    assert!(
        matches!(registry.create_chat_driver(&ProviderConfig::new(DriverId::Anthropic)), Err(AgentLoopError::DriverNotRegistered(id)) if id == "anthropic")
    );
    registry.register_descriptor(DriverDescriptor {
        id: DriverId::external("embeddings-only"),
        display_name: "Embeddings Only".into(),
        services: vec![ServiceKind::Embeddings],
        credential_schema: CredentialFormSchema::empty(),
        base_url_env: None,
        oauth: None,
        chat: None,
        embeddings: None,
    });
    match registry.create_chat_driver(&ProviderConfig::new(DriverId::external("embeddings-only"))) {
        Err(AgentLoopError::Llm(error)) => assert_eq!(
            error.message,
            "Provider driver 'embeddings-only' does not implement the chat service."
        ),
        _ => panic!("expected a missing-chat-service error"),
    }
}

#[tokio::test]
async fn credential_gate_rejects_every_io_operation_before_dispatch() {
    struct ForbiddenDriver;
    #[async_trait]
    impl ChatDriver for ForbiddenDriver {
        async fn chat_completion_stream(
            &self,
            _: &ProviderEndpoint,
            _: Vec<Message>,
            _: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            panic!("unauthenticated stream dispatch")
        }
        async fn list_models(&self, _: &ProviderEndpoint) -> Result<Option<Vec<DiscoveredModel>>> {
            panic!("unauthenticated model dispatch")
        }
        async fn compact(
            &self,
            _: &ProviderEndpoint,
            _: CompactRequest,
        ) -> Result<Option<CompactResponse>> {
            panic!("unauthenticated compact dispatch")
        }
    }
    let mut registry = DriverRegistry::new();
    registry.register(DriverId::OpenAI, |config| {
        if config.api_key.is_some() {
            Box::new(FixtureDriver("authenticated"))
        } else {
            Box::new(ForbiddenDriver)
        }
    });
    let driver = registry
        .create_chat_driver(&ProviderConfig::new(DriverId::OpenAI))
        .unwrap();
    let endpoint = ProviderEndpoint::default();
    let stream_error = match driver
        .chat_completion_stream(&endpoint, vec![], &bare_call_config())
        .await
    {
        Err(error) => error,
        Ok(_) => panic!("expected authentication error"),
    };
    for error in [
        stream_error,
        driver
            .chat_completion(&endpoint, vec![], &bare_call_config())
            .await
            .unwrap_err(),
        driver.list_models(&endpoint).await.unwrap_err(),
        driver
            .compact(&endpoint, compact_fixture())
            .await
            .unwrap_err(),
    ] {
        assert_eq!(error.llm_error_kind(), Some(LlmErrorKind::Authentication));
        assert_eq!(
            error.to_string(),
            "LLM error: API key is required. Configure the API key in provider settings."
        );
    }
    let driver = registry
        .create_chat_driver(&ProviderConfig::new(DriverId::OpenAI).with_api_key("synthetic-key"))
        .unwrap();
    assert_eq!(
        driver
            .chat_completion(&endpoint, vec![], &bare_call_config())
            .await
            .unwrap()
            .text,
        "authenticated"
    );
    assert_eq!(
        driver.list_models(&endpoint).await.unwrap().unwrap()[0].model_id,
        "authenticated"
    );
    assert_eq!(
        serde_json::to_value(
            driver
                .compact(&endpoint, compact_fixture())
                .await
                .unwrap()
                .unwrap()
                .output
        )
        .unwrap(),
        serde_json::json!([{"type":"compaction","encrypted_content":"compact-model"}])
    );
}

#[tokio::test]
async fn request_options_preserve_calls_and_apply_headers_and_diagnostics_independently() {
    struct CapturingDriver(Arc<std::sync::Mutex<Vec<LlmCallConfig>>>);
    impl CapturingDriver {
        fn capture(
            &self,
            endpoint: &ProviderEndpoint,
            messages: &[Message],
            config: &LlmCallConfig,
        ) {
            assert_eq!(
                endpoint.url("probe").as_deref(),
                Some("https://gateway.example/v1/probe")
            );
            assert_eq!(messages.len(), 1);
            assert_eq!(messages[0].role, MessageRole::User);
            assert_eq!(messages[0].content_as_text(), "request text");
            self.0.lock().unwrap().push(config.clone());
        }
    }
    #[async_trait]
    impl ChatDriver for CapturingDriver {
        async fn chat_completion_stream(
            &self,
            endpoint: &ProviderEndpoint,
            messages: Vec<Message>,
            config: &LlmCallConfig,
        ) -> Result<LlmResponseStream> {
            self.capture(endpoint, &messages, config);
            FixtureDriver("stream")
                .chat_completion_stream(endpoint, messages, config)
                .await
        }
        async fn chat_completion(
            &self,
            endpoint: &ProviderEndpoint,
            messages: Vec<Message>,
            config: &LlmCallConfig,
        ) -> Result<LlmResponse> {
            self.capture(endpoint, &messages, config);
            FixtureDriver("completion")
                .chat_completion(endpoint, messages, config)
                .await
        }
    }
    let provider = crate::Provider::new("fixture", FixtureDriver("endpoint"))
        .base_url("https://gateway.example/v1");
    for (headers, diagnostics) in [(false, false), (true, false), (false, true), (true, true)] {
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let options = crate::provider::ProviderRequestOptions {
            headers: if headers {
                vec![crate::provider::ProviderRequestHeader {
                    name: "x-base".into(),
                    value: "connection".into(),
                }]
            } else {
                vec![]
            },
            cache_diagnostics: diagnostics,
        };
        let driver = RequestOptionsDriver::wrap(Box::new(CapturingDriver(seen.clone())), &options);
        let mut config = bare_call_config();
        config.model = "requested-model".into();
        config.temperature = Some(0.25);
        config.max_tokens = Some(42);
        config
            .metadata
            .insert("session_id".into(), "session-one".into());
        config.previous_response_id = Some("response-one".into());
        config.extra_headers = vec![("x-base".into(), "original".into())];
        config.cache_diagnostics = Some(CacheDiagnosticsConfig {
            enabled: false,
            previous_message_id: Some("existing".into()),
        });
        let mut stream = driver
            .chat_completion_stream(
                provider.endpoint(),
                vec![Message::text(MessageRole::User, "request text")],
                &config,
            )
            .await
            .unwrap();
        use futures::StreamExt;
        assert!(
            matches!(stream.next().await.unwrap().unwrap(), LlmStreamEvent::TextDelta(text) if text == "stream")
        );
        assert!(matches!(
            stream.next().await.unwrap().unwrap(),
            LlmStreamEvent::Done(_)
        ));
        assert!(stream.next().await.is_none());
        assert_eq!(
            driver
                .chat_completion(
                    provider.endpoint(),
                    vec![Message::text(MessageRole::User, "request text")],
                    &config
                )
                .await
                .unwrap()
                .text,
            "completion"
        );
        let mut expected_headers = vec![("x-base".into(), "original".into())];
        if headers {
            expected_headers.push(("x-base".into(), "connection".into()));
        }
        let observed = seen.lock().unwrap();
        assert_eq!(observed.len(), 2);
        for received in observed.iter() {
            assert_eq!(received.extra_headers, expected_headers);
            let diagnostic = received.cache_diagnostics.as_ref().unwrap();
            assert_eq!(diagnostic.enabled, diagnostics);
            assert_eq!(
                diagnostic.previous_message_id.as_deref(),
                Some(if diagnostics {
                    "response-one"
                } else {
                    "existing"
                })
            );
            assert_eq!(received.model, "requested-model");
            assert_eq!(received.temperature, Some(0.25));
            assert_eq!(received.max_tokens, Some(42));
            assert_eq!(received.metadata, config.metadata);
            assert_eq!(received.previous_response_id, config.previous_response_id);
        }
        assert_eq!(
            config.extra_headers,
            vec![("x-base".into(), "original".into())]
        );
        assert!(!config.cache_diagnostics.as_ref().unwrap().enabled);
        assert_eq!(
            config
                .cache_diagnostics
                .as_ref()
                .unwrap()
                .previous_message_id
                .as_deref(),
            Some("existing")
        );
    }
    let options = crate::provider::ProviderRequestOptions {
        headers: vec![],
        cache_diagnostics: true,
    };
    let wrapped = RequestOptionsDriver::wrap(Box::new(FixtureDriver("forwarded")), &options);
    assert!(wrapped.supports_compact());
    assert!(wrapped.supports_stateful_responses());
    for (model, expected) in [("known", true), ("unknown", false)] {
        assert_eq!(wrapped.supports_parallel_tool_calls(model), expected);
        assert_eq!(
            wrapped.effective_context_window(model),
            expected.then_some(12345)
        );
    }
    assert_eq!(
        wrapped
            .list_models(provider.endpoint())
            .await
            .unwrap()
            .unwrap()[0]
            .model_id,
        "forwarded"
    );
    assert_eq!(
        serde_json::to_value(
            wrapped
                .compact(provider.endpoint(), compact_fixture())
                .await
                .unwrap()
                .unwrap()
                .output
        )
        .unwrap(),
        serde_json::json!([{"type":"compaction","encrypted_content":"compact-model"}])
    );
}
