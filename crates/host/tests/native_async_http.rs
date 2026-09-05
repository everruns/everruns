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
async fn native_continuation_rejection_does_not_retry_statelessly() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":{
            "type":"invalid_request_error", "message":"No tool output found for function call original"}})))
        .expect(1).mount(&server).await;
    let delivery = everruns_provider::native_async::Delivery {
        previous_response_id: "latest".into(),
        call_ids: vec!["original".into()],
        input: vec![json!({"type":"function_call_output","call_id":"original","output":"42"})],
    };
    let provider = Provider::new(
        "fixture",
        OpenAIChatDriver::new().with_native_async_tools(
            NativeAsyncTools::default()
                .function("lookup")
                .continuation(delivery),
        ),
    )
    .base_url(format!("{}/v1", server.uri()))
    .auth(BearerAuth::new("fixture"));
    let mut config = config();
    config.previous_response_id = Some("latest".into());
    assert!(
        provider
            .chat_completion_stream(
                vec![LlmMessage::text(LlmMessageRole::User, "start")],
                &config
            )
            .await
            .is_err()
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
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

type JournalKey = (
    i64,
    everruns_provider::typed_id::SessionId,
    everruns_provider::typed_id::TurnId,
);
type JournalEntry = (
    Option<uuid::Uuid>,
    everruns_provider::native_async::NativeAsyncCheckpoint,
);
#[derive(Default)]
struct RuntimeJournal(tokio::sync::Mutex<std::collections::HashMap<JournalKey, JournalEntry>>);

#[async_trait]
impl everruns_core::native_async_store::NativeAsyncStore for RuntimeJournal {
    async fn acquire(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<everruns_provider::native_async::NativeAsyncCheckpoint> {
        let mut entries = self.0.lock().await;
        let entry = entries
            .entry((lease.org_id, lease.session_id, lease.turn_id))
            .or_default();
        if entry.0.is_some_and(|owner| owner != lease.owner) {
            return Err(everruns_provider::AgentLoopError::store("fenced"));
        }
        entry.0 = Some(lease.owner);
        Ok(entry.1.clone())
    }
    async fn load(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<everruns_provider::native_async::NativeAsyncCheckpoint> {
        let entries = self.0.lock().await;
        let entry = entries
            .get(&(lease.org_id, lease.session_id, lease.turn_id))
            .filter(|entry| entry.0 == Some(lease.owner))
            .ok_or_else(|| everruns_provider::AgentLoopError::store("fenced"))?;
        Ok(entry.1.clone())
    }
    async fn renew(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<()> {
        self.load(lease).await.map(|_| ())
    }
    async fn save(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
        checkpoint: &everruns_provider::native_async::NativeAsyncCheckpoint,
    ) -> Result<()> {
        let mut entries = self.0.lock().await;
        let entry = entries
            .get_mut(&(lease.org_id, lease.session_id, lease.turn_id))
            .filter(|entry| entry.0 == Some(lease.owner))
            .ok_or_else(|| everruns_provider::AgentLoopError::store("fenced"))?;
        entry.1 = checkpoint.clone();
        Ok(())
    }
    async fn release(
        &self,
        lease: everruns_core::native_async_store::NativeAsyncLease,
    ) -> Result<()> {
        let mut entries = self.0.lock().await;
        let entry = entries
            .get_mut(&(lease.org_id, lease.session_id, lease.turn_id))
            .filter(|entry| entry.0 == Some(lease.owner))
            .ok_or_else(|| everruns_provider::AgentLoopError::store("fenced"))?;
        entry.0 = None;
        Ok(())
    }
}

async fn run_native_runtime_case(custom: bool) {
    use everruns_capability::CapabilityRef;
    use everruns_core::CapabilityRegistry;
    use everruns_host::{
        HarnessBuilder, HostBackends, HostComposition, InProcessRuntimeBuilder, SessionBuilder,
    };
    use everruns_provider::{
        driver_registry::DriverRegistry,
        model_spec::ModelSpec,
        typed_id::{HarnessId, SessionId},
    };
    let server = MockServer::start().await;
    let tool_name = if custom { "raw_lookup" } else { "add" };
    let tool_type = if custom { "custom" } else { "function" };
    let output_type = if custom {
        "custom_tool_call_output"
    } else {
        "function_call_output"
    };
    let format = if custom {
        json!({"type":"text"})
    } else {
        serde_json::Value::Null
    };
    let call_item = if custom {
        json!({"type":"custom_tool_call","id":"item_add","call_id":"original_add","name":tool_name,"input":"raw\nquery: 42","async":true})
    } else {
        json!({"type":"function_call","id":"item_add","call_id":"original_add","name":tool_name,"arguments":"{\"a\":20,\"b\":22}","async":true})
    };
    Mock::given(method("POST")).and(path("/v1/responses"))
        .and(body_partial_json(json!({"model":"gpt-6-astra"})))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(sse(vec![
            json!({"type":"response.output_item.done","item":call_item}),
            json!({"type":"response.output_text.delta","delta":"I can continue reasoning."}),
        ],"response_launch"))).up_to_n_times(1).with_priority(10).mount(&server).await;
    Mock::given(method("POST")).and(path("/v1/responses"))
        .and(body_partial_json(json!({"previous_response_id":"response_launch","input":[{"type":output_type,"call_id":"original_add"}]})))
        .respond_with(ResponseTemplate::new(200).insert_header("content-type","text/event-stream").set_body_string(sse(vec![json!({"type":"response.output_text.delta","delta":"The answer is 42."})],"response_final")))
        .expect(1).with_priority(1).mount(&server).await;
    let mut registry = CapabilityRegistry::new();
    registry.register(everruns_test_support::TestMathCapability);
    registry.register(RawLookupCapability);
    registry.register(everruns_builtins::NativeAsyncToolsCapability);
    let journal = Arc::new(RuntimeJournal::default());
    let harness_id = HarnessId::new();
    let session_id = SessionId::new();
    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(HostComposition::new(registry, DriverRegistry::new()))
        .backends(HostBackends::in_memory().with_native_async_store(journal.clone()))
        .provider(
            Provider::new("native", OpenAIChatDriver::new())
                .base_url(format!("{}/v1", server.uri()))
                .auth(BearerAuth::new("fixture")),
        )
        .default_model(ModelSpec::on("native", "gpt-6-astra"))
        .harness(
            HarnessBuilder::new("native", "Use the lookup and finish after its result.")
                .id(harness_id)
                .capability("test_math")
                .capability("fixture_custom")
                .capability(CapabilityRef::with_config(
                    "native_async_tools",
                    json!({"tools":{(tool_name):format}}),
                ))
                .build(),
        )
        .session(SessionBuilder::new(harness_id).id(session_id).build())
        .build()
        .await
        .unwrap();
    let result = runtime
        .run_text_turn(session_id, "What is 20 plus 22?")
        .await
        .unwrap();
    assert!(result.success, "{result:?}");
    assert_eq!(result.response, "The answer is 42.");
    let messages = runtime.messages(session_id).await.unwrap();
    let call = messages
        .iter()
        .flat_map(|message| message.tool_calls())
        .find(|call| call.id == "original_add")
        .unwrap();
    assert!(call.native.as_ref().unwrap().is_async());
    assert_eq!(
        messages
            .iter()
            .filter(|message| message.tool_call_id() == Some("original_add"))
            .count(),
        1
    );
    let entries = journal.0.lock().await;
    let (_, state) = entries.values().next().unwrap();
    assert!(state.can_complete());
    assert_eq!(state.latest_response_id.as_deref(), Some("response_final"));
    assert!(state.host_outcome.is_some());
    assert_eq!(state.host_responses.len(), 2);
    assert_eq!(
        state.host_outcome.as_ref().unwrap()["native_counts"],
        json!({"llm_calls":2,"tool_calls":1})
    );
    let requests = server.received_requests().await.unwrap();
    let initial: serde_json::Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert!(
        initial["tools"]
            .as_array()
            .unwrap()
            .iter()
            .any(|tool| tool["type"] == tool_type
                && tool["name"] == tool_name
                && tool["async"] == true)
    );
    let continuation: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    let output: serde_json::Value =
        serde_json::from_str(continuation["input"][0]["output"].as_str().unwrap()).unwrap();
    assert!(output["error"].is_null(), "{output}");
    if custom {
        assert_eq!(output["result"]["echo"], "raw\nquery: 42");
        assert!(
            matches!(call.native.as_ref().unwrap(), NativeToolCall::Custom { input, .. } if input == "raw\nquery: 42")
        );
    }
    let events = runtime.events().await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == "turn.completed")
            .count(),
        1
    );
}

struct RawLookupCapability;
impl everruns_core::Capability for RawLookupCapability {
    fn id(&self) -> &str {
        "fixture_custom"
    }
    fn name(&self) -> &str {
        "Raw lookup fixture"
    }
    fn description(&self) -> &str {
        "Echo raw lookup input"
    }
    fn tools(&self) -> Vec<Box<dyn everruns_core::Tool>> {
        vec![Box::new(RawLookup)]
    }
}
struct RawLookup;
#[async_trait]
impl everruns_core::Tool for RawLookup {
    fn name(&self) -> &str {
        "raw_lookup"
    }
    fn description(&self) -> &str {
        "Echo raw lookup input"
    }
    fn parameters_schema(&self) -> serde_json::Value {
        json!({"type":"string"})
    }
    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
    }
    async fn execute(&self, arguments: serde_json::Value) -> everruns_core::ToolExecutionResult {
        everruns_core::ToolExecutionResult::success(json!({"echo":arguments}))
    }
}
#[tokio::test]
async fn native_async_normal_runtime_persists_calls_and_waits_for_receipts() {
    run_native_runtime_case(false).await;
    run_native_runtime_case(true).await;
}
