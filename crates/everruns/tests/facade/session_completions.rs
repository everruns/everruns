//! Host-curated, tool-less completions through Engine sessions (EVE-1303).
//! Imports only the public `everruns` API.
//!
//! A recording driver behind host provider services captures exactly what
//! each provider call receives, on both hosting lanes.
#![allow(deprecated)] // Exercises the legacy facade lane until its retirement (EVE-1298).

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use everruns::hosting::services::{
    AgentLoopError, DriverRegistry, HostModelId, HostModelSpec, HostProviderKey, HostProviders,
    HostServices, ProviderConfig, ProviderStore, ToolCall, async_trait,
};
use everruns::hosting::spi::HostBackends;
use everruns::llm::{Message, MessageRole};
use everruns::{
    Agent, ChatDriver, CompletionRequest, DriverConfig, DriverId, Engine, FunctionTool,
    LlmCallConfig, LlmResponseStream, LlmStreamEvent, Model, ProviderEndpoint, ReasoningEffort,
    Session, SessionControlError, SessionId,
};
use serde_json::{Value, json};

const REVIEW_PROMPT: &str = "REVIEW_PROMPT: decide whether the task is complete.";
const EVIDENCE: &str = "EVIDENCE: tests pass; the PR merged.";
const INSTRUCTIONS: &str = "AGENT_INSTRUCTIONS_SENTINEL";
const RECORDER: &str = "completion_recorder";
const DEADLINE: Duration = Duration::from_secs(20);

/// One provider call, as the driver received it.
#[derive(Clone, Debug)]
struct Call {
    provider: String,
    model: String,
    system: Vec<String>,
    messages: Vec<(MessageRole, String)>,
    tools: usize,
    metadata: HashMap<String, String>,
    effort: Option<ReasoningEffort>,
}

impl Call {
    fn is_review(&self) -> bool {
        self.metadata.contains_key("purpose")
    }
}

#[derive(Default)]
struct Log {
    calls: Mutex<Vec<Call>>,
    /// Review completions never answer while set.
    hang: AtomicBool,
}

impl Log {
    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn reviews(&self) -> Vec<Call> {
        self.calls().into_iter().filter(Call::is_review).collect()
    }
}

struct Recorder {
    provider: String,
    log: Arc<Log>,
}

#[async_trait]
impl ChatDriver for Recorder {
    async fn chat_completion_stream(
        &self,
        _endpoint: &ProviderEndpoint,
        messages: Vec<Message>,
        config: &LlmCallConfig,
    ) -> Result<LlmResponseStream, AgentLoopError> {
        let call = Call {
            provider: self.provider.clone(),
            model: config.model.clone(),
            system: messages
                .iter()
                .filter(|message| message.role == MessageRole::System)
                .map(|message| message.content.to_text())
                .collect(),
            messages: messages
                .iter()
                .filter(|message| message.role != MessageRole::System)
                .map(|message| (message.role.clone(), message.content.to_text()))
                .collect(),
            tools: config.tools.len(),
            metadata: config.metadata.clone(),
            effort: config.reasoning_effort,
        };
        let review = call.is_review();
        let has_review_tool = config.tools.iter().any(|tool| tool.name() == "review");
        let answered = messages
            .iter()
            .any(|message| message.role == MessageRole::Tool);
        self.log.calls.lock().unwrap().push(call);
        if review && self.log.hang.load(Ordering::SeqCst) {
            return Ok(Box::pin(futures::stream::pending()));
        }
        let event = if !review && has_review_tool && !answered {
            LlmStreamEvent::ToolCalls(vec![ToolCall {
                id: "review-1".into(),
                name: "review".into(),
                arguments: json!({}),
            }])
        } else if review {
            LlmStreamEvent::TextDelta(format!("COMPLETE via {}", self.provider))
        } else {
            LlmStreamEvent::TextDelta("done".into())
        };
        Ok(Box::pin(futures::stream::iter([
            Ok(event),
            Ok(LlmStreamEvent::Done(Box::default())),
        ])))
    }
}

/// Host provider catalog: serves the keys in `served` through the recorder.
#[derive(Default)]
struct Catalog {
    served: Mutex<HashSet<String>>,
}

#[async_trait]
impl ProviderStore for Catalog {
    async fn get_model_spec(
        &self,
        _model_id: HostModelId,
    ) -> Result<Option<HostModelSpec>, AgentLoopError> {
        Ok(None)
    }

