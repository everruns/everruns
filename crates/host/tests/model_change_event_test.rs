//! `session.model.changed` is emitted by the host, so every runtime records a
//! mid-session model switch — the in-process framework runtime included, not
//! only the server's message-create path.

use std::sync::Arc;

use everruns_core::events::EventData;
use everruns_core::message::{ContentPart, Controls, MessageRole};
use everruns_core::message_retriever::InputMessage;
use everruns_core::{AgentDefinition, CapabilityRegistry, ExecutionSession};
use everruns_host::HostComposition;
use everruns_host::{
    AgentBuilder, HarnessBuilder, HostBackends, InMemoryProviderStore, InProcessRuntime,
    InProcessRuntimeBuilder, SessionBuilder,
};
use everruns_llmsim::LlmSimRuntimeExt;
use everruns_llmsim::{LlmSimConfig, SimTurn};
use everruns_provider::driver_registry::DriverRegistry;
use everruns_provider::model_spec::ModelSpec;
use everruns_provider::provider::DriverId;
use everruns_provider::typed_id::{AgentId, HarnessId, ModelId, SessionId};

fn agent(agent_id: AgentId) -> AgentDefinition {
    AgentBuilder::new("chat-agent", "Answer briefly.")
        .id(agent_id)
        .build()
}

fn session(session_id: SessionId, harness_id: HarnessId, agent_id: AgentId) -> ExecutionSession {
    SessionBuilder::new(harness_id)
        .id(session_id)
        .agent(agent_id)
        .title("Model change session")
        .build()
}

fn message(text: &str, model_id: Option<ModelId>) -> InputMessage {
    InputMessage {
        role: MessageRole::User,
        content: vec![ContentPart::text(text)],
        controls: model_id.map(|model_id| Controls {
            model_id: Some(model_id),
            ..Default::default()
        }),
        metadata: None,
        tags: vec![],
    }
}

/// Runtime with two selectable models, both served by the simulator.
async fn runtime_with_two_models(seed: u128) -> (InProcessRuntime, SessionId, ModelId, ModelId) {
    let harness_id = HarnessId::from_seed(seed);
    let agent_id = AgentId::from_seed(seed);
    let session_id = SessionId::from_seed(seed);
    let sol = ModelId::from_seed(seed);
    let terra = ModelId::from_seed(seed + 1);

    let provider_store = Arc::new(InMemoryProviderStore::new());
    let provider = DriverId::LlmSim.as_str();
    provider_store
        .add_model(sol, ModelSpec::on(provider, "sim-sol"))
        .await;
    provider_store
        .add_model(terra, ModelSpec::on(provider, "sim-terra"))
        .await;

    let mut backends = HostBackends::in_memory();
    backends.provider_store = provider_store;

    let runtime = InProcessRuntimeBuilder::new()
        .host_composition(HostComposition::new(
            CapabilityRegistry::new(),
            DriverRegistry::new(),
        ))
        .backends(backends)
        .harness(
            HarnessBuilder::new("chat", "You are helpful.")
                .id(harness_id)
                .build(),
        )
        .agent(agent(agent_id))
        .session(session(session_id, harness_id, agent_id))
        .llm_sim_as_default(LlmSimConfig::scripted(vec![
            SimTurn::Assistant("one".to_string()),
            SimTurn::Assistant("two".to_string()),
            SimTurn::Assistant("three".to_string()),
        ]))
        .default_model(ModelSpec::on(provider, "sim-sol"))
        .build()
        .await
        .expect("runtime builds");

    (runtime, session_id, sol, terra)
}

async fn model_changes(runtime: &InProcessRuntime) -> Vec<everruns_core::SessionModelChangedData> {
    runtime
        .events()
        .await
        .expect("events")
        .into_iter()
        .filter_map(|event| match event.data {
            EventData::SessionModelChanged(data) => Some(data),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn in_process_turn_records_a_model_switch() {
    let (runtime, session_id, sol, terra) = runtime_with_two_models(4_101).await;

    runtime
        .run_turn(session_id, message("first", Some(sol)))
        .await
        .expect("first turn runs");
    assert!(
        model_changes(&runtime).await.is_empty(),
        "the first message establishes the model, it does not change it"
    );

    runtime
        .run_turn(session_id, message("second", Some(terra)))
        .await
        .expect("second turn runs");
    runtime
        .run_turn(session_id, message("third", Some(terra)))
        .await
        .expect("third turn runs");

    let changes = model_changes(&runtime).await;
    assert_eq!(changes.len(), 1, "only the switch itself is an event");
    assert_eq!(changes[0].previous_model_id, Some(sol));
    assert_eq!(changes[0].previous_model_name.as_deref(), Some("sim-sol"));
    assert_eq!(changes[0].model_id, terra);
    assert_eq!(changes[0].model_name, "sim-terra");
}

#[tokio::test]
async fn a_turn_without_an_override_reports_no_switch() {
    let (runtime, session_id, sol, _terra) = runtime_with_two_models(4_102).await;

    runtime
        .run_turn(session_id, message("first", Some(sol)))
        .await
        .expect("first turn runs");
    // An inherited default cannot be named here, so it must not be reported as
    // a switch away from the explicit model.
    runtime
        .run_turn(session_id, message("second", None))
        .await
        .expect("second turn runs");

    assert!(model_changes(&runtime).await.is_empty());
}
