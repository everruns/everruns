//! End-to-end local HTTP fixture: OpenAI driver -> native coordinator -> durable
//! journal -> original call IDs across out-of-order response continuations.
use async_trait::async_trait;
use everruns_engine::native_async::{
    NativeAsyncCoordinator, NativeAsyncExecutor, NativeCallPolicy,
};
use everruns_host::native_async::FileNativeAsyncJournal;
use everruns_openai::{OpenAIChatDriver, async_tools::NativeAsyncTools};
use everruns_provider::{
    BearerAuth, LlmCallConfig, LlmMessage, LlmMessageRole, LlmStreamEvent, Provider, Result,
    native_async::NativeToolCall,
    tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy},
};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::Notify;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

struct Lookups(Arc<Notify>);
#[async_trait]
impl NativeAsyncExecutor for Lookups {
    async fn authorize(&self, _: &NativeToolCall) -> Result<NativeCallPolicy> {
        Ok(NativeCallPolicy {
            allow_async: true,
            replay_safe: true,
            concurrency_class: None,
        })
    }
    async fn execute(&self, call: NativeToolCall) -> Result<String> {
        if call.id() == "original_slow" {
            self.0.notified().await;
        }
        Ok(format!("result for {}", call.id()))
    }
}
fn config() -> LlmCallConfig {
    LlmCallConfig {
        model: "gpt-6-astra".into(),
        temperature: None,
        max_tokens: None,
        tools: ["lookup", "raw_lookup"]
            .into_iter()
            .map(|name| {
                ToolDefinition::Builtin(BuiltinTool {
                    name: name.into(),
                    display_name: None,
                    description: "Read-only lookup".into(),
                    parameters: json!({"type":"object","properties":{}}),
                    policy: ToolPolicy::Auto,
                    category: None,
                    deferrable: DeferrablePolicy::Never,
                    hints: ToolHints::default().with_readonly(true),
                    full_parameters: None,
                })
            })
            .collect(),
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        metadata: Default::default(),
        previous_response_id: None,
        provider_opaque_context: None,
        tool_search: None,
        prompt_cache: None,
        openrouter_routing: None,
        parallel_tool_calls: Some(true),
        volatile_suffix_len: 0,
        extra_headers: vec![],
        cache_diagnostics: None,
    }
}
fn sse(items: Vec<serde_json::Value>, id: &str) -> String {
    let mut result = items
        .into_iter()
        .map(|item| format!("data: {item}\n\n"))
        .collect::<String>();
    result.push_str(&format!(
        "data: {}\n\n",
        json!({"type":"response.completed","response":{"id":id,"status":"completed","output":[]}})
    ));
    result
}

#[tokio::test]
async fn native_async_http_delivers_out_of_order_to_latest_response() {
    let server = MockServer::start().await;
    let initial = sse(
        vec![
            json!({"type":"response.output_item.done","item":{"type":"function_call","call_id":"original_slow","id":"slow","name":"lookup","arguments":"{}","async":true}}),
            json!({"type":"response.output_item.done","item":{"type":"custom_tool_call","call_id":"original_fast","id":"fast","name":"raw_lookup","input":"raw query","async":true}}),
            json!({"type":"response.output_text.delta","delta":"independent work"}),
        ],
        "launch",
    );
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .and(body_partial_json(
            json!({"input":[{"type":"message","role":"user","content":"start"}]}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_raw(initial, "text/event-stream"))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .and(body_partial_json(json!({"previous_response_id":"launch","input":[{"type":"custom_tool_call_output","call_id":"original_fast","output":"result for original_fast"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse(vec![],"fast_receipt"),"text/event-stream")).expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .and(body_partial_json(json!({"previous_response_id":"fast_receipt","input":[{"type":"function_call_output","call_id":"original_slow","output":"result for original_slow"}]})))
        .respond_with(ResponseTemplate::new(200).set_body_raw(sse(vec![],"final_receipt"),"text/event-stream")).expect(1).mount(&server).await;
    let directory = tempfile::tempdir().unwrap();
    let journal = FileNativeAsyncJournal::open(directory.path()).unwrap();
    let release = Arc::new(Notify::new());
    let mut coordinator = NativeAsyncCoordinator::open(
        Box::new(journal),
        Arc::new(Lookups(release.clone())),
        2,
        true,
    )
    .await
    .unwrap();
    let mut text = String::new();
    let mut requests = 0;
    let url = format!("{}/v1", server.uri());
    coordinator
        .run(
            4,
            move |delivery, latest| {
                requests += 1;
                let mut options = NativeAsyncTools::default()
                    .function("lookup")
                    .custom("raw_lookup", json!({"type":"text"}));
                if let Some(delivery) = delivery {
                    options = options.continuation(delivery);
                }
                let provider = Provider::new(
                    "test",
                    OpenAIChatDriver::new().with_native_async_tools(options),
                )
                .base_url(url.clone())
                .auth(BearerAuth::new("fixture-key"));
                let mut config = config();
                config.previous_response_id = latest;
                let release = release.clone();
                let index = requests;
                async move {
                    let stream = provider
                        .chat_completion_stream(
                            vec![LlmMessage::text(LlmMessageRole::User, "start")],
                            &config,
                        )
                        .await?;
                    if index == 2 {
                        release.notify_one();
                    }
                    Ok(stream)
                }
            },
            |event| {
                if let LlmStreamEvent::TextDelta(delta) = event {
                    text.push_str(&delta);
                }
            },
        )
        .await
        .unwrap();
    assert_eq!(text, "independent work");
    assert!(coordinator.checkpoint().can_complete());
    assert_eq!(
        coordinator.checkpoint().latest_response_id.as_deref(),
        Some("final_receipt")
    );
    let requests = server.received_requests().await.unwrap();
    let first: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        first["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["async"] == true)
    );
    drop(coordinator);
    use everruns_engine::native_async::NativeAsyncJournal;
    let restored = FileNativeAsyncJournal::open(directory.path())
        .unwrap()
        .load()
        .await
        .unwrap();
    assert!(restored.can_complete());
}
