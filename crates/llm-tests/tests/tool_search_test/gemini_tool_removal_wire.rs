//! Owned cache invariants through the real runtime and Gemini request serializer.
//! Loopback SSE replies deliberately omit implicit-cache usage: prefix stability
//! and execution authorization must hold even when Google reports no cache hit.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use everruns_contracts::driver_registry::{DriverId, DriverRegistry, ProviderConfig};
use everruns_contracts::model_spec::ModelSpec;
use everruns_core::builtins::PromptCachingCapability;
use everruns_core::capabilities::Capability;
use everruns_core::events::{EventData, LLM_GENERATION, TOOL_COMPLETED};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_test_support::TestMathCapability;
use everruns_test_support::in_memory_loop::InMemoryAgenticLoop;
use serde_json::{Value, json, value::RawValue};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct SwitchedTool {
    on: Arc<AtomicBool>,
    executions: Arc<AtomicUsize>,
}

impl Capability for SwitchedTool {
    fn id(&self) -> &str {
        "switched_tool"
    }
    fn name(&self) -> &str {
        "Switched tool"
    }
    fn description(&self) -> &str {
        "A counting tool until switched off."
    }
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        if self.on.load(Ordering::SeqCst) {
            vec![Box::new(CountingTool(self.executions.clone()))]
        } else {
            vec![]
        }
    }
}

struct CountingTool(Arc<AtomicUsize>);

#[async_trait::async_trait]
impl Tool for CountingTool {
    fn name(&self) -> &str {
        "counted_tool"
    }
    fn description(&self) -> &str {
        "Count executions."
    }
    fn parameters_schema(&self) -> Value {
        json!({"type": "object", "properties": {}, "additionalProperties": false})
    }
    async fn execute(&self, _: Value) -> ToolExecutionResult {
        self.0.fetch_add(1, Ordering::SeqCst);
        ToolExecutionResult::success(json!("executed"))
    }
}

#[tokio::test]
async fn held_gemini_removal_preserves_wire_prefix_but_refuses_execution() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/models/gemini-2.5-flash:streamGenerateContent"))
        .and(query_param("alt", "sse"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            let last = body["contents"].as_array().unwrap().last().unwrap();
            let part = if last["parts"][0].get("functionResponse").is_some() {
                json!({"text": "done"})
            } else {
                // Both turns request the same tool, including after its removal.
                json!({"functionCall": {"name": "counted_tool", "args": {}}})
            };
            ResponseTemplate::new(200)
                .insert_header("connection", "close")
                .set_body_raw(
                    format!(
                        "data: {}\n\n",
                        json!({"candidates": [{"content": {"role": "model", "parts": [part]}, "finishReason": "STOP"}]})
                    ),
                    "text/event-stream",
                )
        })
        .mount(&server)
        .await;

    let on = Arc::new(AtomicBool::new(true));
    let executions = Arc::new(AtomicUsize::new(0));
    let mut drivers = DriverRegistry::new();
    everruns_drivers::gemini::register_driver(&mut drivers);
    let runner = InMemoryAgenticLoop::builder()
        .system_prompt("Keep this system instruction stable.")
        .model((
            ModelSpec::on("gemini", "gemini-2.5-flash"),
            ProviderConfig::new(DriverId::Gemini)
                .with_api_key("loopback-only")
                .with_base_url(server.uri()),
        ))
        .driver_registry(drivers)
        .capability(TestMathCapability)
        .capability(SwitchedTool {
            on: on.clone(),
            executions: executions.clone(),
        })
        .capability(PromptCachingCapability::new())
        .max_iterations(3)
        .build()
        .await
        .unwrap();

    let first = runner.run_turn("First turn").await.unwrap();
    assert!(first.success, "{:?}", first.error);
    assert_eq!(first.tool_calls_count, 1);
    assert_eq!(executions.load(Ordering::SeqCst), 1);
    let before = runner.events_by_type(LLM_GENERATION).await.len();
    on.store(false, Ordering::SeqCst);
    let second = runner.run_turn("Second turn").await.unwrap();
    assert!(second.success, "{:?}", second.error);
    assert_eq!(second.tool_calls_count, 1);
    assert_eq!(executions.load(Ordering::SeqCst), 1, "removed tool ran");

    let completed = runner.events_by_type(TOOL_COMPLETED).await;
    let EventData::ToolCompleted(refusal) = &completed.last().unwrap().data else {
        panic!("expected tool completion");
    };
    assert_eq!(refusal.tool_name, "counted_tool");
    assert!(!refusal.success);
    assert_eq!(
        refusal.error.as_deref(),
        Some("Tool definition not found: counted_tool")
    );

    let generations = runner.events_by_type(LLM_GENERATION).await;
    for event in &generations[before..] {
        let EventData::LlmGeneration(data) = &event.data else {
            panic!("expected generation");
        };
        assert!(data.metadata.tool_changes.is_none(), "removal was released");
    }

    let requests = server.received_requests().await.unwrap();
    let first: HashMap<String, Box<RawValue>> =
        serde_json::from_slice(&requests.first().unwrap().body).unwrap();
    let tools: Value = serde_json::from_str(first["tools"].get()).unwrap();
    let names: Vec<_> = tools[0]["functionDeclarations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        ["add", "subtract", "multiply", "divide", "counted_tool"]
    );
    assert!(
        first["systemInstruction"]
            .get()
            .contains("Keep this system instruction stable.")
    );
    let initial_contents: Vec<Value> = serde_json::from_str(first["contents"].get()).unwrap();
    let mut saw_second_turn = false;
    for request in &requests[1..] {
        let body: HashMap<String, Box<RawValue>> = serde_json::from_slice(&request.body).unwrap();
        // Compare the actual serialized field bytes, including order and schemas.
        for field in ["systemInstruction", "tools"] {
            assert_eq!(body[field].get(), first[field].get(), "rewrote {field}");
        }
        let contents: Vec<Value> = serde_json::from_str(body["contents"].get()).unwrap();
        assert_eq!(
            &contents[..initial_contents.len()],
            initial_contents.as_slice()
        );
        saw_second_turn |= contents
            .iter()
            .any(|item| item["parts"][0]["text"] == "Second turn");
    }
    assert!(saw_second_turn, "did not capture a request after removal");
}
