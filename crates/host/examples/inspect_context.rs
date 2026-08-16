//! Inspect the assembled turn context of an in-process host without running a
//! turn.
//!
//! Ordinary applications use `Session::inspect` from `everruns`; this example
//! shows the equivalent at the `everruns-host` boundary.
//!
//! Run it:
//!
//! ```text
//! cargo run -p everruns-host --example inspect_context
//! ```

use everruns_host::HostComposition;
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::TestMathCapability;

use everruns_core::{
    AgentDefinition, CapabilityRegistry, ExecutionSession, HarnessDefinition, SessionExecutionState,
};
use everruns_host::InProcessRuntimeBuilder;
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_test_support::llmsim_driver::LlmSimConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let harness_id = "harness_00000000000000000000000000000071".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000071".parse().unwrap();
    let session_id = "session_00000000000000000000000000000071".parse().unwrap();

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    let platform = HostComposition::new(capabilities, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(platform)
        .llm_sim_as_default(LlmSimConfig::fixed("Context example"))
        .default_model(ModelSpec::on((DriverId::LlmSim).as_str(), "llmsim-model"))
        .harness(everruns_host::SeededHarness {
            id: harness_id,
            definition: HarnessDefinition {
                capabilities: vec![everruns_capability::CapabilityRef::new("test_math")],
                ..HarnessDefinition::new("math", "You are a math harness.")
            },
        })
        .agent(AgentDefinition {
            display_name: Some("Math Agent".into()),
            max_iterations: Some(8),
            ..AgentDefinition::new(agent_id, "math-agent", "Use tools when needed.")
        })
        .session(ExecutionSession {
            id: session_id,
            workspace_id: everruns_provider::typed_id::WorkspaceId::from_uuid((session_id).uuid()),
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            title: Some("Context ExecutionSession".into()),
            goal: None,
            locale: Some("en-US".into()),
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            mcp_servers: Default::default(),
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            status: SessionExecutionState::Started,
            usage: None,
            parent_session_id: None,
            forked_from_session_id: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .build()
        .await?;

    let initial = runtime.load_context(session_id).await?;
    println!("before first turn:");
    println!("  model: {}", initial.runtime_agent.model);
    println!("  messages: {}", initial.messages.len());
    println!("  tools:");
    for tool in &initial.runtime_agent.tools {
        println!("    - {}", tool.name());
    }

    runtime.run_text_turn(session_id, "What is 9 * 9?").await?;

    let after_turn = runtime.load_context(session_id).await?;
    println!("after first turn:");
    println!("  model: {}", after_turn.runtime_agent.model);
    println!("  messages: {}", after_turn.messages.len());
    println!("  tools:");
    for tool in &after_turn.runtime_agent.tools {
        println!("  - {}", tool.name());
    }

    Ok(())
}
