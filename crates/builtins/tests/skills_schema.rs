#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! `activate_skill` must accept `arguments: null`: models often send it for
//! skills that take no arguments.

use everruns_builtins::SkillsCapability;
use everruns_core::Capability;

fn arguments_type(schema: &serde_json::Value) -> &serde_json::Value {
    &schema["properties"]["arguments"]["type"]
}

#[test]
fn activate_skill_arguments_schema_is_nullable() {
    let nullable = serde_json::json!(["string", "null"]);

    let def = SkillsCapability
        .tool_definitions()
        .into_iter()
        .find(|d| d.name() == "activate_skill")
        .expect("activate_skill definition");
    assert_eq!(arguments_type(def.parameters()), &nullable);

    let tool = SkillsCapability
        .tools()
        .into_iter()
        .find(|t| t.name() == "activate_skill")
        .expect("activate_skill tool");
    assert_eq!(arguments_type(&tool.parameters_schema()), &nullable);
}
