use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

struct Noop;
#[async_trait]
impl ChatDriver for Noop {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<crate::Message>,
        _config: &crate::LlmCallConfig,
    ) -> Result<crate::LlmResponseStream> {
        unreachable!("configuration-only fixture must not execute")
    }
}

struct Catalog;
#[async_trait]
impl ChatDriver for Catalog {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<crate::Message>,
        _config: &crate::LlmCallConfig,
    ) -> Result<crate::LlmResponseStream> {
        unreachable!("catalog-only fixture must not execute")
    }

    async fn list_models(
        &self,
        _endpoint: &ProviderEndpoint,
    ) -> Result<Option<Vec<crate::driver_registry::DiscoveredModel>>> {
        Ok(Some(vec![crate::driver_registry::DiscoveredModel {
            model_id: "gpt-5.6-terra".to_string(),
            display_name: None,
            created_at: None,
            owned_by: None,
            capabilities: vec!["chat".to_string()],
            discovered_profile: None,
        }]))
    }
}

struct NativeNonStreaming {
    native_called: Arc<AtomicBool>,
    connect_delay: Duration,
    event_delay: Duration,
    response: String,
}

#[async_trait]
impl ChatDriver for NativeNonStreaming {
    fn supports_native_non_streaming(&self) -> bool {
        true
    }

    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<crate::Message>,
        _config: &crate::LlmCallConfig,
    ) -> Result<crate::LlmResponseStream> {
        tokio::time::sleep(self.connect_delay).await;
        let events = vec![
            (
                Duration::ZERO,
                Ok(crate::LlmStreamEvent::TextDelta(self.response.clone())),
            ),
            (
                self.event_delay,
                Ok(crate::LlmStreamEvent::Done(Box::default())),
            ),
        ];
        Ok(Box::pin(futures::stream::iter(events).then(
            |(delay, event)| async move {
                tokio::time::sleep(delay).await;
                event
            },
        )))
    }

    async fn chat_completion_non_streaming(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<crate::Message>,
        _config: &crate::LlmCallConfig,
    ) -> Result<crate::LlmResponse> {
        self.native_called.store(true, Ordering::SeqCst);
        tokio::time::sleep(self.connect_delay).await;
        tokio::time::sleep(self.event_delay).await;
        Ok(crate::LlmResponse {
            text: self.response.clone(),
            reasoning: vec![],
            tool_calls: None,
            metadata: Default::default(),
        })
    }
}

#[test]
fn the_driver_kind_falls_back_to_the_runtime_key() {
    assert_eq!(
        RuntimeProvider::new("openai", Noop).driver_id(),
        crate::provider::DriverId::OpenAI
    );
}

#[test]
fn a_declared_driver_kind_survives_a_caller_chosen_key() {
    let provider =
        RuntimeProvider::new("my-gateway", Noop).with_driver_id(crate::provider::DriverId::OpenAI);
    assert_eq!(provider.id().as_str(), "my-gateway");
    assert_eq!(provider.driver_id(), crate::provider::DriverId::OpenAI);
}

#[tokio::test]
async fn models_enriches_bare_ids_through_the_declared_driver_kind() {
    let catalog = RuntimeProvider::new("my-gateway", Catalog)
        .with_driver_id(crate::provider::DriverId::OpenAI)
        .models()
        .await
        .expect("catalog request")
        .expect("driver offers a catalog");
    assert_eq!(catalog.len(), 1);
    // The driver returned a bare id; the profile registry named it.
    assert!(catalog[0].display_name.is_some());
}

#[tokio::test]
async fn a_driver_without_a_catalog_reports_no_catalog() {
    assert!(
        RuntimeProvider::new("openai", Noop)
            .models()
            .await
            .expect("catalog request")
            .is_none()
    );
}

