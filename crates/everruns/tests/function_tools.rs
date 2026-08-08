//! Consumer-facing test for function tools (EVE-828).
//!
//! Proves the "done when" criterion: a library user registers a custom tool by
//! passing an async closure, importing only the `everruns` facade — never
//! `CapabilityRegistry`, `ToolDefinition`, `ToolContextServices`, or
//! `PlatformStore`.

use everruns::{Agent, BuildError, FunctionTool, Model, ToolResponse};
use serde_json::{Value, json};

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "city": { "type": "string" } },
        "required": ["city"],
    })
}

#[test]
fn registers_function_and_capability_tools_via_public_api() {
    let weather = FunctionTool::new(
        "get_weather",
        "Look up the weather for a city.",
        schema(),
        |args: Value| async move {
            let city = args["city"].as_str().unwrap_or("unknown");
            if city.is_empty() {
                return Ok(ToolResponse::error("city must not be empty"));
            }
            Ok::<_, String>(ToolResponse::json(json!({ "city": city, "sky": "clear" })))
        },
    );

    // A function tool and a capability-referenced tool coexist on the same
    // builder, with no core/registry types in sight.
    let agent = Agent::builder()
        .instructions("You are a helpful weather assistant.")
        .model(Model::simulated("Clear skies."))
        .tool("test_math")
        .tool(weather)
        .build();

    assert!(
        agent.is_ok(),
        "public-API registration must build: {agent:?}"
    );
}

#[test]
fn duplicate_tool_names_are_rejected_at_build() {
    let make = || {
        FunctionTool::new("lookup", "desc", schema(), |_: Value| async move {
            Ok::<_, String>(json!({}))
        })
    };

    let err = Agent::builder()
        .instructions("Concise.")
        .model(Model::simulated("ok"))
        .tool(make())
        .tool(make())
        .build()
        .expect_err("duplicate names must fail");

    assert_eq!(
        err,
        BuildError::DuplicateTool {
            name: "lookup".to_string()
        }
    );
}
