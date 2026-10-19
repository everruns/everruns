//! Host provider-request metadata through Engine composition (EVE-1302).
//! Imports only the public `everruns` API and records the metadata on the
//! actual provider request, not a host-held copy.
#![allow(deprecated)] // Exercises the legacy custom-backend lane until its retirement (EVE-1298).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use everruns::hosting::services::{HostServices, RequestMetadata};
use everruns::hosting::spi::HostBackends;
use everruns::hosting::{AgentBuild, MemoryHosting};
use everruns::llm::Message;
use everruns::{
    Agent, AgentLoopError, BuildError, ChatDriver, Engine, LlmCallConfig, LlmResponseStream,
    LlmStreamEvent, Provider, ProviderEndpoint, Session, SessionId,
};

const MODEL: &str = "recorded-model";

/// The five values Yolop attaches through the Core harness today.
fn yolop_metadata() -> RequestMetadata {
    RequestMetadata::new()
        .entry("app", "yolop")
        .entry("yolop_version", "0.42.0")
        .entry("everruns_core_version", "0.1.0")
        .openrouter_attribution("https://github.com/everruns/yolop", "Yolop")
}

fn expected() -> BTreeMap<String, String> {
    yolop_metadata()
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

/// Model and metadata of one provider request.
type Request = (String, HashMap<String, String>);

/// Records the model and metadata of every provider request.
#[derive(Clone, Default)]
struct Recorder(Arc<Mutex<Vec<Request>>>);

impl Recorder {
    fn last(&self) -> Request {
        self.0.lock().unwrap().last().cloned().expect("a request")
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }
}

#[async_trait]
impl ChatDriver for Recorder {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        _messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        self.0
            .lock()
            .unwrap()
            .push((config.model.clone(), config.metadata.clone()));
        Ok(Box::pin(futures::stream::iter([
            Ok(LlmStreamEvent::TextDelta("recorded".to_string())),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

fn agent(recorder: &Recorder, backends: Option<HostBackends>) -> Agent {
    let mut builder = Agent::builder()
        .instructions("Answer briefly.")
        .provider(Provider::new("recorder", recorder.clone()).base_url("https://rec.invalid/v1"))
        .model(MODEL)
        .host_services(HostServices::new().request_metadata(yolop_metadata()));
    if let Some(backends) = backends {
        builder = builder.backends(backends);
    }
    builder.build().expect("valid agent")
}

/// The last provider request carried every host value, the live provider and
/// model were unchanged, and the runtime still owns the session correlation.
fn assert_request(recorder: &Recorder, session: SessionId, stage: &str) {
    let (model, metadata) = recorder.last();
    assert_eq!(model, MODEL, "{stage}: provider/model selection unchanged");
    for (key, value) in expected() {
        assert_eq!(
            metadata.get(&key),
            Some(&value),
            "{stage}: {key} on the provider request"
        );
    }
    assert_eq!(
        metadata.get("session_id"),
        Some(&session.to_string()),
        "{stage}: runtime-owned session correlation"
    );
}

async fn assert_turn(session: &Session, recorder: &Recorder, stage: &str) {
    let before = recorder.len();
    let turn = session.run(stage).await.expect("turn");
    assert!(turn.success, "{stage}: {:?}", turn.error);
    assert!(recorder.len() > before, "{stage}: a provider request ran");
    assert_request(recorder, session.session_id(), stage);
    let (_, metadata) = recorder.last();
    assert!(
        metadata.contains_key("turn_id"),
        "{stage}: turn correlation"
    );
}

async fn assert_queries(session: &Session, recorder: &Recorder, stage: &str) {
    assert_eq!(
        session.inspect().await.expect("inspect").request_metadata,
        expected(),
        "{stage}: inspect"
    );
    assert_eq!(
        session
            .command_context()
            .await
            .expect("command context")
            .request_metadata,
        expected(),
        "{stage}: command context"
    );
    let review = session
        .complete_out_of_band(Some("Review."), "done?")
        .await
        .expect("completion");
    assert_eq!(review, "recorded");
    assert_request(
        recorder,
        session.session_id(),
        &format!("{stage} completion"),
    );
}

#[tokio::test]
async fn custom_backend_agent_keeps_metadata_across_first_run_reconstruction_and_resume() {
    let recorder = Recorder::default();
    let backends = HostBackends::in_memory();
    let engine = Engine::new();
    let session = engine.create(agent(&recorder, Some(backends.clone())));
    let id = session.session_id();

    assert_turn(&session, &recorder, "first run").await;
    assert_queries(&session, &recorder, "first run").await;
    let events = session.events_after(0).await.expect("events");
    assert!(
        events
            .iter()
            .all(|event| event.session_id == id.to_string())
    );

    drop(session);
    let resumed = engine.resume(id).await.expect("resume");
    assert_turn(&resumed, &recorder, "resume").await;
    drop(resumed);

    // A new Engine and a rebuilt Agent value over the same host stores.
    let rebuilt = Engine::new();
    rebuilt
        .attach(id, agent(&recorder, Some(backends)))
        .await
        .expect("attach");
    let reconstructed = rebuilt.resume(id).await.expect("resume rebuilt");
    assert_turn(&reconstructed, &recorder, "reconstruction").await;
    assert_queries(&reconstructed, &recorder, "reconstruction").await;
}

#[tokio::test]
async fn coherent_activations_keep_metadata_across_engine_reconstruction() {
    let recorder = Recorder::default();
    let build = AgentBuild::new("assistant", "metadata-v1");
    let profile = MemoryHosting::default();
    let first = Engine::builder()
        .hosting(profile.clone())
        .register(build.clone(), agent(&recorder, None))
        .build();
    let id = SessionId::new();
    let session = first.create_registered(id, &build).await.expect("create");
    assert_turn(&session, &recorder, "coherent first run").await;
    assert_queries(&session, &recorder, "coherent first run").await;
    drop(session);
    drop(first);

    let second = Engine::builder()
        .hosting(profile)
        .register(build, agent(&recorder, None))
        .build();
    let resumed = second.resume(id).await.expect("resume");
    assert_turn(&resumed, &recorder, "coherent resume").await;
    assert_queries(&resumed, &recorder, "coherent resume").await;
}

#[test]
fn metadata_cannot_claim_runtime_correlation_keys() {
    for key in ["session_id", "turn_id", "exec_id", "agent_id", "Session_Id"] {
        let error = Agent::builder()
            .instructions("x")
            .provider(Provider::new("recorder", Recorder::default()))
            .model(MODEL)
            .host_services(
                HostServices::new().request_metadata(RequestMetadata::new().entry(key, "spoof")),
            )
            .build()
            .expect_err("reserved key");
        assert!(
            matches!(&error, BuildError::InvalidRequestMetadata { key: rejected, .. } if rejected == key),
            "{error:?}"
        );
    }
}
