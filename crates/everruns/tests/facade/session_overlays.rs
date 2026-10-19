//! EVE-1299: rebuilding a session's runtime from a trusted Agent keeps the
//! session's persisted, mutable overlay instead of reseeding it from the Agent.
//!
//! Live capability controls on the Engine API arrive with EVE-1297; until then
//! these tests make live changes through the public hosting SPI session
//! mutator, which is the same write `activate_capability` and
//! `deactivate_capability` perform.

use everruns::hosting::spi::{EventReadLimit, EventReadRequest, HostBackends, SessionMutator};
use everruns::{Agent, AgentBuilder, CapabilityRef, Engine, Model, SessionId};
use serde_json::{Value, json};

fn agent(backends: &HostBackends, model: Model) -> AgentBuilder {
    Agent::builder()
        .instructions("Answer briefly.")
        .model(model)
        .backends(backends.clone())
}

/// The downstream regression from EVE-1299, kept as close to the reported
/// program as the in-repo harness allows.
#[tokio::test]
async fn engine_reconstruction_keeps_the_session_capability_overlay() {
    let backends = HostBackends::in_memory();
    let store = backends.session_store.clone();
    let agent = agent(&backends, Model::simulated("retained answer"))
        .build()
        .unwrap();
    let first = Engine::new();
    let session = first.create(agent.clone());
    let result = session.run("retained user request").await.unwrap();
    assert!(result.success);
    let id = session.session_id();
    let history = session.history().page().await.unwrap();
    assert_eq!(history.messages.len(), 2);

    let mut record = store.get_session(id).await.unwrap().expect("cataloged");
    record.title = Some("retained title".into());
    // An inert reference isolates persisted overlay retention from capability
    // execution.
    record
        .capabilities
        .push(CapabilityRef::new("late_extension"));
    store.add_session(record).await.unwrap();
    drop(session);
    drop(first);

    let replacement = Engine::new();
    replacement.attach(id, agent).await.unwrap();
    let resumed = replacement.resume(id).await.unwrap();
    resumed.inspect().await.unwrap();

    let record = store.get_session(id).await.unwrap().expect("resumed");
    assert_eq!(record.title.as_deref(), Some("retained title"));
    assert_eq!(
        resumed.history().page().await.unwrap().messages.len(),
        history.messages.len(),
        "runtime reconstruction must preserve conversation history"
    );
    assert!(
        record
            .capabilities
            .iter()
            .any(|capability| capability.capability_id() == "late_extension"),
        "Engine reconstruction discarded the persisted session capability overlay"
    );
}

#[cfg(feature = "builtins")]
mod registered {
    use super::*;
    use everruns::{LlmSimConfig, ToolCall};

    fn tool_names(context: &everruns::SessionContext) -> Vec<String> {
        context.tools.iter().map(|tool| tool.name.clone()).collect()
    }

    fn overlay_ids(record: &everruns::hosting::spi::ExecutionSession) -> Vec<String> {
        record
            .capabilities
            .iter()
            .map(|capability| capability.capability_id().to_string())
            .collect()
    }

    /// Model that calls `get_current_time` once, then answers.
    fn clock_model() -> Model {
        Model::simulated_with_config(LlmSimConfig::fixed("It is now.").with_tool_call_sequence(
            vec![
                vec![ToolCall {
                    id: "call_clock".into(),
                    name: "get_current_time".into(),
                    arguments: json!({}),
                }],
                vec![],
            ],
        ))
    }

    async fn event_log(backends: &HostBackends, id: SessionId) -> Value {
        let page = backends
            .event_log
            .read_page(EventReadRequest::new(
                id,
                EventReadLimit::new(1000).unwrap(),
            ))
            .await
            .unwrap();
        assert!(page.next_cursor.is_none());
        serde_json::to_value(&page.events).unwrap()
    }

    /// The saved record without the association ids every build re-derives.
    async fn saved_configuration(backends: &HostBackends, id: SessionId) -> Value {
        let record = backends.session_store.get_session(id).await.unwrap();
        let mut value = serde_json::to_value(record.expect("saved")).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("harness_id");
        object.remove("agent_id");
        value
    }

