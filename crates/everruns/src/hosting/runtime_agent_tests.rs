use super::*;
use serde_json::json;

/// A package compiles to a declaration and reads back unchanged.
#[test]
fn a_package_compiles_and_reads_back() {
    let package = AgentPackage::parse(
        r#"{"name":"support","instructions":"Help.","capabilities":["current_time"]}"#,
        PackageFormat::Json,
    )
    .unwrap();
    let declaration = RuntimeAgent::from_package(&package).unwrap();
    assert_eq!(declaration.format, RUNTIME_AGENT_FORMAT);
    declaration.validate().unwrap();
    let back = declaration.package().unwrap();
    assert_eq!(back.manifest().name, "support");
    assert_eq!(back.manifest().instructions, "Help.");
    assert_eq!(back.manifest().capabilities.len(), 1);
}

/// A stored agent, model or harness id is refused: hosts keep their ids.
#[test]
fn a_declaration_with_stored_ids_is_refused() {
    for key in ["id", "default_model_id", "harness_id"] {
        let mut agent = json!({ "name": "support", "instructions": "Help." });
        let id = match key {
            "id" => "agent_01933b5a000070008000000000000001",
            "default_model_id" => "model_01933b5a000070008000000000000001",
            _ => "harness_01933b5a000070008000000000000001",
        };
        agent[key] = json!(id);
        let error = RuntimeAgent::new(agent).validate().unwrap_err();
        assert!(error.to_string().contains("stored ids"), "{key}: {error}");
    }
}

/// A format this executor does not read is refused before anything binds.
#[test]
fn an_unreadable_format_is_refused() {
    let mut declaration = RuntimeAgent::new(json!({ "name": "support", "instructions": "Help." }));
    declaration.format = RUNTIME_AGENT_FORMATS.end() + 1;
    let error = declaration.validate().unwrap_err();
    assert!(error.to_string().contains("not readable"), "{error}");
}

/// An agent built in code compiles when everything it carries is data, and
/// refuses behavior only code can carry.
#[test]
fn a_code_built_agent_compiles_when_it_is_data() {
    let agent = Agent::builder()
        .name("support")
        .instructions("Help.")
        .model(crate::Model::simulated("ok"))
        .build()
        .unwrap();
    let declaration = RuntimeAgent::from_agent(&agent).unwrap();
    assert_eq!(declaration.package().unwrap().manifest().name, "support");

    let gated = Agent::builder()
        .name("support")
        .instructions("Help.")
        .model(crate::Model::simulated("ok"))
        .on_turn_start(|_| async {})
        .build()
        .unwrap();
    assert!(RuntimeAgent::from_agent(&gated).is_err());
}
