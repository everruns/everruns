// EVE-872: the Framework in-process runtime constructs the canonical resolved
// execution snapshot before turn planning, the host contract returns it from
// `load_resolved_turn`, and no credential or platform-only metadata reaches
// snapshot serialization, Debug output, or emitted event metadata.

use everruns_core::mcp_server::ScopedMcpServer;
use everruns_core::{DEFAULT_ORG_ID, ResolvedExecutionSnapshot};
use everruns_host::{
    AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, RuntimeHostAdapter, SessionBuilder,
};
use everruns_provider::runtime_provider::BearerAuth;
use everruns_provider::typed_id::{AgentId, HarnessId, SessionId};
use everruns_test_support::llmsim_driver::LlmSimConfig;
use everruns_test_support::{LlmSimRuntimeExt, llm_sim_provider};

const HEADER_SECRET: &str = "SECRET-MCP-HEADER-MARKER";
const PROVIDER_SECRET: &str = "SECRET-PROVIDER-KEY-MARKER";
const TITLE_MARKER: &str = "UI-TITLE-MARKER";

fn scoped_server_with_secret_header() -> ScopedMcpServer {
    ScopedMcpServer {
        url: "http://127.0.0.1:9/mcp".into(),
        headers: [(
            "Authorization".to_string(),
            format!("Bearer {HEADER_SECRET}"),
        )]
        .into_iter()
        .collect(),
        // Keep discovery off so the test never touches the network; the
        // scoped server still flows through capability/MCP resolution.
        tool_discovery: false,
        ..Default::default()
    }
}

struct Fixture {
    runtime: everruns_host::InProcessRuntime,
    harness: everruns_host::SeededHarness,
    agent: everruns_core::AgentDefinition,
    session: everruns_core::ExecutionSession,
}

async fn fixture() -> Fixture {
    let harness_id = HarnessId::from_seed(872);
    let agent_id = AgentId::from_seed(872);
    let session_id = SessionId::from_seed(872);

    let harness = HarnessBuilder::new("hoster", "Harness instructions.")
        .id(harness_id)
        .build();
    let agent = AgentBuilder::new("hosted-agent", "Agent instructions.")
        .id(agent_id)
        .max_iterations(7)
        .mcp_servers(
            [("docs".to_string(), scoped_server_with_secret_header())]
                .into_iter()
                .collect(),
        )
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .title(TITLE_MARKER)
        .build();

    let provider = llm_sim_provider(LlmSimConfig::fixed("deterministic-ok"))
        .base_url("https://credential-bearing-host.invalid/v1")
        .auth(BearerAuth::new(PROVIDER_SECRET));
    let runtime = InProcessRuntimeBuilder::new()
        .provider_with_default_model(provider, "llmsim-model")
        .harness(harness.clone())
        .agent(agent.clone())
        .session(session.clone())
        .build()
        .await
        .expect("runtime builds");

    Fixture {
        runtime,
        harness,
        agent,
        session,
    }
}

#[tokio::test]
async fn framework_load_resolved_turn_matches_direct_projection() {
    let fixture = fixture().await;
    let inputs = fixture
        .runtime
        .load_resolved_turn(DEFAULT_ORG_ID, fixture.session.id)
        .await
        .expect("resolved turn loads");

    let direct = ResolvedExecutionSnapshot::project(
        &fixture.harness.definition,
        Some(&fixture.agent),
        &fixture.session,
    )
    .expect("direct projection");

    // Equivalent configuration produces an equivalent snapshot regardless of
    // which path constructed it.
    assert_eq!(
        serde_json::to_value(&inputs.snapshot).unwrap(),
        serde_json::to_value(&direct).unwrap()
    );

    assert_eq!(inputs.snapshot.session_id, fixture.session.id);
    assert_eq!(inputs.snapshot.harness_id, fixture.harness.id);
    assert_eq!(inputs.snapshot.agent_id, Some(fixture.agent.id));
    assert_eq!(inputs.snapshot.max_iterations, Some(7));
    assert_eq!(
        inputs.snapshot.instructions.as_deref(),
        Some("Harness instructions.\n\nAgent instructions.")
    );
    // Scoped MCP config survives as non-secret scope only.
    let docs = inputs.snapshot.mcp_servers.get("docs").expect("docs scope");
    assert_eq!(docs.header_names, vec!["Authorization".to_string()]);
}

