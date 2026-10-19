// Recorded tool changes (knowledge/execution/cache-stable-tool-changes.md):
// the first answer of a session stores the baseline tool list, a reveal
// mid-turn is stored once as a redefinition on the answer it preceded, and the
// entries survive the round trip through the event log.
//
// Run with: cargo test -p everruns-test-support --all-features --test integration tool_changes_test::

use async_trait::async_trait;
use serde_json::{Value, json};

use everruns_contracts::tool_changes::ToolChanges;
use everruns_core::builtins::ToolSearchCapability;
use everruns_core::message::{RuntimeMessage, RuntimeMessageRole};
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_llmsim::{LlmSimConfig, SimToolCall, SimTurn};
use everruns_test_support::InMemoryAgenticLoop;

struct WeatherTool;

#[async_trait]
impl Tool for WeatherTool {
    fn name(&self) -> &str {
        "get_weather"
    }
    fn description(&self) -> &str {
        "Current weather for a city."
    }
    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        })
    }
    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::success(json!({ "sky": "clear" }))
    }
}

fn recorded(messages: &[RuntimeMessage]) -> Vec<Option<ToolChanges>> {
    messages
        .iter()
        .filter(|message| message.role == RuntimeMessageRole::Agent)
        .map(|message| {
            message
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.get("tool_changes"))
                .map(|value| serde_json::from_value(value.clone()).expect("tool_changes entry"))
        })
        .collect()
}

#[tokio::test]
async fn a_reveal_is_recorded_once_on_top_of_the_baseline() {
    let sim = LlmSimConfig::scripted(vec![
        SimTurn::ToolCalls(vec![SimToolCall {
            name: "tool_search".to_string(),
            arguments: json!({ "query": "weather" }),
            id: Some("call_search".to_string()),
        }]),
        SimTurn::Assistant("Found it.".to_string()),
        SimTurn::Assistant("Still here.".to_string()),
    ]);
    let runner = InMemoryAgenticLoop::builder()
        .with_llm_sim(sim)
        .capability(ToolSearchCapability::with_threshold(1))
        .tool(WeatherTool)
        .max_iterations(5)
        .build()
        .await
        .expect("build in-memory loop");

    runner
        .run_turn("find a weather tool")
        .await
        .expect("turn 1");
    runner.run_turn("thanks").await.expect("turn 2");

    let entries = recorded(&runner.messages().await.expect("messages"));
    assert_eq!(entries.len(), 3, "three answers: {entries:?}");

    let baseline = entries[0]
        .as_ref()
        .and_then(|entry| entry.baseline.as_ref())
        .expect("first answer stores the baseline");
    let stub = baseline
        .iter()
        .find(|tool| tool.name() == "get_weather")
        .expect("weather tool in the baseline");
    assert!(
        stub.parameters().get("required").is_none(),
        "deferred, so the baseline holds a stub: {stub:?}"
    );

    let reveal = entries[1].as_ref().expect("the reveal is recorded");
    assert!(reveal.baseline.is_none());
    let names: Vec<_> = reveal.changed_names().collect();
    assert_eq!(names, ["get_weather"]);
    assert_eq!(
        reveal.redefined[0].parameters()["required"],
        json!(["city"])
    );

    assert!(entries[2].is_none(), "nothing changed in turn 2");
}
