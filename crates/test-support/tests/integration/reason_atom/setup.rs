//! Shared reason-phase fixture construction for regression tests and benchmarks.
use everruns_contracts::driver_registry::{DriverId, ProviderConfig};
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::typed_id::{AgentId, HarnessId};
use everruns_core::AgentDefinition;
use everruns_core::harness_definition::HarnessDefinition;
use everruns_core::host::{
    InMemoryAgentStore, InMemoryHarnessStore, InMemoryProviderStore, InMemorySessionStore,
};
use everruns_core::session::{ExecutionSession, SessionExecutionState};
use everruns_test_support::InMemoryMessageRetriever;
use uuid::Uuid;

pub(crate) type TestEnvironment = (
    InMemoryHarnessStore,
    InMemoryAgentStore,
    InMemorySessionStore,
    InMemoryMessageRetriever,
    InMemoryProviderStore,
    HarnessId,
    Uuid,
    Uuid,
);

pub(crate) async fn set_default_test_model(
    provider_store: &InMemoryProviderStore,
    provider_type: DriverId,
    model: impl Into<String>,
    api_key: Option<&str>,
) {
    let mut config = ProviderConfig::new(provider_type.clone());
    if let Some(api_key) = api_key {
        config = config.with_api_key(api_key);
    }
    provider_store.set_provider_config(config).await;
    provider_store
        .set_default_model_spec(ModelSpec::on(provider_type.as_str(), model.into()))
        .await;
}

/// Create a basic test setup with in-memory stores
pub(crate) async fn setup_test_environment() -> TestEnvironment {
    let harness_store = InMemoryHarnessStore::new();
    let agent_store = InMemoryAgentStore::new();
    let session_store = InMemorySessionStore::new();
    let message_retriever = InMemoryMessageRetriever::new();
    let provider_store = InMemoryProviderStore::new();

    // Create a test harness
    let harness_id = HarnessId::from_seed(1);
    let harness = HarnessDefinition::new("test-harness", "You are a helpful assistant.");
    harness_store.add_harness(harness_id, harness).await;

    // Create a test agent
    let agent_id = Uuid::now_v7();
    let agent = AgentDefinition {
        display_name: Some("Test Agent".to_string()),
        ..AgentDefinition::new(
            AgentId::from_uuid(agent_id),
            "test-agent",
            "You are a helpful assistant.",
        )
    };
    agent_store.add_agent(agent).await;

    // Create a test session
    let session_id = Uuid::now_v7();
    let session = ExecutionSession {
        id: session_id.into(),
        workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid(session_id),
        organization_id: "default".to_string(),
        harness_id,
        agent_id: Some(agent_id.into()),
        title: Some("Test ExecutionSession".to_string()),
        goal: None,
        locale: None,
        tags: vec![],
        status: SessionExecutionState::Started,
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
        usage: None,
        parent_session_id: None,
        forked_from_session_id: None,
        blueprint_id: None,
        blueprint_config: None,
    };
    session_store.add_session(session).await;

    // Set up a default model using the LlmSim provider
    set_default_test_model(
        &provider_store,
        DriverId::LlmSim,
        "llmsim-test",
        Some("fake-api-key"),
    )
    .await;

    (
        harness_store,
        agent_store,
        session_store,
        message_retriever,
        provider_store,
        harness_id,
        agent_id,
        session_id,
    )
}