#[tokio::test]
async fn bounded_non_streaming_calls_enforce_the_total_timeout_before_headers() {
    let native_called = Arc::new(AtomicBool::new(false));
    let provider = RuntimeProvider::new(
        "bounded",
        NativeNonStreaming {
            native_called: Arc::clone(&native_called),
            connect_delay: Duration::from_millis(100),
            event_delay: Duration::ZERO,
            response: "done".into(),
        },
    );
    let mut config = crate::LlmCallConfig::new("model");
    config.limits =
        crate::turn_collector::TurnLimits::default().with_total(Duration::from_millis(10));

    let error = provider
        .chat_completion_non_streaming(vec![], &config)
        .await
        .expect_err("the call must time out");

    assert!(error.to_string().contains("did not finish"));
    assert!(native_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn bounded_non_streaming_calls_enforce_the_total_timeout_during_the_body() {
    let native_called = Arc::new(AtomicBool::new(false));
    let provider = RuntimeProvider::new(
        "bounded",
        NativeNonStreaming {
            native_called: Arc::clone(&native_called),
            connect_delay: Duration::ZERO,
            event_delay: Duration::from_millis(100),
            response: "partial".into(),
        },
    );
    let mut config = crate::LlmCallConfig::new("model");
    config.limits =
        crate::turn_collector::TurnLimits::default().with_total(Duration::from_millis(10));

    let error = provider
        .chat_completion_non_streaming(vec![], &config)
        .await
        .expect_err("the body must time out");

    assert!(error.to_string().contains("did not finish"));
    assert!(native_called.load(Ordering::SeqCst));
}

#[tokio::test]
async fn bounded_non_streaming_calls_reject_oversized_responses() {
    let native_called = Arc::new(AtomicBool::new(false));
    let provider = RuntimeProvider::new(
        "bounded",
        NativeNonStreaming {
            native_called: Arc::clone(&native_called),
            connect_delay: Duration::ZERO,
            event_delay: Duration::ZERO,
            response: "too large".into(),
        },
    );
    let mut config = crate::LlmCallConfig::new("model");
    config.limits = crate::turn_collector::TurnLimits::default().with_max_response_bytes(3);

    let error = provider
        .chat_completion_non_streaming(vec![], &config)
        .await
        .expect_err("the response must exceed the cap");

    assert!(error.to_string().contains("3-byte limit"));
    assert!(!native_called.load(Ordering::SeqCst));
}

#[test]
fn endpoint_deduplicates_only_complete_path_segments() {
    for (base, path, expected) in [
        (
            "https://service.example/v1/",
            "chat",
            "https://service.example/v1/chat",
        ),
        (
            "https://service.example/v1/chat",
            "/chat",
            "https://service.example/v1/chat",
        ),
        (
            "https://service.example/v1/notchat",
            "chat",
            "https://service.example/v1/notchat/chat",
        ),
        ("https://chat", "chat", "https://chat/chat"),
        (
            "https://service.example/v1beta",
            "models/gemini-2.5-flash:streamGenerateContent?alt=sse",
            "https://service.example/v1beta/models/gemini-2.5-flash:streamGenerateContent?alt=sse",
        ),
        (
            "https://service.example/v1?token=a%2Fb#base",
            "chat?alt=sse#operation",
            "https://service.example/v1/chat?token=a%2Fb&alt=sse#operation",
        ),
        (
            "https://service.example/v1/chat?token=x",
            "chat?alt=sse",
            "https://service.example/v1/chat?token=x&alt=sse",
        ),
        (
            "https://service.example/v1?token=x",
            "chat",
            "https://service.example/v1/chat?token=x",
        ),
        (
            "https://service.example/v1/chat?token=x",
            "chat",
            "https://service.example/v1/chat?token=x",
        ),
        (
            "https://service.example/v1/notchat/completions",
            "chat/completions",
            "https://service.example/v1/notchat/completions/chat/completions",
        ),
        (
            "https://service.example/v1/chat/",
            "",
            "https://service.example/v1/chat",
        ),
    ] {
        let endpoint = ProviderEndpoint {
            base_url: Some(base.into()),
            ..Default::default()
        };
        assert_eq!(
            endpoint.url(path).as_deref(),
            Some(expected),
            "{base} + {path}"
        );
    }
    assert_eq!(ProviderEndpoint::default().url("chat"), None);
}

#[test]
fn provider_key_deserialization_is_canonical() {
    let key: ProviderKey = serde_json::from_str(r#"" Gateway-PROD ""#).unwrap();
    assert_eq!(key.as_str(), "gateway-prod");
    assert_eq!(serde_json::to_string(&key).unwrap(), r#""gateway-prod""#);
}

#[tokio::test]
async fn debug_redacts_auth_and_header_values() {
    let provider = RuntimeProvider::new("Gateway", Noop)
        .base_url("https://example.test/")
        .header("x-secret", "hidden-service-value")
        .auth(BearerAuth::new("hidden-key"));
    let debug = format!("{provider:?}");
    assert!(debug.contains("gateway"));
    assert!(debug.contains("x-secret"));
    assert!(!debug.contains("hidden-service-value"));
    assert!(!debug.contains("hidden-key"));
    let request = provider
        .endpoint()
        .resolve(
            "POST",
            "https://user:password@example.test/chat?token=query-secret#fragment-secret",
            b"{}",
        )
        .await
        .unwrap();
    assert_eq!(
        format!("{request:?}"),
        "ResolvedProviderRequest { url: \"https://example.test/chat\", headers: [\"x-secret\", \"authorization\"] }"
    );
}

#[test]
fn duplicate_registration_is_explicit() {
    let mut registry = RuntimeProviderRegistry::new();
    registry.register(RuntimeProvider::new("a", Noop)).unwrap();
    let error = registry
        .register(RuntimeProvider::new("A", Noop).base_url("https://rejected.example"))
        .unwrap_err();
    let crate::AgentLoopError::Configuration(message) = error else {
        panic!("duplicate must be a configuration error")
    };
    assert_eq!(
        message,
        "provider 'a' is already registered; use replace() to overwrite intentionally"
    );
    assert_eq!(registry.ids(), vec!["a"]);
    let original = registry.get(&ProviderKey::new(" A ")).unwrap();
    assert!(
        original.endpoint().base_url().is_none(),
        "rejected duplicate must not replace original"
    );
    let old =
        registry.replace(RuntimeProvider::new("a", Noop).base_url("https://replacement.example"));
    assert!(Arc::ptr_eq(&old.unwrap(), &original));
    assert_eq!(
        registry
            .get(&ProviderKey::new("a"))
            .unwrap()
            .endpoint()
            .base_url(),
        Some("https://replacement.example")
    );
    assert!(
        original.endpoint().base_url().is_none(),
        "existing handle retains old provider"
    );
    assert!(registry.get(&ProviderKey::new("missing")).is_none());
    assert!(registry.replace(RuntimeProvider::new("z", Noop)).is_none());
    assert_eq!(registry.ids(), ["a", "z"]);
}

#[tokio::test]
async fn one_protocol_serves_distinct_provider_identities() {
    let protocol: Arc<dyn ChatDriver> = Arc::new(Noop);
    let first = Provider::from_driver("first", protocol.clone())
        .base_url("https://first.example/v1")
        .header("x-service", "first")
        .auth(BearerAuth::new("first-key"));
    let second = Provider::from_driver("second", protocol.clone())
        .base_url("https://second.example/v1")
        .header("x-service", "second")
        .auth(BearerAuth::new("second-key"));

    assert!(Arc::ptr_eq(
        first.driver().unwrap(),
        second.driver().unwrap()
    ));
    let first_request = first
        .endpoint()
        .resolve("POST", first.endpoint().url("chat").unwrap(), b"{}")
        .await
        .unwrap();
    let second_request = second
        .endpoint()
        .resolve("POST", second.endpoint().url("chat").unwrap(), b"{}")
        .await
        .unwrap();
    assert_eq!(first_request.url, "https://first.example/v1/chat");
    assert_eq!(second_request.url, "https://second.example/v1/chat");
    assert_eq!(
        first_request.headers,
        [
            ("x-service".into(), "first".into()),
            ("authorization".into(), "Bearer first-key".into())
        ]
    );
    assert_eq!(
        second_request.headers,
        [
            ("x-service".into(), "second".into()),
            ("authorization".into(), "Bearer second-key".into())
        ]
    );
}

#[tokio::test]
async fn refreshable_auth_is_resolved_for_each_request() {
    struct Rotating(std::sync::atomic::AtomicUsize);
    #[async_trait]
    impl ProviderAuth for Rotating {
        async fn headers(&self, request: ProviderAuthRequest<'_>) -> Result<Vec<(String, String)>> {
            assert_eq!(request.method, "POST");
            assert_eq!(request.url, "https://service.example/chat");
            assert_eq!(
                request.headers,
                [
                    ("AUTHORIZATION".into(), "stale".into()),
                    ("x-static".into(), "preserve".into())
                ]
            );
            let token = self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Ok(vec![
                ("authorization".into(), format!("Bearer token-{token}")),
                (
                    "x-signed-body".into(),
                    String::from_utf8_lossy(request.body).into_owned(),
                ),
            ])
        }
        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    let endpoint = ProviderEndpoint {
        base_url: Some("https://service.example".into()),
        headers: vec![
            ("AUTHORIZATION".into(), "stale".into()),
            ("x-static".into(), "preserve".into()),
        ],
        auth: Some(Arc::new(Rotating(std::sync::atomic::AtomicUsize::new(0)))),
    };
    let first = endpoint
        .resolve("POST", "https://service.example/chat", b"one")
        .await
        .unwrap();
    let second = endpoint
        .resolve("POST", "https://service.example/chat", b"two")
        .await
        .unwrap();
    assert_eq!(
        first.headers,
        [
            ("x-static".into(), "preserve".into()),
            ("authorization".into(), "Bearer token-1".into()),
            ("x-signed-body".into(), "one".into())
        ]
    );
    assert_eq!(
        second.headers,
        [
            ("x-static".into(), "preserve".into()),
            ("authorization".into(), "Bearer token-2".into()),
            ("x-signed-body".into(), "two".into())
        ]
    );
}

#[tokio::test]
async fn provider_identity_prefixes_start_and_stream_errors() {
    struct Failing {
        fail_to_start: bool,
    }
    #[async_trait]
    impl ChatDriver for Failing {
        async fn chat_completion_stream(
            &self,
            _endpoint: &ProviderEndpoint,
            _messages: Vec<crate::Message>,
            _config: &crate::LlmCallConfig,
        ) -> Result<crate::LlmResponseStream> {
            if self.fail_to_start {
                return Err(crate::AgentLoopError::llm("request failed"));
            }
            Ok(Box::pin(futures::stream::once(async {
                Err(crate::AgentLoopError::llm("stream failed"))
            })))
        }
    }

    let config = crate::LlmCallConfig {
        model: "model".into(),
        ..Default::default()
    };
    let start = Provider::new(
        "customer-gateway",
        Failing {
            fail_to_start: true,
        },
    );
    let error = match start.chat_completion_stream(Vec::new(), &config).await {
        Ok(_) => panic!("the test driver should fail before returning a stream"),
        Err(error) => error,
    };
    let crate::AgentLoopError::Llm(error) = error else {
        panic!("LLM error variant must survive")
    };
    assert_eq!(error.message, "provider 'customer-gateway': request failed");

    let stream = Provider::new(
        "customer-gateway",
        Failing {
            fail_to_start: false,
        },
    );
    let error = stream
        .chat_completion_stream(Vec::new(), &config)
        .await
        .unwrap()
        .next()
        .await
        .unwrap()
        .unwrap_err();
    let crate::AgentLoopError::Llm(error) = error else {
        panic!("LLM error variant must survive")
    };
    assert_eq!(error.message, "provider 'customer-gateway': stream failed");
}
