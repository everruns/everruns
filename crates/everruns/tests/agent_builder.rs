//! Compile/build test for the value-first agent API (EVE-832): a clean program
//! describes an agent importing only `everruns::{Agent, Model}` — no `Harness`,
//! hosted `Agent` record, ids, timestamps, registries, or `HostComposition`.

use everruns::{Agent, Model};

#[test]
fn describes_an_agent_with_only_the_facade() {
    let agent = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("Sure."))
        .name("assistant")
        .build()
        .expect("valid agent");

    // The built value is usable and independent of any backend record.
    let _clone = agent.clone();
}

#[test]
fn missing_model_is_a_typed_error() {
    use everruns::BuildError;
    let err = Agent::builder()
        .instructions("You are concise.")
        .build()
        .unwrap_err();
    assert_eq!(err, BuildError::MissingModel);
}

#[cfg(feature = "builtins")]
#[test]
fn invalid_compaction_budget_is_a_typed_error() {
    use everruns::{BuildError, CompactionConfig};

    for invalid in [0.05, 1.5, f32::NAN] {
        let err = Agent::builder()
            .instructions("You are concise.")
            .model(Model::simulated("ok"))
            .capability(CompactionConfig::new().budget_percent(invalid))
            .build()
            .unwrap_err();
        assert!(matches!(err, BuildError::InvalidCapability { ref id, .. } if id == "compaction"));
    }
}

#[test]
fn duplicate_mcp_names_are_a_typed_error() {
    use everruns::{BuildError, McpServer};

    let err = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .mcp_server(McpServer::http("docs", "https://one.invalid/mcp"))
        .mcp_server(McpServer::http("docs", "https://two.invalid/mcp"))
        .build()
        .unwrap_err();
    assert!(matches!(err, BuildError::InvalidMcpServer { .. }));
}

#[test]
fn colliding_sanitized_mcp_names_are_a_typed_error() {
    use everruns::{BuildError, McpServer};

    let err = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .mcp_server(McpServer::http("github-prod", "https://one.invalid/mcp"))
        .mcp_server(McpServer::http("github_prod", "https://two.invalid/mcp"))
        .build()
        .unwrap_err();
    assert!(matches!(err, BuildError::InvalidMcpServer { .. }));
}

#[test]
fn reserved_mcp_name_delimiter_is_a_typed_error() {
    use everruns::{BuildError, McpServer};

    let err = Agent::builder()
        .instructions("You are concise.")
        .model(Model::simulated("ok"))
        .mcp_server(McpServer::http("a__b", "https://one.invalid/mcp"))
        .build()
        .unwrap_err();
    assert!(matches!(err, BuildError::InvalidMcpServer { .. }));
}

// --- backends(): caller-provided host backends replace the in-memory defaults ---------------------
mod custom_backends {
    use everruns::{Agent, BuildError, Engine, HostBackends, Model};
    use everruns_host::{
        EventLog, EventReadLimit, EventReadRequest, EventReader, InMemoryEventLog,
    };
    use std::sync::Arc;

    fn agent(backends: HostBackends) -> Agent {
        Agent::builder()
            .instructions("Reply deterministically.")
            .model(Model::simulated("ok"))
            .backends(backends)
            .build()
            .expect("valid agent")
    }

    #[tokio::test]
    async fn events_land_in_the_provided_event_log() {
        let log = Arc::new(InMemoryEventLog::new());
        let backends = HostBackends::in_memory().with_event_log(log.clone());
        let session = Engine::new().create(agent(backends));
        session.send_and_wait("hi").await.unwrap();
        let page = log
            .read_page(EventReadRequest::new(
                session.session_id(),
                EventReadLimit::new(100).unwrap(),
            ))
            .await
            .unwrap();
        assert!(
            page.events.iter().any(|e| e.event_type == "input.message"),
            "input event recorded in the caller's log: {:?}",
            page.events
                .iter()
                .map(|e| e.event_type.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(log.durability(), everruns_host::EventDurability::Volatile);
    }

    #[tokio::test]
    async fn a_fresh_engine_resumes_from_the_provided_backends() {
        let backends = HostBackends::in_memory();
        let session = Engine::new().create(agent(backends.clone()));
        let session_id = session.session_id();
        session.send_and_wait("first").await.unwrap();
        drop(session);

        // New engine, new Agent value, same backends: the session catalog and
        // history come from the caller's stores, not from engine memory.
        let engine = Engine::new();
        engine.attach(session_id, agent(backends)).await.unwrap();
        let resumed = engine.resume(session_id).await.unwrap();
        assert_eq!(resumed.history().page().await.unwrap().messages.len(), 2);
        assert_eq!(
            resumed.send_and_wait("second").await.unwrap().response,
            "ok"
        );
    }

    #[tokio::test]
    async fn separate_backends_stay_isolated_within_one_engine() {
        let engine = Engine::new();
        let alpha_log = Arc::new(InMemoryEventLog::new());
        let beta_log = Arc::new(InMemoryEventLog::new());
        let alpha = engine.create(agent(
            HostBackends::in_memory().with_event_log(alpha_log.clone()),
        ));
        let beta = engine.create(agent(
            HostBackends::in_memory().with_event_log(beta_log.clone()),
        ));
        alpha.send_and_wait("hello").await.unwrap();
        beta.send_and_wait("world").await.unwrap();

        let limit = EventReadLimit::new(100).unwrap();
        let alpha_id = alpha.session_id();
        let beta_id = beta.session_id();
        assert!(
            !alpha_log
                .read_page(EventReadRequest::new(alpha_id, limit))
                .await
                .unwrap()
                .events
                .is_empty()
        );
        assert!(
            alpha_log
                .read_page(EventReadRequest::new(beta_id, limit))
                .await
                .unwrap()
                .events
                .is_empty()
        );
        assert!(
            !beta_log
                .read_page(EventReadRequest::new(beta_id, limit))
                .await
                .unwrap()
                .events
                .is_empty()
        );
        assert!(
            beta_log
                .read_page(EventReadRequest::new(alpha_id, limit))
                .await
                .unwrap()
                .events
                .is_empty()
        );
    }

    #[cfg(feature = "local")]
    #[test]
    fn backends_and_local_are_mutually_exclusive() {
        let dir = std::env::temp_dir().join(format!("everruns-backends-{}", std::process::id()));
        let err = Agent::builder()
            .instructions("t")
            .model(Model::simulated("ok"))
            .backends(HostBackends::in_memory())
            .local(everruns::LocalConfig::new(dir))
            .build()
            .expect_err("conflict");
        assert_eq!(err, BuildError::ConflictingBackends);
    }
}