#[tokio::test]
async fn turn_execution_and_events_stay_free_of_secrets_and_platform_metadata() {
    let fixture = fixture().await;

    let result = fixture
        .runtime
        .run_text_turn(fixture.session.id, "hello")
        .await
        .expect("turn runs");
    assert!(result.success, "turn failed: {:?}", result.error);
    assert_eq!(result.response, "deterministic-ok");

    // Snapshot surfaces.
    let inputs = fixture
        .runtime
        .load_resolved_turn(DEFAULT_ORG_ID, fixture.session.id)
        .await
        .expect("resolved turn loads");
    let serialized = serde_json::to_string(&inputs.snapshot).unwrap();
    let debugged = format!("{:?}", inputs.snapshot);
    for surface in [serialized.as_str(), debugged.as_str()] {
        assert!(!surface.contains(HEADER_SECRET), "{surface}");
        assert!(!surface.contains(TITLE_MARKER), "{surface}");
        assert!(!surface.contains(PROVIDER_SECRET), "{surface}");
    }

    let assembled = fixture
        .runtime
        .load_context(fixture.session.id)
        .await
        .expect("context loads");
    let assembled_debug = format!("{assembled:?}");
    assert!(!assembled_debug.contains(PROVIDER_SECRET));
    assert!(!assembled_debug.contains("credential-bearing-host.invalid"));
    assert!(assembled_debug.contains("<opaque>"));

    // Event metadata: no emitted event may carry the scoped-MCP credential or
    // platform-only session metadata.
    let events = fixture.runtime.events().await.expect("events load");
    assert!(!events.is_empty());
    for event in events {
        let payload = serde_json::to_string(&event).expect("event serializes");
        assert!(
            !payload.contains(HEADER_SECRET),
            "event leaked MCP credential: {payload}"
        );
        assert!(
            !payload.contains(TITLE_MARKER),
            "event leaked session UI metadata: {payload}"
        );
    }
}

/// EVE-877: archived/deleted lifecycle validation lives at the platform
/// loading seam (`everruns_platform::Agent::execution_definition`), not in the
/// portable definition the embedded host seeds. This adapter stands in for a
/// hosted store whose stored record is archived: the loading seam errors and
/// execution never starts.
#[derive(Clone)]
struct ArchivedAtSeamAgentStore;

#[async_trait::async_trait]
impl everruns_core::execution_loading::AgentStore for ArchivedAtSeamAgentStore {
    async fn get_agent(
        &self,
        agent_id: AgentId,
    ) -> everruns_provider::error::Result<Option<everruns_core::AgentDefinition>> {
        Err(everruns_provider::error::AgentLoopError::config(format!(
            "agent {agent_id} is archived and cannot execute turns"
        )))
    }
}

#[async_trait::async_trait]
impl everruns_host::RuntimeAgentStore for ArchivedAtSeamAgentStore {
    async fn add_agent(
        &self,
        _agent: everruns_core::AgentDefinition,
    ) -> everruns_provider::error::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn inactive_agent_fails_at_loading_seam_before_execution() {
    use everruns_host::{EventReadLimit, EventReadRequest};

    let harness_id = HarnessId::from_seed(873);
    let agent_id = AgentId::from_seed(873);
    let session_id = SessionId::from_seed(873);
    let harness = HarnessBuilder::new("orphan", "Prompt.")
        .id(harness_id)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    // Keep a handle on the session store so the session can be added after
    // build: builder-time seeding also walks the agent loading seam, and this
    // test wants the failure to surface at turn time.
    let session_store = std::sync::Arc::new(everruns_host::InMemorySessionStore::new());
    let backends = everruns_host::HostBackends::in_memory()
        .with_agent_store(std::sync::Arc::new(ArchivedAtSeamAgentStore))
        .with_session_store(session_store.clone());
    let runtime = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("unused"))
        .backends(backends)
        .harness(harness)
        .build()
        .await
        .expect("runtime builds");
    everruns_host::RuntimeSessionStore::add_session(session_store.as_ref(), session)
        .await
        .expect("session seeds");

    let error = runtime
        .run_text_turn(session_id, "hello")
        .await
        .expect_err("loading seam fails before execution");
    assert!(error.to_string().contains("archived"), "{error}");

    // No turn events were emitted: the failure happened at the platform
    // loading seam, before host execution began.
    let page = runtime
        .event_log()
        .read_page(EventReadRequest::new(session_id, EventReadLimit::default()))
        .await
        .expect("events load");
    assert!(
        page.events
            .iter()
            .all(|event| !event.event_type.starts_with("turn.")),
        "no turn lifecycle events expected"
    );
}
