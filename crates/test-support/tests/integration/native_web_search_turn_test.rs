//! Native web search is decided per turn against the organization's current
//! web search defaults: the turn resolver keeps the `everruns/web_search`
//! driver option only while native comes first in the agent's plan, and adds
//! the organization's blocked domains to it.

use async_trait::async_trait;
use everruns_contracts::CapabilityRef;
use everruns_contracts::driver_registry::{DriverId, DriverRegistry, ProviderConfig};
use everruns_contracts::error::Result;
use everruns_contracts::model_spec::ModelSpec;
use everruns_contracts::native_web_search::{NATIVE_WEB_SEARCH_OPTION, NativeWebSearch};
use everruns_contracts::runtime::capabilities::Capability;
use everruns_contracts::runtime::connection_services::UserConnectionResolver;
use everruns_contracts::runtime::web_search::WebSearchOrgSettings;
use everruns_contracts::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::capabilities::CapabilityRegistry;
use everruns_core::harness_definition::HarnessDefinition;
use everruns_core::host::{
    InMemoryAgentStore, InMemoryHarnessStore, InMemoryProviderStore, InMemorySessionStore,
    StoreTurnContextResolver,
};
use everruns_core::session::{ExecutionSession, SessionExecutionState};
use everruns_core::{AgentDefinition, RuntimeMessage, TurnContextRequest, TurnContextResolver};
use everruns_llmsim::register_driver;
use everruns_test_support::InMemoryMessageRetriever;
use std::sync::Arc;
use uuid::Uuid;

/// Contributes a native search request the way the web search capability does.
struct NativeSearchProbe;

impl Capability for NativeSearchProbe {
    fn id(&self) -> &str {
        "native_search_probe"
    }
    fn name(&self) -> &str {
        "Native search probe"
    }
    fn description(&self) -> &str {
        "Asks for native web search."
    }
    fn driver_options(&self, _config: &serde_json::Value) -> Vec<(String, serde_json::Value)> {
        vec![
            NativeWebSearch {
                provider: "org_default".into(),
                blocked_domains: vec!["x.com".into()],
                ..Default::default()
            }
            .to_driver_option(),
        ]
    }
}

/// An organization whose web search settings are fixed.
struct Settings(Option<WebSearchOrgSettings>);

#[async_trait]
impl UserConnectionResolver for Settings {
    async fn get_connection_token(
        &self,
        _session_id: SessionId,
        _provider: &str,
    ) -> Result<Option<String>> {
        Ok(None)
    }

    async fn get_web_search_settings(
        &self,
        _session_id: SessionId,
    ) -> Result<Option<WebSearchOrgSettings>> {
        Ok(self.0.clone())
    }
}

/// The turn's driver options with `settings` as the organization's (`None`:
/// a host without web search settings).
async fn turn_options(
    settings: Option<Option<WebSearchOrgSettings>>,
) -> std::collections::HashMap<String, serde_json::Value> {
    let harness_store = InMemoryHarnessStore::new();
    let agent_store = InMemoryAgentStore::new();
    let session_store = InMemorySessionStore::new();
    let messages = InMemoryMessageRetriever::new();
    let provider_store = InMemoryProviderStore::new();
    let harness_id = HarnessId::from_seed(7);
    harness_store
        .add_harness(harness_id, HarnessDefinition::new("h", "You help."))
        .await;
    let agent_id = AgentId::from_uuid(Uuid::now_v7());
    agent_store
        .add_agent(AgentDefinition::new(agent_id, "a", "You help."))
        .await;
    let session_id = Uuid::now_v7();
    session_store
        .add_session(ExecutionSession {
            id: session_id.into(),
            workspace_id: everruns_contracts::typed_id::WorkspaceId::from_uuid(session_id),
            organization_id: "default".to_string(),
            harness_id,
            agent_id: Some(agent_id),
            title: None,
            goal: None,
            locale: None,
            tags: vec![],
            status: SessionExecutionState::Started,
            model_id: None,
            capabilities: vec![CapabilityRef::new("native_search_probe")],
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
        })
        .await;
    messages
        .seed(
            session_id.into(),
            vec![RuntimeMessage::user("search, please")],
        )
        .await;
    provider_store
        .set_provider_config(ProviderConfig::new(DriverId::LlmSim).with_api_key("k"))
        .await;
    provider_store
        .set_default_model_spec(ModelSpec::on(DriverId::LlmSim.as_str(), "llmsim-test"))
        .await;
    let mut drivers = DriverRegistry::new();
    register_driver(&mut drivers);
    let mut capabilities = CapabilityRegistry::new();
    capabilities.register(NativeSearchProbe);
    let mut resolver = StoreTurnContextResolver::new(
        Arc::new(harness_store),
        Arc::new(agent_store),
        Arc::new(session_store),
        Arc::new(messages),
        Arc::new(provider_store),
        capabilities,
        drivers,
    );
    if let Some(settings) = settings {
        resolver = resolver.with_web_search_settings(Some(Arc::new(Settings(settings))));
    }
    resolver
        .resolve_turn_context(TurnContextRequest {
            session_id: session_id.into(),
            harness_id,
            agent_id: Some(agent_id),
            mcp_tool_definitions: vec![],
            allow_provider_managed_reduction: true,
        })
        .await
        .expect("turn context resolves")
        .runtime_agent
        .driver_options
}

fn org(defaults: serde_json::Value) -> WebSearchOrgSettings {
    WebSearchOrgSettings {
        defaults: serde_json::from_value(defaults).unwrap(),
        accounts: vec![],
    }
}

#[tokio::test]
async fn org_defaults_decide_native_search_each_turn() {
    // No web search settings on the host: the built-in defaults keep native.
    let options = turn_options(None).await;
    assert!(options.contains_key(NATIVE_WEB_SEARCH_OPTION));

    // A host whose organization has none: built-in defaults too.
    let options = turn_options(Some(None)).await;
    assert!(options.contains_key(NATIVE_WEB_SEARCH_OPTION));

    // The organization allows native: kept, with its blocked domains added.
    let options = turn_options(Some(Some(org(
        serde_json::json!({"blocked_domains": ["pinterest.com"]}),
    ))))
    .await;
    let native = NativeWebSearch::from_driver_options(&options).expect("native kept");
    assert_eq!(native.blocked_domains, ["x.com", "pinterest.com"]);

    // Native turned off, or free search as the default: no native request.
    for defaults in [
        serde_json::json!({"allow_native": false}),
        serde_json::json!({"provider": "free"}),
    ] {
        let options = turn_options(Some(Some(org(defaults.clone())))).await;
        assert!(
            !options.contains_key(NATIVE_WEB_SEARCH_OPTION),
            "{defaults}"
        );
    }
}
