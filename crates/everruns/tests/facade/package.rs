use everruns::{Agent, AgentPackage, Engine, FunctionTool, Model, PackageFormat};
use serde_json::json;

fn example(path: &str) -> String {
    format!(
        "{}/../../examples/agents/{path}",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[tokio::test]
async fn markdown_and_folder_execute_and_export() {
    for path in ["legacy/dad-jokes.md", "triage", "project-review"] {
        let package = AgentPackage::load(example(path)).unwrap();
        let agent = package
            .builder()
            .unwrap()
            .model(Model::simulated("package ran"))
            .build()
            .unwrap();
        let exported = agent.to_package().unwrap();
        assert_eq!(
            exported.manifest().instructions,
            package.manifest().instructions
        );
        assert_eq!(
            exported.manifest().initial_files.len(),
            package.manifest().initial_files.len()
        );
        let session = package.create(&Engine::new(), agent).unwrap();
        let context = session.inspect().await.unwrap();
        if path == "triage" {
            assert!(context.tools.iter().any(|t| t.name == "activate_skill"));
            assert!(context.tools.iter().any(|t| t.name == "read_file"));
        }
        let result = session.send_and_wait("Run the agent").await.unwrap();
        assert!(result.success);
        assert_eq!(result.response, "package ran");
        let dir = tempfile::tempdir().unwrap();
        exported.write_folder(dir.path()).unwrap();
        assert!(
            exported
                .diff(&AgentPackage::load(dir.path()).unwrap())
                .unwrap()
                .is_empty()
        );
    }
}

#[tokio::test]
async fn folder_files_and_skill_scripts_reach_the_runtime() {
    use everruns::{LlmSimConfig, ToolCall};
    let package = AgentPackage::load(example("triage")).unwrap();
    let config = LlmSimConfig::fixed("read files").with_tool_call_sequence(vec![
        vec![ToolCall {
            id: "read-runbook".into(),
            name: "read_file".into(),
            arguments: json!({"path":"/runbook.md"}),
        }],
        vec![ToolCall {
            id: "read-skill-script".into(),
            name: "read_file".into(),
            arguments: json!({"path":"/.agents/skills/investigate/scripts/check.py"}),
        }],
        vec![],
    ]);
    let agent = package
        .builder()
        .unwrap()
        .model(Model::simulated_with_config(config))
        .build()
        .unwrap();
    let session = package.create(&Engine::new(), agent).unwrap();
    assert!(
        session
            .send_and_wait("Read the bundled files")
            .await
            .unwrap()
            .success
    );
    let history = format!("{:?}", session.history().page().await.unwrap().messages);
    assert!(history.contains("Support runbook"), "{history}");
    assert!(history.contains("Optional host-run checklist"), "{history}");
}

#[tokio::test]
async fn custom_tools_export_schema_and_require_matching_binding() {
    let builder = Agent::builder()
        .name("custom")
        .instructions("Use ping")
        .model(Model::simulated("pong"));
    let tool = || {
        FunctionTool::new(
            "ping",
            "Return pong",
            json!({"type":"object","properties":{}}),
            |_: serde_json::Value| async { Ok::<_, String>(json!("pong")) },
        )
    };
    let package = builder.tool(tool()).build().unwrap().to_package().unwrap();
    assert_eq!(package.manifest().tools.len(), 1);
    assert!(package.builder().is_err());
    let agent = package
        .apply_to(
            Agent::builder()
                .tool(tool())
                .model(Model::simulated("bound")),
        )
        .unwrap()
        .build()
        .unwrap();
    assert!(
        Engine::new()
            .create(agent)
            .send_and_wait("ping")
            .await
            .unwrap()
            .success
    );
}

/// A client-side tool travels with its package: the client runs it, so
/// applying the package binds nothing and the agent still declares it.
#[test]
fn client_tools_travel_with_their_package() {
    let parameters = json!({"type":"object","properties":{"binding":{"type":"string"}}});
    let package = Agent::builder()
        .name("page")
        .instructions("Read the page")
        .model(Model::simulated("ok"))
        .client_tool(
            "read_page",
            "Read the page beside the chat.",
            parameters.clone(),
        )
        .build()
        .unwrap()
        .to_package()
        .unwrap();
    let declared = |package: &AgentPackage| {
        package
            .manifest()
            .tools
            .iter()
            .map(|tool| (tool.name().to_owned(), tool.parameters().clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(declared(&package), [("read_page".to_owned(), parameters)]);
    // A package declares host-bound and client-run tools alike: the host
    // says which, or the tool must be bound.
    assert!(
        package
            .apply_to(Agent::builder().model(Model::simulated("ok")))
            .is_err()
    );
    let rebuilt = package
        .apply_to(Agent::builder().model(Model::simulated("ok")).client_tool(
            "read_page",
            "",
            json!({"type":"object"}),
        ))
        .expect("a tool the host runs on the client needs no binding")
        .build()
        .unwrap()
        .to_package()
        .unwrap();
    assert_eq!(declared(&rebuilt), declared(&package));
}

#[test]
fn unknown_runtime_dependencies_fail() {
    let package = AgentPackage::parse(
        "name = 'test'\ninstructions = 'Hi'\ncapabilities = ['missing-capability']",
        PackageFormat::Toml,
    )
    .unwrap();
    assert!(
        package
            .builder()
            .unwrap_err()
            .to_string()
            .contains("host implementation")
    );
    let package = AgentPackage::parse(
        "name = 'test'\ninstructions = 'Hi'\nharness = 'custom'",
        PackageFormat::Toml,
    )
    .unwrap();
    let agent = package
        .builder()
        .unwrap()
        .model(Model::simulated("ok"))
        .build()
        .unwrap();
    assert!(package.create(&Engine::new(), agent).is_err());
}

#[test]
fn resolved_mcp_credentials_are_never_debugged_or_exported() {
    let package = AgentPackage::parse(r#"{"name":"test","instructions":"Hi","mcpServers":{"docs":{"url":"https://example.org/mcp","headers":{"Authorization":"${TOKEN}"}}}}"#, PackageFormat::Json).unwrap().bind_mcp(|_| Some("Bearer private-token".into())).unwrap();
    assert!(!format!("{package:?}").contains("private-token"));
    let exported = package.to_string(PackageFormat::Json).unwrap();
    assert!(!exported.contains("private-token"));
    assert!(exported.contains("${TOKEN}"));
}