    async fn get_default_model_spec(&self) -> Result<Option<HostModelSpec>, AgentLoopError> {
        Ok(None)
    }

    async fn get_provider_config_for_session(
        &self,
        provider: &HostProviderKey,
        _session: SessionId,
    ) -> Result<Option<ProviderConfig>, AgentLoopError> {
        self.get_provider_config(provider).await
    }

    async fn get_provider_config(
        &self,
        provider: &HostProviderKey,
    ) -> Result<Option<ProviderConfig>, AgentLoopError> {
        Ok(self
            .served
            .lock()
            .unwrap()
            .contains(provider.as_str())
            .then(|| ProviderConfig::for_provider(provider.clone(), DriverId::external(RECORDER))))
    }
}

#[derive(Clone, Copy, Debug)]
enum Lane {
    /// The default Engine hosts sessions coherently.
    Coherent,
    /// Caller-owned backends select the explicit legacy lane.
    Legacy,
}

const LANES: [Lane; 2] = [Lane::Coherent, Lane::Legacy];

struct Fixture {
    log: Arc<Log>,
    catalog: Arc<Catalog>,
}

impl Fixture {
    fn new() -> Self {
        let catalog = Arc::new(Catalog::default());
        catalog
            .served
            .lock()
            .unwrap()
            .extend(["alpha".to_string(), "beta".to_string()]);
        Self {
            log: Arc::default(),
            catalog,
        }
    }

    fn agent(&self, lane: Lane, tools: Vec<FunctionTool>) -> Agent {
        let mut drivers = DriverRegistry::new();
        let log = self.log.clone();
        drivers.register_external(RECORDER, move |config: &DriverConfig| {
            Box::new(Recorder {
                provider: config.provider.to_string(),
                log: log.clone(),
            })
        });
        let mut builder = Agent::builder()
            .instructions(INSTRUCTIONS)
            .model(Model::on("alpha", "m1"))
            .host_services(
                HostServices::new().providers(HostProviders::shared(self.catalog.clone(), drivers)),
            );
        for tool in tools {
            builder = builder.tool(tool);
        }
        if let Lane::Legacy = lane {
            builder = builder.backends(HostBackends::in_memory());
        }
        builder.build().expect("valid agent")
    }
}

fn review() -> CompletionRequest {
    CompletionRequest::new()
        .system_prompt(REVIEW_PROMPT)
        .user(EVIDENCE)
        .purpose("completion_review")
}

async fn canonical(session: &Session) -> (usize, usize) {
    (
        session.events_after(0).await.expect("events").len(),
        session
            .history()
            .page()
            .await
            .expect("history")
            .messages
            .len(),
    )
}

fn assert_exact_review(call: &Call, session: &Session, provider: &str, model: &str) {
    assert_eq!(
        call.system,
        [REVIEW_PROMPT],
        "only the host's system prompt"
    );
    assert_eq!(
        call.messages,
        [(MessageRole::User, EVIDENCE.to_string())],
        "only the bounded evidence message"
    );
    assert_eq!(call.tools, 0, "no tools");
    assert_eq!(call.metadata["purpose"], "completion_review");
    assert_eq!(call.metadata["session_id"], session.id());
    assert_eq!(
        (call.provider.as_str(), call.model.as_str()),
        (provider, model)
    );
}

// ---------------------------------------------------------------------------
// The explicit envelope and the convenience form
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_curated_completion_sends_exactly_the_host_envelope_on_both_lanes() {
    for lane in LANES {
        let fixture = Fixture::new();
        let session = Engine::new().create(fixture.agent(lane, vec![]));
        assert!(session.run("original request").await.unwrap().success);
        let before = canonical(&session).await;

        let answer = session.complete(review()).await.expect("review");
        assert_eq!(answer, "COMPLETE via alpha", "{lane:?}");
        let reviews = fixture.log.reviews();
        assert_eq!(reviews.len(), 1, "{lane:?}");
        assert_exact_review(&reviews[0], &session, "alpha", "m1");
        assert_eq!(
            canonical(&session).await,
            before,
            "{lane:?}: history unchanged"
        );

        // The convenience form still prepends the session's own context.
        session
            .complete_out_of_band(Some("Review strictly."), "Was it done?")
            .await
            .expect("convenience review");
        let convenience = fixture.log.calls().pop().unwrap();
        assert!(convenience.system.join("\n").contains(INSTRUCTIONS));
        assert!(
            convenience
                .messages
                .iter()
                .any(|(_, text)| text.contains("original request"))
        );
        assert_eq!(
            canonical(&session).await,
            before,
            "{lane:?}: history unchanged"
        );
    }
}

