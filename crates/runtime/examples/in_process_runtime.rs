use chrono::Utc;
use everruns_core::capabilities::TestMathCapability;
use everruns_core::llm_driver_registry::DriverRegistry;
use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::{
    Agent, AgentCapabilityConfig, AgentStatus, CapabilityRegistry, Harness, HarnessStatus,
    LlmProviderType, ModelWithProvider, PlatformDefinition, Session, SessionStatus,
};
use everruns_runtime::InProcessRuntimeBuilder;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(TestMathCapability);

    let platform = PlatformDefinition::new(capabilities, DriverRegistry::new());

    let harness_id = "harness_00000000000000000000000000000010".parse()?;
    let agent_id = "agent_00000000000000000000000000000010".parse()?;
    let session_id = "session_00000000000000000000000000000010".parse()?;

    let runtime = InProcessRuntimeBuilder::new()
        .platform_definition(platform)
        .llm_sim(
            LlmSimConfig::fixed("Let me calculate that.").with_tool_call_sequence(vec![
                vec![everruns_core::ToolCall {
                    id: "call_mul_1".into(),
                    name: "multiply".into(),
                    arguments: serde_json::json!({"a": 6, "b": 7}),
                }],
                vec![],
            ]),
        )
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
            description: Some("Minimal embedded harness".into()),
            system_prompt: "You are a math assistant.".into(),
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
            system_prompt: "Use tools when they help.".into(),
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
            title: Some("Embedded Math Session".into()),
            locale: None,
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

    let result = runtime.run_text_turn(session_id, "What is 6 * 7?").await?;

    println!("success: {}", result.success);
    println!("iterations: {}", result.iterations);
    println!("response: {}", result.response);

    Ok(())
}
