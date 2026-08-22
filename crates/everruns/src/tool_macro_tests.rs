//! In-crate tests for `#[everruns::tool]` (EVE-829).
//!
//! These live inside the crate (not `tests/`) so they can read the generated
//! tool's `pub(crate)` schema and drive a scripted tool call through the
//! `pub(crate)` simulator hook. Compile-pass/compile-fail coverage of rejected
//! signatures lives in `tests/ui/` via `trybuild`.

use everruns_core::tools::Tool as CoreTool;
use everruns_provider::tool_types::ToolCall;
use serde_json::{Value, json};

use crate::{Agent, InMemoryEngine, Model};

/// Look up weather for a city. Doc comment becomes the tool description.
#[everruns::tool]
async fn weather(
    city: String,
    #[tool(rename = "unit")] temperature_unit: Option<String>,
) -> Result<Value, String> {
    if city.is_empty() {
        return Err("city must not be empty".to_string());
    }
    Ok(json!({ "city": city, "unit": temperature_unit, "forecast": "sunny" }))
}

/// Explicit options override the defaults; doc text here is ignored.
#[everruns::tool(name = "adder", description = "Add one to a number.")]
async fn add_one(n: i64) -> i64 {
    n + 1
}

/// A tool with no return value.
#[everruns::tool]
async fn record(note: String) {
    let _ = note;
}

#[test]
fn schema_marks_required_optional_and_renamed_arguments() {
    let tool = weather();
    let schema = tool.schema();

    assert_eq!(schema["type"], json!("object"), "schema: {schema}");

    let properties = schema["properties"]
        .as_object()
        .expect("object schema has properties");
    assert!(properties.contains_key("city"), "required arg present");
    assert!(
        properties.contains_key("unit"),
        "renamed arg uses its new name: {schema}"
    );
    assert!(
        !properties.contains_key("temperature_unit"),
        "original name must not leak: {schema}"
    );

    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    assert!(required.contains(&"city"), "city is required: {schema}");
    assert!(
        !required.contains(&"unit"),
        "Option<T> argument is optional: {schema}"
    );
}

#[test]
fn options_override_name_and_description() {
    let tool = add_one();
    assert_eq!(CoreTool::name(&tool), "adder");
    assert_eq!(CoreTool::description(&tool), "Add one to a number.");
}

#[test]
fn doc_comment_becomes_description() {
    let tool = weather();
    assert_eq!(
        CoreTool::description(&tool),
        "Look up weather for a city. Doc comment becomes the tool description."
    );
}

#[tokio::test]
async fn generated_tool_executes_through_agent_builder() {
    let agent = Agent::builder()
        .instructions("Call weather when asked.")
        .model(Model::simulated_scripted(
            "Done.",
            vec![
                vec![ToolCall {
                    id: "call_1".into(),
                    name: "weather".into(),
                    arguments: json!({ "city": "London", "unit": "C" }),
                }],
                vec![],
            ],
        ))
        .tool(weather())
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let turn = session.run("weather in London?").await.expect("turn runs");

    assert!(turn.success, "turn should succeed: {:?}", turn.error);
    assert_eq!(turn.tool_calls, 1, "the generated tool executed");
    assert_eq!(turn.response, "Done.");
}

#[tokio::test]
async fn result_err_becomes_model_visible_tool_error() {
    // The handler returns `Err(..)` for an empty city; the turn recovers rather
    // than panicking, exactly like a hand-written `FunctionTool`.
    let agent = Agent::builder()
        .instructions("Call weather.")
        .model(Model::simulated_scripted(
            "Recovered.",
            vec![
                vec![ToolCall {
                    id: "call_err".into(),
                    name: "weather".into(),
                    arguments: json!({ "city": "" }),
                }],
                vec![],
            ],
        ))
        .tool(weather())
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let turn = session.run("go").await.expect("turn runs");
    assert!(turn.success, "turn recovers from a tool error");
    assert_eq!(turn.tool_calls, 1);
}

#[tokio::test]
async fn unit_return_tool_executes() {
    let agent = Agent::builder()
        .instructions("Call record.")
        .model(Model::simulated_scripted(
            "Logged.",
            vec![
                vec![ToolCall {
                    id: "call_rec".into(),
                    name: "record".into(),
                    arguments: json!({ "note": "hello" }),
                }],
                vec![],
            ],
        ))
        .tool(record())
        .build()
        .expect("valid agent");

    let session = InMemoryEngine::new().create(agent.clone());
    let turn = session.run("record it").await.expect("turn runs");
    assert!(turn.success, "unit-return tool succeeds: {:?}", turn.error);
    assert_eq!(turn.tool_calls, 1);
}

#[tokio::test]
async fn invalid_arguments_surface_as_a_tool_error() {
    // Missing the required `city` field: deserialization fails and the adapter
    // returns a tool error string instead of panicking.
    //
    // This stays model-visible even though handler errors are now redacted. The
    // model wrote these arguments; naming the bad field is what lets it correct
    // its own call, and the message describes the call, not the host.
    let tool = weather();
    match CoreTool::execute(&tool, json!({ "unit": "C" })).await {
        everruns_core::tools::ToolExecutionResult::ToolError(message) => {
            assert!(
                message.contains("invalid arguments for tool `weather`"),
                "unexpected message: {message}"
            );
            assert!(
                message.contains("city"),
                "the model needs the offending field named: {message}"
            );
        }
        other => panic!("expected a tool error, got {other:?}"),
    }
}