#[tokio::test]
async fn invalid_envelopes_and_metadata_are_rejected_before_any_provider_call() {
    for lane in LANES {
        let fixture = Fixture::new();
        let session = Engine::new().create(fixture.agent(lane, vec![]));
        let invalid = [
            CompletionRequest::new().system_prompt(REVIEW_PROMPT),
            review().system_prompt("  "),
            review().metadata("session_id", "spoofed"),
            review().metadata("turn_id", "spoofed"),
            review().metadata("Not A Key", "x"),
            review().metadata("note", "x".repeat(300)),
            review().message(everruns::hosting::services::RuntimeMessage::tool_result(
                "call-1",
                Some(json!({})),
                None,
            )),
            review().timeout(Duration::ZERO),
        ];
        for request in invalid {
            assert!(
                matches!(
                    session.complete(request).await,
                    Err(SessionControlError::Rejected(_))
                ),
                "{lane:?}"
            );
        }
        assert!(fixture.log.calls().is_empty(), "{lane:?}: no provider call");
    }
}

// ---------------------------------------------------------------------------
// Effective model, provider changes and reasoning controls
// ---------------------------------------------------------------------------

#[tokio::test]
async fn completions_follow_live_provider_changes_without_fallback() {
    for lane in LANES {
        let fixture = Fixture::new();
        let session = Engine::new().create(fixture.agent(lane, vec![]));
        session
            .set_model(Model::on("beta", "m2"))
            .await
            .expect("select");
        assert_eq!(
            session.complete(review()).await.unwrap(),
            "COMPLETE via beta",
            "{lane:?}"
        );
        assert_exact_review(
            fixture.log.reviews().last().unwrap(),
            &session,
            "beta",
            "m2",
        );

        // The selected provider stops being served: the request fails rather
        // than reaching the Agent's provider.
        fixture.catalog.served.lock().unwrap().remove("beta");
        let calls = fixture.log.calls().len();
        assert!(
            session.complete(review()).await.is_err(),
            "{lane:?}: no silent fallback"
        );
        assert_eq!(fixture.log.calls().len(), calls, "{lane:?}");

        fixture
            .catalog
            .served
            .lock()
            .unwrap()
            .insert("beta".to_string());
        session.reset_model().await.unwrap();
        assert_eq!(
            session.complete(review()).await.unwrap(),
            "COMPLETE via alpha",
            "{lane:?}"
        );
    }
}

#[tokio::test]
async fn reasoning_controls_default_to_the_session_and_override_per_request() {
    for lane in LANES {
        let fixture = Fixture::new();
        let session = Engine::new().create(fixture.agent(lane, vec![]));
        session.complete(review()).await.unwrap();
        session
            .set_reasoning_effort(Some(ReasoningEffort::High))
            .await
            .unwrap();
        session.complete(review()).await.unwrap();
        session
            .complete(review().reasoning_effort(ReasoningEffort::Low))
            .await
            .unwrap();
        session.complete(review()).await.unwrap();
        let efforts: Vec<_> = fixture
            .log
            .reviews()
            .iter()
            .map(|call| call.effort)
            .collect();
        assert_eq!(
            efforts,
            [
                None,
                Some(ReasoningEffort::High),
                Some(ReasoningEffort::Low),
                Some(ReasoningEffort::High),
            ],
            "{lane:?}: a per-request effort never changes the session selection"
        );
    }
}

// ---------------------------------------------------------------------------
// Active turns, timeouts and cancellation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn a_tool_of_the_active_turn_runs_a_reentrant_review() {
    for lane in LANES {
        let fixture = Fixture::new();
        let slot: Arc<Mutex<Option<Session>>> = Arc::default();
        let tool = FunctionTool::new("review", "Review the task.", json!({"type": "object"}), {
            let slot = slot.clone();
            move |_: Value| {
                let session = slot.lock().unwrap().clone().expect("session installed");
                async move {
                    let verdict = session
                        .complete(review())
                        .await
                        .map_err(|error| error.to_string())?;
                    Ok::<_, String>(json!({ "verdict": verdict }))
                }
            }
        });
        let session = Engine::new().create(fixture.agent(lane, vec![tool]));
        *slot.lock().unwrap() = Some(session.clone());

        let turn = tokio::time::timeout(DEADLINE, session.run("finish the task"))
            .await
            .expect("no deadlock on the session's own mailbox")
            .expect("turn");
        assert!(turn.success, "{lane:?}: {:?}", turn.error);
        let reviews = fixture.log.reviews();
        assert_eq!(reviews.len(), 1, "{lane:?}");
        assert_exact_review(&reviews[0], &session, "alpha", "m1");
        let last = fixture.log.calls().pop().unwrap();
        assert!(
            last.messages
                .iter()
                .any(|(_, text)| text.contains("COMPLETE via alpha")),
            "{lane:?}: the verdict reached the turn's next model call"
        );
        let users = session
            .history()
            .page()
            .await
            .unwrap()
            .messages
            .iter()
            .filter(|message| message.role == everruns::MessageRole::User)
            .count();
        assert_eq!(users, 1, "{lane:?}: no nested turn");
        slot.lock().unwrap().take();
    }
}

