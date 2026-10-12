#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
// Prompt-prefix stability across drivers.
//
// Every provider cache matches on an exact prefix of the request, so a driver
// that reorders tools, rewrites an earlier message, or puts anything
// call-specific ahead of the conversation silently turns every call into a
// cache miss. Nothing fails when that happens; the bill just goes up. This
// suite pins the property each driver must keep: two consecutive calls of one
// conversation, the second adding an assistant tool call and its result, send
// identical requests apart from the conversation array, and that array only
// grows at the end.
//
// Cache markers (`cache_control`, `cachePoint`) are stripped before comparing:
// they are expected to move to the newest messages, and providers match the
// content around them, not the markers. Each turn gets its own mock server
// answering 400, so the first recorded request is exactly the one the driver
// built, with no retry or fallback in the way.
//
// Bedrock is not here: the AWS SDK owns its request encoding, and the cache
// point placement it is given is covered in `bedrock/cache.rs`.

#[cfg(any(
    feature = "anthropic",
    feature = "openai",
    feature = "gemini",
    feature = "openrouter",
    feature = "mistral"
))]
mod support {
    use everruns_contracts::Provider;
    use everruns_contracts::driver_registry::{
        LlmCallConfig, Message, MessageRole, PromptCacheConfig,
    };
    use everruns_contracts::tool_types::{ToolCall, ToolDefinition};
    use serde_json::Value;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// A call config with caching on, two tools, and a session id, which is
    /// what drivers derive cache keys from.
    pub fn cached_config(model: &str) -> LlmCallConfig {
        let mut config = LlmCallConfig::new(model);
        config.prompt_cache = Some(PromptCacheConfig {
            enabled: true,
            ..Default::default()
        });
        config.tools = vec![
            ToolDefinition::function(
                "read_file",
                "Read a file",
                serde_json::json!({
                    "type": "object",
                    "properties": {"path": {"type": "string"}},
                    "required": ["path"]
                }),
            ),
            ToolDefinition::function(
                "search",
                "Search the web",
                serde_json::json!({
                    "type": "object",
                    "properties": {"query": {"type": "string"}},
                    "required": ["query"]
                }),
            ),
        ];
        config
            .metadata
            .insert("session_id".into(), "session-prefix".into());
        config
    }

    /// Turn one, then turn two: the same conversation plus the model's tool
    /// call and its result.
    pub fn turns() -> (Vec<Message>, Vec<Message>) {
        let one = vec![
            Message::text(MessageRole::System, "You are a careful assistant."),
            Message::text(MessageRole::User, "What is in /tmp/a?"),
            Message::text(MessageRole::Assistant, "Let me check."),
            Message::text(MessageRole::User, "Please read it."),
        ];
        let mut call = Message::text(MessageRole::Assistant, "");
        call.tool_calls = Some(vec![ToolCall {
            id: "call_1".to_string(),
            name: "read_file".to_string(),
            arguments: serde_json::json!({"path": "/tmp/a"}),
        }]);
        let mut result = Message::text(MessageRole::Tool, "hello");
        result.tool_call_id = Some("call_1".to_string());
        let mut two = one.clone();
        two.extend([call, result]);
        (one, two)
    }

    /// The body the driver sent for one call.
    async fn sent(
        provider: impl Fn(&MockServer) -> Provider,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Value {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(400)
                    .insert_header("connection", "close")
                    .set_body_json(serde_json::json!({
                        "error": {"type": "invalid_request_error", "message": "captured"}
                    })),
            )
            .mount(&server)
            .await;
        let _ = provider(&server)
            .chat_completion_stream(messages, config)
            .await;
        let requests = server.received_requests().await.expect("recorded requests");
        let first = requests.first().expect("the driver sent a request");
        serde_json::from_slice(&first.body).expect("json body")
    }

