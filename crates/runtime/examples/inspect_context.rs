use chrono::Utc;
use everruns_core::capabilities::TestMathCapability;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llm_models::LlmProviderType;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, Harness, HarnessStatus,
    ModelWithProvider, PlatformDefinition, Session, SessionStatus,
};
use everruns_runtime::InProcessRuntimeBuilder;
use uuid::Uuid;

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
        .default_model(ModelWithProvider {
            model: "llmsim-model".into(),
            provider_type: LlmProviderType::LlmSim,
            api_key: Some("fake-key".into()),
            base_url: None,
        })
        .harness(Harness {
            id: harness_id,
            name: "math".into(),
            display_name: Some("Math".into()),
            description: Some("Minimal harness for context inspection".into()),
            system_prompt: "You are a math harness.".into(),
            parent_harness_id: None,
            default_model_id: None,
            tags: vec![],
            capabilities: vec![AgentCapabilityConfig::new("test_math")],
            initial_files: vec![],
            network_access: None,
            is_built_in: false,
            status: HarnessStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
        })
        .agent(Agent {
            public_id: agent_id,
            internal_id: Uuid::nil(),
            name: "math-agent".into(),
            display_name: Some("Math Agent".into()),
            description: None,
            system_prompt: "Use tools when needed.".into(),
            default_model_id: None,
            tags: vec![],
            capabilities: vec![],
            initial_files: vec![],
            network_access: None,
            max_iterations: Some(8),
            tools: vec![],
            status: AgentStatus::Active,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            archived_at: None,
            deleted_at: None,
            usage: None,
        })
        .session(Session {
            id: session_id,
            organization_id: everruns_core::DEFAULT_ORG_PUBLIC_ID.to_string(),
            harness_id,
            agent_id: Some(agent_id),
            agent_identity_id: None,
            title: Some("Context Session".into()),
            locale: Some("en-US".into()),
            preview: None,
            output_preview: None,
            tags: vec![],
            model_id: None,
            capabilities: vec![],
            tools: vec![],
            system_prompt: None,
            initial_files: vec![],
            hints: None,
            network_access: None,
            max_iterations: None,
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
            subagent_name: None,
            subagent_task: None,
            subagent_status: None,
            blueprint_id: None,
            blueprint_config: None,
        })
        .build()
        .await?;

    runtime.run_text_turn(session_id, "What is 9 * 9?").await?;

    let context = runtime.load_context(session_id).await?;
    println!("model: {}", context.runtime_agent.model);
    println!("messages: {}", context.messages.len());
    println!("tools:");
    for tool in &context.runtime_agent.tools {
        println!("  - {}", tool.name());
    }

    Ok(())
}