#[tokio::test]
async fn timed_out_and_abandoned_completions_leave_the_session_usable() {
    for lane in LANES {
        let fixture = Fixture::new();
        let session = Engine::new().create(fixture.agent(lane, vec![]));
        assert!(session.run("original request").await.unwrap().success);
        let before = canonical(&session).await;
        fixture.log.hang.store(true, Ordering::SeqCst);

        let timeout = Duration::from_millis(100);
        assert!(
            matches!(
                session.complete(review().timeout(timeout)).await,
                Err(SessionControlError::TimedOut(after)) if after == timeout
            ),
            "{lane:?}"
        );

        let pending = tokio::spawn({
            let session = session.clone();
            async move { session.complete(review()).await }
        });
        tokio::time::timeout(DEADLINE, async {
            while fixture.log.reviews().len() < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("second review reached the provider");
        pending.abort();
        assert!(pending.await.unwrap_err().is_cancelled());

        fixture.log.hang.store(false, Ordering::SeqCst);
        let answer = tokio::time::timeout(DEADLINE, session.complete(review()))
            .await
            .expect("still answered")
            .unwrap();
        assert_eq!(answer, "COMPLETE via alpha");
        assert!(session.run("next").await.unwrap().success, "{lane:?}");
        assert_eq!(
            canonical(&session).await.1,
            before.1 + 2,
            "{lane:?}: only the next turn's messages were appended"
        );
    }
}

// ---------------------------------------------------------------------------
// Reconstructed sessions
// ---------------------------------------------------------------------------

#[tokio::test]
async fn reconstructed_sessions_complete_against_the_retained_selection() {
    use everruns::hosting::{AgentBuild, MemoryHosting};

    // Coherent hosting: a new Engine over the same authority.
    let fixture = Fixture::new();
    let build = AgentBuild::new("reviewer", "completions-v1");
    let profile = MemoryHosting::default();
    let first = Engine::builder()
        .hosting(profile.clone())
        .register(build.clone(), fixture.agent(Lane::Coherent, vec![]))
        .build();
    let id = SessionId::new();
    let session = first.create_registered(id, &build).await.unwrap();
    assert!(session.run("original request").await.unwrap().success);
    session.set_model(Model::on("beta", "m2")).await.unwrap();
    drop(session);
    drop(first);
    let second = Engine::builder()
        .hosting(profile)
        .register(build, fixture.agent(Lane::Coherent, vec![]))
        .build();
    let resumed = second.resume(id).await.unwrap();
    let before = canonical(&resumed).await;
    assert_eq!(
        resumed.complete(review()).await.unwrap(),
        "COMPLETE via beta"
    );
    assert_exact_review(
        fixture.log.reviews().last().unwrap(),
        &resumed,
        "beta",
        "m2",
    );
    assert_eq!(canonical(&resumed).await, before);

    // Legacy lane: a resumed handle of the owning Engine.
    let fixture = Fixture::new();
    let engine = Engine::new();
    let session = engine.create(fixture.agent(Lane::Legacy, vec![]));
    let id = session.session_id();
    assert!(session.run("original request").await.unwrap().success);
    session.set_model(Model::on("beta", "m2")).await.unwrap();
    drop(session);
    let resumed = engine.resume(id).await.unwrap();
    let before = canonical(&resumed).await;
    assert_eq!(
        resumed.complete(review()).await.unwrap(),
        "COMPLETE via beta"
    );
    assert_exact_review(
        fixture.log.reviews().last().unwrap(),
        &resumed,
        "beta",
        "m2",
    );
    assert_eq!(canonical(&resumed).await, before);
}
