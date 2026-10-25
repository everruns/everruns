//! Live acceptance of native async tool calls (function and custom) on the
//! OpenAI Responses API with Astra: the coordinator, its durable journal and
//! the original call ids across response continuations. The local HTTP fixture
//! suite is `crates/everruns/tests/host/integration/native_async_http.rs`.

use async_trait::async_trait;
use everruns_contracts::{
    BearerAuth, LlmCallConfig, LlmStreamEvent, Message, MessageRole, Provider, Result,
    native_async::NativeToolCall,
    tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy},
};
use everruns_core::engine::native_async::{
    NativeAsyncCoordinator, NativeAsyncExecutor, NativeCallPolicy,
};
use everruns_core::host::native_async::FileNativeAsyncJournal;
use everruns_drivers::openai::{OpenAIChatDriver, async_tools::NativeAsyncTools};
use serde_json::json;
use std::sync::Arc;

fn config() -> LlmCallConfig {
    let mut config = LlmCallConfig::new("gpt-6-astra");
    config.tools = ["lookup", "raw_lookup"]
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
        .collect();
    config.parallel_tool_calls = Some(true);
    config
}
struct LiveLookups(std::sync::Mutex<std::collections::BTreeMap<String, NativeToolCall>>);
#[async_trait]
impl NativeAsyncExecutor for LiveLookups {
    async fn authorize(&self, call: &NativeToolCall) -> Result<NativeCallPolicy> {
        assert!(matches!(call.name(), "lookup" | "raw_lookup"));
        Ok(NativeCallPolicy {
            allow_async: true,
            replay_safe: true,
            concurrency_class: None,
        })
    }
    async fn execute(&self, call: NativeToolCall) -> Result<String> {
        let output = format!("result for {}", call.id());
        assert!(
            self.0
                .lock()
                .unwrap()
                .insert(call.id().to_owned(), call)
                .is_none(),
            "call must execute once"
        );
        Ok(output)
    }
}

/// Bounded synthetic acceptance: no repository content is sent to the provider.
#[tokio::test]
async fn native_async_astra_live_function_and_custom_calls() {
    let key = std::env::var("OPENAI_API_KEY").expect("OPENAI_API_KEY is required");
    let directory = tempfile::tempdir().unwrap();
    let executor = Arc::new(LiveLookups(std::sync::Mutex::new(Default::default())));
    let mut coordinator = NativeAsyncCoordinator::open(
        Box::new(FileNativeAsyncJournal::open(directory.path()).unwrap()),
        executor.clone(),
        2,
        true,
    )
    .await
    .unwrap();
    let mut text = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(90), coordinator.run(
        4,
        move |delivery, latest| {
            let mut options = NativeAsyncTools::default().function("lookup")
                .custom("raw_lookup", json!({"type":"text"}));
            if let Some(delivery) = delivery { options = options.continuation(delivery); }
            let provider = Provider::new("live", OpenAIChatDriver::new().with_native_async_tools(options))
                .base_url("https://api.openai.com/v1").auth(BearerAuth::new(key.clone()));
            let mut config = config();
            config.max_tokens = Some(1000);
            config.reasoning_effort = Some(everruns_contracts::ReasoningEffort::Low);
            config.previous_response_id = latest;
            async move {
                provider.chat_completion_stream(vec![Message::text(MessageRole::User,
                    "This is a synthetic demo. Call lookup with {} and raw_lookup with exactly Paris demo. Both return fixed demo data. While they run, name three packing essentials. After both results arrive, report both returned outputs. Do not call either tool more than once.")], &config).await
            }
        },
        |event| if let LlmStreamEvent::TextDelta(delta) = event { text.push_str(&delta); },
    )).await.expect("bounded live acceptance timed out").unwrap();
    let state = coordinator.checkpoint();
    assert!(state.can_complete(), "all outputs need provider receipts");
    assert_eq!(state.calls.len(), 2, "both requested tools must run once");
    assert!(state.calls.values().all(|pending| pending.call.is_async()));
    assert!(state.calls.values().any(
        |pending| matches!(&pending.call, NativeToolCall::Function {name, ..} if name == "lookup")
    ));
    assert!(state.calls.values().any(|pending| matches!(&pending.call, NativeToolCall::Custom {name, input, ..} if name == "raw_lookup" && !input.is_empty())));
    for (id, pending) in &state.calls {
        assert_eq!(
            executor.0.lock().unwrap().get(id),
            Some(&pending.call),
            "persisted raw call must match executor input exactly"
        );
        assert!(
            text.contains(&format!("result for {id}")),
            "final response must use each original-call output"
        );
    }
    println!(
        "live Astra: function/custom calls delivered; {} responses; all original-call outputs reflected",
        state.completed_responses
    );
}