    #[tokio::test]
    async fn hot_enabled_stays_effective_and_disabled_stays_disabled_after_resume() {
        let backends = HostBackends::in_memory();
        let store = backends.session_store.clone();
        let first = Engine::new();
        let session = first.create(agent(&backends, Model::simulated("ok")).build().unwrap());
        session.run("hello").await.unwrap();
        let id = session.session_id();

        // Live changes on the running session: enable two registered
        // capabilities, then disable one of them again.
        store
            .upsert_session_capability(id, CapabilityRef::new("current_time"))
            .await
            .unwrap();
        store
            .upsert_session_capability(id, CapabilityRef::new("stateless_todo_list"))
            .await
            .unwrap();
        store
            .remove_session_capability(id, "stateless_todo_list")
            .await
            .unwrap();
        let live = tool_names(&session.inspect().await.unwrap());
        assert!(live.contains(&"get_current_time".to_string()), "{live:?}");
        assert!(!live.contains(&"write_todos".to_string()), "{live:?}");
        drop(session);
        drop(first);

        let replacement = Engine::new();
        replacement
            .attach(id, agent(&backends, clock_model()).build().unwrap())
            .await
            .unwrap();
        let resumed = replacement.resume(id).await.unwrap();
        let tools = tool_names(&resumed.inspect().await.unwrap());
        assert!(tools.contains(&"get_current_time".to_string()), "{tools:?}");
        assert!(!tools.contains(&"write_todos".to_string()), "{tools:?}");

        // The hot-enabled tool executes after the restart.
        let result = resumed.run("what time is it?").await.unwrap();
        assert!(result.success);
        let events = event_log(&backends, id).await;
        assert!(
            events.as_array().unwrap().iter().any(|event| {
                event["type"] == "tool.completed"
                    && event["data"]["tool_name"] == "get_current_time"
                    && event["data"]["success"] == true
            }),
            "hot-enabled tool did not complete after resume: {events:#}"
        );
        assert_eq!(
            overlay_ids(&store.get_session(id).await.unwrap().unwrap()),
            ["current_time"]
        );
    }

    #[tokio::test]
    async fn repeated_attach_and_inspect_rewrite_neither_history_nor_configuration() {
        let backends = HostBackends::in_memory();
        let store = backends.session_store.clone();
        // One trusted Agent value across restarts: caller-owned in-memory
        // backends keep the environment binding with the Agent value, so the
        // workspace association is reopened rather than re-provisioned.
        let trusted = agent(&backends, clock_model()).build().unwrap();
        let first = Engine::new();
        let session = first.create(trusted.clone());
        let id = session.session_id();
        // Inspection catalogs the session; enable the overlay before the
        // tool-calling turn needs it.
        session.inspect().await.unwrap();
        store
            .upsert_session_capability(id, CapabilityRef::new("current_time"))
            .await
            .unwrap();
        let mut record = store.get_session(id).await.unwrap().unwrap();
        record.title = Some("kept".into());
        record.tags = vec!["triage".into()];
        record.forked_from_session_id = Some(SessionId::new());
        store.add_session(record).await.unwrap();
        assert!(session.run("what time is it?").await.unwrap().success);
        drop(session);
        drop(first);

        let log = event_log(&backends, id).await;
        let configuration = saved_configuration(&backends, id).await;
        let messages = log.as_array().unwrap().len();
        assert!(messages > 0);

        for _ in 0..3 {
            let engine = Engine::new();
            engine.attach(id, trusted.clone()).await.unwrap();
            let resumed = engine.resume(id).await.unwrap();
            resumed.inspect().await.unwrap();
            resumed.inspect().await.unwrap();
            assert_eq!(
                event_log(&backends, id).await,
                log,
                "attach/inspect rewrote the canonical log"
            );
            assert_eq!(
                saved_configuration(&backends, id).await,
                configuration,
                "attach/inspect mutated the saved session configuration"
            );
        }

        // The saved session stays writable on the reconstructed runtime.
        let engine = Engine::new();
        engine.attach(id, trusted).await.unwrap();
        let resumed = engine.resume(id).await.unwrap();
        let before = resumed.history().page().await.unwrap().messages.len();
        assert!(resumed.run("again").await.unwrap().success);
        assert!(resumed.history().page().await.unwrap().messages.len() > before);
        let after = event_log(&backends, id).await;
        assert_eq!(
            after.as_array().unwrap()[..messages],
            log.as_array().unwrap()[..],
            "a later turn must append, never rewrite"
        );
    }

