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

use chrono::Utc;
use everruns_test_support::LlmSimRuntimeExt;
use everruns_test_support::TestMathCapability;

use everruns_core::driver_registry::DriverRegistry;
use everruns_core::provider::DriverId;
use everruns_core::{
    AgentCapabilityConfig, AgentDefinition, CapabilityRegistry, HarnessDefinition,
    PlatformDefinition, ResolvedModel, Session, SessionStatus,
};
use everruns_host::InProcessRuntimeBuilder;
use everruns_test_support::llmsim_driver::LlmSimConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let harness_id = "harness_00000000000000000000000000000071".parse().unwrap();
    let agent_id = "agent_00000000000000000000000000000071".parse().unwrap();
    let session_id = "session_00000000000000000000000000000071".parse().unwrap();

    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);
    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(LlmSimConfig::fixed("Context example"))
        .default_model(ResolvedModel {
            model: "llmsim-model".into(),
            provider_type: DriverId::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
            provider_metadata: None,
        })
        .harness(everruns_host::SeededHarness {
            id: harness_id,
            definition: HarnessDefinition {
                capabilities: vec![AgentCapabilityConfig::new("test_math")],
                ..HarnessDefinition::new("math", "You are a math harness.")
            },
        })
        .agent(AgentDefinition {
            display_name: Some("Math Agent".into()),
            max_iterations: Some(8),
            ..AgentDefinition::new(agent_id, "math-agent", "Use tools when needed.")
        })
        .session(Session {
            source: Default::default(),
            activity: Default::default(),
            id: session_id,
            workspace_id: everruns_core::WorkspaceId::from_uuid((session_id).uuid()),
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            agent_version_id: None,
            agent_identity_id: None,
            owner_principal_id: everruns_core::PrincipalId::from_seed(1),
            resolved_owner_user_id: None,
            owner: None,
            effective_owner: None,
            title: Some("Context Session".into()),
            goal: None,
            locale: Some("en-US".into()),
            preview: None,
            output_preview: None,
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
            status: SessionStatus::Started,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            started_at: None,
            finished_at: None,
            usage: None,
            is_pinned: None,
            active_schedule_count: None,
            features: vec![],
            parent_session_id: None,
            forked_from_session_id: None,
            forked_from_sequence: None,
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
