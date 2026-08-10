// EVE-872: the Framework in-process runtime constructs the canonical resolved
// execution snapshot before turn planning, the host contract returns it from
// `load_resolved_turn`, and no credential or platform-only metadata reaches
// snapshot serialization, Debug output, or emitted event metadata.

use everruns_core::llmsim_driver::LlmSimConfig;
use everruns_core::mcp_server::ScopedMcpServer;
use everruns_core::typed_id::{AgentId, HarnessId, SessionId};
use everruns_core::{DEFAULT_ORG_ID, ResolvedExecutionSnapshot};
use everruns_host::{
    AgentBuilder, HarnessBuilder, InProcessRuntimeBuilder, RuntimeHostAdapter, SessionBuilder,
};

const HEADER_SECRET: &str = "SECRET-MCP-HEADER-MARKER";
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
    harness: everruns_core::Harness,
    agent: everruns_core::Agent,
    session: everruns_core::Session,
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
        .harness_id(harness_id)
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

    let runtime = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("deterministic-ok"))
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
        std::slice::from_ref(&fixture.harness),
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
    assert_eq!(inputs.snapshot.agent_id, Some(fixture.agent.public_id));
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
    }

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

#[tokio::test]
async fn inactive_agent_fails_during_projection_before_execution() {
    let harness_id = HarnessId::from_seed(873);
    let agent_id = AgentId::from_seed(873);
    let session_id = SessionId::from_seed(873);
    let harness = HarnessBuilder::new("orphan", "Prompt.")
        .id(harness_id)
        .build();
    let agent = AgentBuilder::new("archived-agent", "Prompt.")
        .id(agent_id)
        .harness_id(harness_id)
        .status(everruns_core::AgentStatus::Archived)
        .build();
    let session = SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .build();

    let runtime = InProcessRuntimeBuilder::new()
        .llm_sim(LlmSimConfig::fixed("unused"))
        .harness(harness)
        .agent(agent)
        .session(session)
        .build()
        .await
        .expect("runtime builds");

    let error = runtime
        .run_text_turn(session_id, "hello")
        .await
        .expect_err("projection fails before execution");
    assert!(error.to_string().contains("archived"), "{error}");

    // No turn events were emitted: the failure happened during platform
    // projection, before host execution began.
    let events = runtime.events().await.expect("events load");
    assert!(
        events
            .iter()
            .all(|event| !event.event_type.starts_with("turn.")),
        "no turn lifecycle events expected, got {:?}",
        events
            .iter()
            .map(|e| e.event_type.clone())
            .collect::<Vec<_>>()
    );
}