    /// `value` without cache markers, which move between calls by design.
    fn without_cache_markers(value: &Value) -> Value {
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .filter(|(key, _)| *key != "cache_control" && *key != "cachePoint")
                    .map(|(key, value)| (key.clone(), without_cache_markers(value)))
                    .collect(),
            ),
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .filter(|item| item.get("cachePoint").is_none())
                    .map(without_cache_markers)
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// Two calls of one conversation differ only by what `sequence` gained at
    /// its end.
    pub async fn assert_append_only(
        label: &str,
        provider: impl Fn(&MockServer) -> Provider,
        config: &LlmCallConfig,
        sequence: &str,
    ) {
        let (one, two) = turns();
        let first = without_cache_markers(&sent(&provider, one, config).await);
        let second = without_cache_markers(&sent(&provider, two, config).await);

        let first_items = first[sequence]
            .as_array()
            .unwrap_or_else(|| panic!("{label}: no `{sequence}` array in {first}"));
        let second_items = second[sequence].as_array().unwrap();
        assert!(
            second_items.len() > first_items.len(),
            "{label}: turn two should add to `{sequence}`"
        );
        assert_eq!(
            &second_items[..first_items.len()],
            first_items.as_slice(),
            "{label}: turn two rewrote earlier `{sequence}` entries"
        );

        let mut first_rest = first.clone();
        let mut second_rest = second.clone();
        first_rest.as_object_mut().unwrap().remove(sequence);
        second_rest.as_object_mut().unwrap().remove(sequence);
        assert_eq!(
            first_rest, second_rest,
            "{label}: everything outside `{sequence}` must be identical across calls"
        );
    }
}

#[cfg(feature = "anthropic")]
#[tokio::test]
async fn anthropic_requests_grow_only_at_the_end() {
    use everruns_contracts::{Provider, StaticHeaderAuth};
    use everruns_drivers::anthropic::AnthropicChatDriver;

    support::assert_append_only(
        "anthropic",
        |server| {
            Provider::new("anthropic-test", AnthropicChatDriver::new())
                .base_url(format!("{}/v1/messages", server.uri()))
                .auth(StaticHeaderAuth::new("x-api-key", "test-key"))
        },
        &support::cached_config("claude-sonnet-5-5"),
        "messages",
    )
    .await;
}

#[cfg(feature = "openai")]
#[tokio::test]
async fn openai_responses_requests_grow_only_at_the_end() {
    use everruns_contracts::{BearerAuth, Provider};
    use everruns_drivers::openai::OpenAIChatDriver;

    support::assert_append_only(
        "openai responses",
        |server| {
            Provider::new("openai-test", OpenAIChatDriver::new())
                .base_url(format!("{}/v1", server.uri()))
                .auth(BearerAuth::new("test-key"))
        },
        &support::cached_config("gpt-6.1-sol"),
        "input",
    )
    .await;
}

#[cfg(feature = "openai")]
#[tokio::test]
async fn openai_chat_completions_requests_grow_only_at_the_end() {
    use everruns_contracts::{BearerAuth, Provider};
    use everruns_drivers::openai::OpenAICompletionsChatDriver;

    support::assert_append_only(
        "openai chat completions",
        |server| {
            Provider::new(
                "openai-completions-test",
                OpenAICompletionsChatDriver::new(),
            )
            .base_url(format!("{}/v1", server.uri()))
            .auth(BearerAuth::new("test-key"))
        },
        &support::cached_config("gpt-6.1-sol"),
        "messages",
    )
    .await;
}

#[cfg(feature = "mistral")]
#[tokio::test]
async fn mistral_requests_grow_only_at_the_end() {
    support::assert_append_only(
        "mistral",
        |server| {
            everruns_drivers::mistral::provider("mistral-test", "test-key")
                .base_url(format!("{}/v1", server.uri()))
        },
        &support::cached_config("mistral-large-4"),
        "messages",
    )
    .await;
}

#[cfg(feature = "gemini")]
#[tokio::test]
async fn gemini_requests_grow_only_at_the_end() {
    use everruns_contracts::{Provider, StaticHeaderAuth};
    use everruns_drivers::gemini::GeminiChatDriver;

    support::assert_append_only(
        "gemini",
        |server| {
            Provider::new("gemini-test", GeminiChatDriver::new())
                .base_url(server.uri())
                .auth(StaticHeaderAuth::new("x-goog-api-key", "test-key"))
        },
        &support::cached_config("gemini-3-pro"),
        "contents",
    )
    .await;
}

#[cfg(feature = "openrouter")]
#[tokio::test]
async fn openrouter_requests_grow_only_at_the_end() {
    use everruns_contracts::{BearerAuth, Provider};
    use everruns_drivers::openrouter::OpenRouterChatDriver;

    for model in ["anthropic/claude-sonnet-5.5", "openai/gpt-6.1-sol"] {
        support::assert_append_only(
            &format!("openrouter {model}"),
            |server| {
                Provider::new("openrouter-test", OpenRouterChatDriver::new())
                    .base_url(format!("{}/v1/responses", server.uri()))
                    .auth(BearerAuth::new("test-key"))
            },
            &support::cached_config(model),
            "input",
        )
        .await;
    }
}