    /// A record seeded by a build before EVE-1299 carried a copy of the
    /// Agent's capabilities in its session layer next to the real overlay.
    #[tokio::test]
    async fn legacy_seeded_record_keeps_its_overlay_and_follows_the_current_agent() {
        let backends = HostBackends::in_memory();
        let store = backends.session_store.clone();
        let v1 = agent(&backends, Model::simulated("ok"))
            .capability("current_time")
            .client_tool("v1_tool", "Old client tool.", json!({"type": "object"}))
            .build()
            .unwrap();
        let first = Engine::new();
        let session = first.create(v1);
        session.run("hello").await.unwrap();
        let id = session.session_id();
        drop(session);
        drop(first);

        let mut record = store.get_session(id).await.unwrap().unwrap();
        let stale_harness = everruns_contracts::typed_id::HarnessId::new();
        let stale_workspace = everruns_contracts::typed_id::WorkspaceId::new();
        record.harness_id = stale_harness;
        record.workspace_id = stale_workspace;
        record.title = Some("legacy title".into());
        record.goal = Some("legacy goal".into());
        record.capabilities = vec![
            CapabilityRef::new("current_time"),
            CapabilityRef::new("stateless_todo_list"),
        ];
        record.mcp_servers.insert(
            "retired".into(),
            serde_json::from_value(json!({"url": "https://retired.invalid/mcp"})).unwrap(),
        );
        store.add_session(record).await.unwrap();

        let v2 = agent(&backends, Model::simulated("ok"))
            .capability("current_time")
            .client_tool("v2_tool", "New client tool.", json!({"type": "object"}))
            .build()
            .unwrap();
        let engine = Engine::new();
        engine.attach(id, v2).await.unwrap();
        let resumed = engine.resume(id).await.unwrap();
        let tools = tool_names(&resumed.inspect().await.unwrap());
        for expected in ["get_current_time", "write_todos", "v2_tool"] {
            assert!(tools.contains(&expected.to_string()), "{tools:?}");
        }
        assert!(!tools.contains(&"v1_tool".to_string()), "{tools:?}");

        let record = store.get_session(id).await.unwrap().unwrap();
        assert_eq!(record.title.as_deref(), Some("legacy title"));
        assert_eq!(record.goal.as_deref(), Some("legacy goal"));
        // The Agent default left the session layer; the overlay stayed.
        assert_eq!(overlay_ids(&record), ["stateless_todo_list"]);
        // Identity associations and Agent configuration are re-derived.
        assert_ne!(record.harness_id, stale_harness);
        assert_ne!(record.workspace_id, stale_workspace);
        assert!(record.mcp_servers.is_empty());
        assert!(resumed.run("still writable").await.unwrap().success);
    }

    #[tokio::test]
    async fn removing_an_agent_default_applies_to_existing_sessions() {
        let backends = HostBackends::in_memory();
        let first = Engine::new();
        let session = first.create(
            agent(&backends, Model::simulated("ok"))
                .capability("current_time")
                .build()
                .unwrap(),
        );
        session.run("hello").await.unwrap();
        let id = session.session_id();
        assert!(tool_names(&session.inspect().await.unwrap()).contains(&"get_current_time".into()));
        drop(session);
        drop(first);

        let engine = Engine::new();
        engine
            .attach(
                id,
                agent(&backends, Model::simulated("ok")).build().unwrap(),
            )
            .await
            .unwrap();
        let resumed = engine.resume(id).await.unwrap();
        let tools = tool_names(&resumed.inspect().await.unwrap());
        assert!(
            !tools.contains(&"get_current_time".to_string()),
            "a stale Agent default was pinned into the session: {tools:?}"
        );
    }
}

/// Host-registered capabilities (EVE-1296) follow the same overlay rules as
/// built-ins: a session-enabled one stays effective after a restart and a
/// session-disabled one stays off.
mod host_registered {
    use super::*;
    use everruns::hosting::services::{Capability, HostServices};

    struct Marker(&'static str, &'static str);

    impl Capability for Marker {
        fn id(&self) -> &str {
            self.0
        }

        fn name(&self) -> &str {
            self.0
        }

        fn description(&self) -> &str {
            "Contributes a marker to the instructions."
        }

        fn system_prompt_addition(&self) -> Option<&str> {
            Some(self.1)
        }
    }

    fn host_agent(backends: &HostBackends) -> Agent {
        agent(backends, Model::simulated("ok"))
            .host_services(
                HostServices::new()
                    .register(Marker("host_kept", "HOST_KEPT_MARKER"))
                    .register(Marker("host_dropped", "HOST_DROPPED_MARKER")),
            )
            .build()
            .unwrap()
    }

    #[tokio::test]
    async fn session_enabled_host_capability_survives_and_disabled_one_stays_off() {
        let backends = HostBackends::in_memory();
        let store = backends.session_store.clone();
        let first = Engine::new();
        let session = first.create(host_agent(&backends));
        let before = session.inspect().await.unwrap().instructions;
        assert!(
            !before.contains("HOST_KEPT_MARKER"),
            "registered is not active"
        );
        let id = session.session_id();
        for capability in ["host_kept", "host_dropped"] {
            store
                .upsert_session_capability(id, CapabilityRef::new(capability))
                .await
                .unwrap();
        }
        store
            .remove_session_capability(id, "host_dropped")
            .await
            .unwrap();
        drop(session);
        drop(first);

        let engine = Engine::new();
        engine.attach(id, host_agent(&backends)).await.unwrap();
        let resumed = engine.resume(id).await.unwrap();
        let instructions = resumed.inspect().await.unwrap().instructions;
        assert!(instructions.contains("HOST_KEPT_MARKER"), "{instructions}");
        assert!(
            !instructions.contains("HOST_DROPPED_MARKER"),
            "{instructions}"
        );
        assert!(resumed.run("still writable").await.unwrap().success);
    }
}
