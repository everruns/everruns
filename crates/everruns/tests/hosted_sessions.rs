//! Session authority conformance through the public Framework API.
use everruns::hosting::{AdmissionState, AgentBuild, MemoryHosting, SessionHosting};
use everruns::{Agent, Engine, MessageId, Model, RunError, SessionId};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

fn build() -> AgentBuild {
    AgentBuild::new("assistant", "test-v1")
}
fn agent() -> Agent {
    Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .build()
        .unwrap()
}
fn engine(profile: impl SessionHosting, agent: Agent) -> Engine {
    Engine::builder()
        .hosting(profile)
        .register(build(), agent)
        .build()
}

/// Simulate independent alarm delivery. The test owns and can kill an executor
/// without also killing the authority, just like a durable host deployment.
#[derive(Clone)]
struct External<H>(H);
#[async_trait::async_trait]
impl<H: SessionHosting> SessionHosting for External<H> {
    async fn resolve_waits(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: everruns::TurnId,
        approvals: Vec<everruns::hosting::ApprovalCommit>,
        results: Vec<everruns_core::events::ToolCompletedData>,
    ) -> everruns_contracts::error::Result<()> {
        self.0
            .resolve_waits(id, command, target, approvals, results)
            .await
    }
    async fn submit_approvals(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: everruns::TurnId,
        decisions: Vec<everruns_contracts::execution_api::ToolApprovalAnswer>,
    ) -> everruns_contracts::error::Result<
        everruns_contracts::execution_api::SubmitToolApprovalsResponse,
    > {
        self.0
            .submit_approvals(id, command, target, decisions)
            .await
    }
    async fn configure(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        patch: everruns::hosting::SessionConfiguration,
    ) -> everruns_contracts::error::Result<()> {
        self.0.configure(id, command, patch).await
    }
    async fn seed_history(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        messages: Vec<everruns_core::RuntimeMessage>,
    ) -> everruns_contracts::error::Result<bool> {
        self.0.seed_history(id, command, messages).await
    }
    async fn query_snapshot(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::SessionQuerySnapshot> {
        self.0.query_snapshot(id).await
    }
    async fn approve_many(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        approvals: Vec<everruns::hosting::ApprovalCommit>,
    ) -> everruns_contracts::error::Result<()> {
        self.0.approve_many(id, command, approvals).await
    }
    async fn create(
        &self,
        id: SessionId,
        binding: &everruns::hosting::SessionBinding,
    ) -> everruns_contracts::error::Result<()> {
        self.0.create(id, binding).await
    }
    async fn status(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::HostingStatus> {
        self.0.status(id).await
    }
    async fn admit(
        &self,
        id: SessionId,
        message: everruns_core::RuntimeMessage,
        cancelled: bool,
    ) -> everruns_contracts::error::Result<everruns::hosting::AdmissionReceipt> {
        self.0.admit(id, message, cancelled).await
    }
    async fn receipt(
        &self,
        id: SessionId,
        message: MessageId,
    ) -> everruns_contracts::error::Result<Option<everruns::hosting::AdmissionReceipt>> {
        self.0.receipt(id, message).await
    }
    fn reader(&self, id: SessionId) -> everruns_core::host::HostBackends {
        self.0.reader(id)
    }
    async fn record(
        &self,
        id: SessionId,
        key: &str,
    ) -> everruns_contracts::error::Result<Option<serde_json::Value>> {
        self.0.record(id, key).await
    }
    async fn resolve(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        target: everruns::TurnId,
        results: Vec<everruns_core::events::ToolCompletedData>,
    ) -> everruns_contracts::error::Result<()> {
        self.0.resolve(id, command, target, results).await
    }
    async fn cancel(
        &self,
        id: SessionId,
        command: uuid::Uuid,
        input: MessageId,
    ) -> everruns_contracts::error::Result<()> {
        self.0.cancel(id, command, input).await
    }
    async fn claim(
        &self,
        id: SessionId,
        holder: &str,
        ttl: std::time::Duration,
    ) -> everruns_contracts::error::Result<Option<everruns::hosting::SessionActivation>> {
        self.0.claim(id, holder, ttl).await
    }
}

#[tokio::test]
async fn durable_identity_admission_and_outcome_reopen_in_a_new_engine() {
    let profile = MemoryHosting::default();
    let first = engine(profile.clone(), agent());
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    let sent = session.send_with_id(message, "hello").await.unwrap();
    assert!(
        sent.turn_id.is_none(),
        "admission must not fabricate a turn id"
    );
    assert_eq!(
        session
            .send_with_id(message, "hello")
            .await
            .unwrap()
            .message_id,
        sent.message_id
    );
    assert!(session.send_with_id(message, "different").await.is_err());
    drop(first);
    drop(session);
    drop(sent);
    let replacement = engine(profile, agent());
    let resumed = replacement.resume(id).await.unwrap();
    let handle = resumed.admission(message).await.unwrap().unwrap();
    let result = tokio::time::timeout(std::time::Duration::from_secs(5), handle.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(result.response, "Hello!");
    assert!(result.success);
    assert!(handle.assignment().await.unwrap().is_some());
    let events = resumed.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "input.message")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.started")
            .count(),
        1
    );
    assert_eq!(resumed.history().page().await.unwrap().messages.len(), 2);
}

#[tokio::test]
async fn completion_is_not_visible_until_awaited_framework_hooks_settle() {
    let profile = MemoryHosting::default();
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let entered_hook = entered.clone();
    let release_hook = release.clone();
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("done"))
        .on_completion(move |_| {
            let entered = entered_hook.clone();
            let release = release_hook.clone();
            async move {
                entered.notify_one();
                release.notified().await;
            }
        })
        .build()
        .unwrap();
    let engine = engine(profile.clone(), agent);
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "hello").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), entered.notified())
        .await
        .unwrap();
    let handle = session.admission(message).await.unwrap().unwrap();
    assert!(matches!(
        handle.state().await.unwrap(),
        AdmissionState::Assigned { .. }
    ));
    assert!(
        session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.event_type() == "turn.completed")
    );
    assert!(profile.status(session.session_id()).await.unwrap().runnable);
    release.notify_one();
    assert!(handle.wait().await.unwrap().success);
}

#[tokio::test]
async fn settled_pre_effect_failure_rejects_the_command_and_never_runs_the_model() {
    let profile = MemoryHosting::default();
    let calls = Arc::new(AtomicUsize::new(0));
    let counted = calls.clone();
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("must not run"))
        .on_agent_start(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            async { Err::<(), _>("gate refused") }
        })
        .build()
        .unwrap();
    let engine = engine(profile.clone(), agent);
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    let result = session.send("hello").await.unwrap().wait().await;
    assert!(matches!(result, Err(RunError::Hook(_))));
    engine.activate(session.session_id()).await.unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(
        !session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.event_type() == "reason.started")
    );
}

async fn configuration_seed_and_queries_reopen_without_execution(
    profile: impl SessionHosting + Clone,
) {
    use everruns::hosting::SessionConfiguration;
    let id = SessionId::new();
    let owner = engine(External(profile.clone()), agent());
    let session = owner.create_registered(id, &build()).await.unwrap();
    let command = uuid::Uuid::new_v4();
    let patch = SessionConfiguration {
        instructions: Some(Some("Retained override.".into())),
        client_tools: Some(Vec::new()),
    };
    profile.configure(id, command, patch.clone()).await.unwrap();
    profile.configure(id, command, patch).await.unwrap();
    assert!(
        profile
            .configure(id, command, SessionConfiguration::default())
            .await
            .is_err()
    );
    let seed = uuid::Uuid::new_v4();
    let messages = vec![
        everruns_core::RuntimeMessage::user("Earlier question"),
        everruns_core::RuntimeMessage::assistant("Earlier answer"),
    ];
    assert!(
        profile
            .seed_history(id, seed, messages.clone())
            .await
            .unwrap()
    );
    assert!(
        profile
            .seed_history(id, seed, messages.clone())
            .await
            .unwrap()
    );
    assert!(
        !profile
            .seed_history(id, uuid::Uuid::new_v4(), messages)
            .await
            .unwrap()
    );
    let before = profile.query_snapshot(id).await.unwrap().high_watermark;
    let context = session.inspect().await.unwrap();
    assert!(context.instructions.contains("Retained override."));
    assert_eq!(context.messages.len(), 2);
    assert_eq!(
        profile.query_snapshot(id).await.unwrap().high_watermark,
        before
    );
    assert!(!profile.status(id).await.unwrap().runnable);
    let replacement = engine(External(profile.clone()), agent());
    assert!(
        replacement
            .resume(id)
            .await
            .unwrap()
            .inspect()
            .await
            .unwrap()
            .instructions
            .contains("Retained override.")
    );
    // Explicit null clears an override across serialization and reopening.
    let clear: SessionConfiguration =
        serde_json::from_value(serde_json::json!({"instructions":null})).unwrap();
    profile
        .configure(id, uuid::Uuid::new_v4(), clear)
        .await
        .unwrap();
    assert!(
        !session
            .inspect()
            .await
            .unwrap()
            .instructions
            .contains("Retained override.")
    );
}

#[tokio::test]
async fn memory_queries_seed_and_configuration_are_coherent() {
    configuration_seed_and_queries_reopen_without_execution(MemoryHosting::default()).await;
}

#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_queries_seed_and_configuration_are_coherent() {
    let dir = tempfile::tempdir().unwrap();
    configuration_seed_and_queries_reopen_without_execution(
        everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap(),
    )
    .await;
}

#[tokio::test(start_paused = true)]
async fn uncertain_agent_start_suspends_without_repeating_the_effect_or_dispatching_a_model() {
    let profile = MemoryHosting::default();
    let effects = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let counted = effects.clone();
    let signal = entered.clone();
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("must not run"))
        .on_agent_start(move |_| {
            let counted = counted.clone();
            let signal = signal.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                signal.notify_one();
                std::future::pending::<()>().await;
            }
        })
        .build()
        .unwrap();
    let make = || engine(External(profile.clone()), agent.clone());
    let first = make();
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let sent = session.send("hello").await.unwrap();
    let executor = tokio::spawn(async move { first.activate(id).await });
    entered.notified().await;
    executor.abort();
    let _ = executor.await;
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    let replacement = make();
    replacement.activate(id).await.unwrap();
    assert!(matches!(sent.wait().await, Err(RunError::Suspended)));
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(
        !session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.event_type() == "reason.started")
    );
}

#[tokio::test]
async fn registered_build_conflicts_and_unavailable_builds_never_select_latest() {
    let profile = MemoryHosting::default();
    let id = SessionId::new();
    engine(profile.clone(), agent())
        .create_registered(id, &build())
        .await
        .unwrap();
    let other = Engine::builder()
        .hosting(profile.clone())
        .register(AgentBuild::new("assistant", "test-v2"), agent())
        .build();
    assert!(
        other
            .create_registered(id, &AgentBuild::new("assistant", "test-v2"))
            .await
            .is_err()
    );
    let readable = other.resume(id).await.unwrap();
    assert!(readable.events_after(0).await.unwrap().is_empty());
    assert!(matches!(
        readable
            .send("run the retained build")
            .await
            .unwrap()
            .wait()
            .await,
        Err(RunError::Suspended)
    ));
    assert_eq!(profile.status(id).await.unwrap().binding.behavior, build());
}

#[tokio::test]
async fn authoritative_events_catch_up_after_frontend_loss() {
    let profile = MemoryHosting::default();
    let first = engine(profile.clone(), agent());
    let session = first
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    session.send_and_wait("one").await.unwrap();
    let before = session.events_after(0).await.unwrap();
    let cursor = before.last().unwrap().sequence().unwrap();
    let replacement = engine(profile, agent());
    let resumed = replacement.resume(session.session_id()).await.unwrap();
    let mut stream = resumed.events_from(cursor).await.unwrap();
    session.send_and_wait("two").await.unwrap();
    let after = resumed.events_after(cursor).await.unwrap();
    for expected in after {
        let next = tokio::time::timeout(std::time::Duration::from_secs(3), stream.recv())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(next.sequence(), expected.sequence());
    }
}

#[tokio::test(start_paused = true)]
async fn executor_loss_after_terminal_recovers_finalization_without_repeating_effects() {
    let profile = MemoryHosting::default();
    let effects = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let counted = effects.clone();
    let signal = entered.clone();
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("done"))
        .on_completion(move |_| {
            let counted = counted.clone();
            let signal = signal.clone();
            async move {
                counted.fetch_add(1, Ordering::SeqCst);
                signal.notify_one();
                std::future::pending::<()>().await;
            }
        })
        .build()
        .unwrap();
    let make = || {
        Engine::builder()
            .hosting(External(profile.clone()))
            .register(build(), agent.clone())
            .build()
    };
    let first = make();
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "hello").await.unwrap();
    let executor = tokio::spawn(async move { first.activate(id).await });
    entered.notified().await;
    executor.abort();
    let _ = executor.await;
    drop(session);
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    let replacement = make();
    replacement.activate(id).await.unwrap();
    let resumed = replacement.resume(id).await.unwrap();
    let outcome = resumed
        .admission(message)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(outcome.success);
    assert_eq!(
        effects.load(Ordering::SeqCst),
        1,
        "uncertain completion effect must not repeat"
    );
    assert_eq!(outcome.hook_failures.len(), 1);
    assert!(
        outcome.hook_failures[0]
            .message
            .contains("uncertain outcome")
    );
    let events = resumed.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "reason.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.started")
            .count(),
        1
    );
}

#[tokio::test]
async fn cancelling_a_pending_admission_never_cancels_a_later_turn() {
    let profile = MemoryHosting::default();
    let starts = Arc::new(AtomicUsize::new(0));
    let counted = starts.clone();
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("done"))
        .on_agent_start(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            async {}
        })
        .build()
        .unwrap();
    let engine = Engine::builder()
        .hosting(External(profile.clone()))
        .register(build(), agent)
        .build();
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "cancel this").await.unwrap();
    let admission = session.admission(message).await.unwrap().unwrap();
    let cancellation = uuid::Uuid::new_v4();
    admission.cancel(cancellation).await.unwrap();
    engine.activate(id).await.unwrap();
    assert_eq!(
        admission.wait().await.unwrap().stop_reason,
        everruns::TurnStopReason::Cancelled
    );
    assert_eq!(
        starts.load(Ordering::SeqCst),
        0,
        "cancelled admission must not dispatch lifecycle effects"
    );
    let second = session
        .send_with_id(MessageId::new(), "next")
        .await
        .unwrap();
    admission.cancel(cancellation).await.unwrap();
    admission.cancel(uuid::Uuid::new_v4()).await.unwrap();
    engine.activate(id).await.unwrap();
    assert!(second.wait().await.unwrap().success);
    assert_eq!(
        session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .filter(|event| event.event_type() == "reason.started")
            .count(),
        1
    );
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_startup_supervision_recovers_an_accepted_command_without_client_traffic() {
    use everruns::hosting::SqliteHosting;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let id = SessionId::new();
    let message = MessageId::new();
    {
        let profile = SqliteHosting::open(&path).unwrap();
        let first = engine(External(profile), agent());
        let session = first.create_registered(id, &build()).await.unwrap();
        session.send_with_id(message, "hello").await.unwrap();
        // No executor was started. Drop every connection and frontend.
    }
    let profile = SqliteHosting::open(&path).unwrap();
    let replacement = engine(profile.clone(), agent());
    let stopped = everruns::CancellationToken::new();
    let signal = stopped.clone();
    let supervisor_engine = replacement.clone();
    let supervisor =
        tokio::spawn(async move { profile.supervise(supervisor_engine, signal).await });
    // The only client operations after restart are reads. They do not activate.
    let resumed = replacement.resume(id).await.unwrap();
    let admission = resumed.admission(message).await.unwrap().unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), admission.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.response, "Hello!");
    stopped.cancel();
    supervisor.await.unwrap().unwrap();
    drop(resumed);
    drop(replacement);
    let final_engine = engine(SqliteHosting::open(&path).unwrap(), agent());
    let session = final_engine.resume(id).await.unwrap();
    assert!(
        session
            .admission(message)
            .await
            .unwrap()
            .unwrap()
            .wait()
            .await
            .unwrap()
            .success
    );
    assert_eq!(
        session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .filter(|event| event.event_type() == "input.message")
            .count(),
        1
    );
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_independent_connections_fence_all_canonical_state_after_expiry() {
    use everruns::hosting::SqliteHosting;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let first = SqliteHosting::open(&path).unwrap();
    let second = SqliteHosting::open(&path).unwrap();
    let engine = engine(External(first.clone()), agent());
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    session.send("hello").await.unwrap();
    let stale = first
        .claim(id, "first-owner", std::time::Duration::from_millis(5))
        .await
        .unwrap()
        .unwrap();
    stale
        .epoch
        .backends()
        .storage_store
        .set_value(id, "counter", "1")
        .await
        .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(15)).await;
    let current = second
        .claim(id, "replacement", std::time::Duration::from_secs(10))
        .await
        .unwrap()
        .unwrap();
    assert!(current.actor.fence > stale.actor.fence);
    assert!(
        stale
            .epoch
            .backends()
            .storage_store
            .set_value(id, "counter", "2")
            .await
            .is_err()
    );
    assert!(
        stale
            .epoch
            .commit("outcome/stale", serde_json::json!({}), false)
            .await
            .is_err()
    );
    assert!(
        first
            .reader(id)
            .storage_store
            .set_value(id, "counter", "3")
            .await
            .is_err()
    );
    assert_eq!(
        second
            .reader(id)
            .storage_store
            .get_value(id, "counter")
            .await
            .unwrap()
            .as_deref(),
        Some("1")
    );
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_rejected_admission_rolls_back_event_cursor_receipt_and_wakeup() {
    use everruns::hosting::SqliteHosting;
    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let engine = engine(External(profile.clone()), agent());
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    assert!(
        session
            .send_with_id(message, "a".repeat(3 * 1024 * 1024))
            .await
            .is_err()
    );
    assert!(session.admission(message).await.unwrap().is_none());
    assert!(session.events_after(0).await.unwrap().is_empty());
    assert!(!profile.status(id).await.unwrap().runnable);
    let sent = session.send_with_id(message, "fits").await.unwrap();
    assert_eq!(
        session
            .admission(message)
            .await
            .unwrap()
            .unwrap()
            .sequence(),
        1
    );
    engine.activate(id).await.unwrap();
    assert!(sent.wait().await.unwrap().success);
}

async fn parked_client_calls_reopen_and_resolve(profile: impl SessionHosting + Clone) {
    let completions = Arc::new(AtomicUsize::new(0));
    let counted = completions.clone();
    let agent = Agent::builder()
        .instructions("Use the client tools.")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("Both resolved.").with_tool_call_sequence(vec![
                vec![
                    everruns::ToolCall {
                        id: "call_one".into(),
                        name: "client_one".into(),
                        arguments: serde_json::json!({}),
                    },
                    everruns::ToolCall {
                        id: "call_two".into(),
                        name: "client_two".into(),
                        arguments: serde_json::json!({}),
                    },
                ],
                vec![],
            ]),
        ))
        .client_tool(
            "client_one",
            "First external operation",
            serde_json::json!({"type":"object","properties":{}}),
        )
        .client_tool(
            "client_two",
            "Second external operation",
            serde_json::json!({"type":"object","properties":{}}),
        )
        .on_completion(move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
            async {}
        })
        .build()
        .unwrap();
    let first = engine(profile.clone(), agent.clone());
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "call both").await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if session.pending_tools().await.unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    drop(session);
    drop(first);
    let replacement = engine(profile, agent);
    let session = replacement.resume(id).await.unwrap();
    let admission = session.admission(message).await.unwrap().unwrap();
    assert!(
        matches!(admission.state().await.unwrap(),AdmissionState::Waiting{calls,..} if calls.len()==2)
    );
    assert_eq!(completions.load(Ordering::SeqCst), 0);
    let turn_id = replacement
        .session_status(id)
        .await
        .unwrap()
        .open_turn
        .unwrap();
    let command = uuid::Uuid::new_v4();
    let first = everruns::ToolAnswer {
        tool_call_id: "call_one".into(),
        tool_name: "client_one".into(),
        result: serde_json::json!(1),
        error: None,
    };
    session
        .resolve_tools(command, turn_id, vec![first.clone()])
        .await
        .unwrap();
    session
        .resolve_tools(command, turn_id, vec![first])
        .await
        .unwrap();
    assert_eq!(session.pending_tools().await.unwrap().len(), 1);
    let wrong = everruns::ToolAnswer {
        tool_call_id: "call_two".into(),
        tool_name: "wrong".into(),
        result: serde_json::json!(2),
        error: None,
    };
    assert!(
        session
            .resolve_tools(uuid::Uuid::new_v4(), turn_id, vec![wrong])
            .await
            .is_err()
    );
    let second = everruns::ToolAnswer {
        tool_call_id: "call_two".into(),
        tool_name: "client_two".into(),
        result: serde_json::json!(2),
        error: None,
    };
    session
        .resolve_tools(uuid::Uuid::new_v4(), turn_id, vec![second])
        .await
        .unwrap();
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), admission.wait())
            .await
            .unwrap()
            .unwrap()
            .response,
        "Both resolved."
    );
    assert_eq!(completions.load(Ordering::SeqCst), 1);
    let events = session.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "tool.completed")
            .count(),
        2
    );
}

#[tokio::test]
async fn memory_durable_client_calls_reopen_and_partial_resolution_stays_parked() {
    parked_client_calls_reopen_and_resolve(MemoryHosting::default()).await;
}

#[tokio::test(start_paused = true)]
async fn explicitly_idempotent_completion_recovers_under_the_same_invocation_identity() {
    let profile = MemoryHosting::default();
    let seen = Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()));
    let attempts = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let effects = seen.clone();
    let counted = attempts.clone();
    let signal = entered.clone();
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("done"))
        .on_completion_with_recovery(everruns::HookRecovery::Idempotent, move |context| {
            let effects = effects.clone();
            let counted = counted.clone();
            let signal = signal.clone();
            async move {
                effects.lock().unwrap().insert(context.turn.turn_id);
                if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                    signal.notify_one();
                    std::future::pending::<()>().await;
                }
            }
        })
        .build()
        .unwrap();
    let make = || engine(External(profile.clone()), agent.clone());
    let first = make();
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "hello").await.unwrap();
    let task = tokio::spawn(async move { first.activate(id).await });
    entered.notified().await;
    task.abort();
    let _ = task.await;
    tokio::time::advance(std::time::Duration::from_secs(11)).await;
    let replacement = make();
    replacement.activate(id).await.unwrap();
    let turn = replacement
        .resume(id)
        .await
        .unwrap()
        .admission(message)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert!(turn.hook_failures.is_empty());
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    assert_eq!(seen.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn reconstructed_outcome_preserves_iteration_limit_and_shared_reply_projection() {
    let profile = MemoryHosting::default();
    let agent = Agent::builder()
        .instructions("Use tools.")
        .max_iterations(1)
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::scripted(vec![everruns::SimTurn::Mixed {
                text: "Checking.".into(),
                tool_calls: vec![everruns::SimToolCall {
                    name: "external".into(),
                    arguments: serde_json::json!({}),
                    id: Some("call_limit".into()),
                }],
            }]),
        ))
        .client_tool(
            "external",
            "External operation",
            serde_json::json!({"type":"object","properties":{}}),
        )
        .build()
        .unwrap();
    let first = engine(profile.clone(), agent.clone());
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    let expected = session
        .send_with_id(message, "hello")
        .await
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(
        expected.stop_reason,
        everruns::TurnStopReason::MaxTurnRequests
    );
    assert_eq!(
        expected.response, "",
        "working commentary is excluded by the shared reply projection"
    );
    drop(session);
    drop(first);
    let recovered = engine(profile, agent)
        .resume(id)
        .await
        .unwrap()
        .admission(message)
        .await
        .unwrap()
        .unwrap()
        .wait()
        .await
        .unwrap();
    assert_eq!(recovered.stop_reason, expected.stop_reason);
    assert_eq!(recovered.response, expected.response);
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_durable_client_calls_reopen_and_partial_resolution_stays_parked() {
    let directory = tempfile::tempdir().unwrap();
    parked_client_calls_reopen_and_resolve(
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
    )
    .await;
}

#[cfg(feature = "builtins")]
async fn approval_survives_frontend_loss(profile: impl SessionHosting + Clone) {
    let effects = Arc::new(AtomicUsize::new(0));
    let counted = effects.clone();
    let agent = Agent::builder()
        .instructions("Request the action, then retry only after approval.")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("Approved action done.").with_tool_call_sequence(vec![
                vec![everruns::ToolCall {
                    id: "call_approval_first".into(),
                    name: "mutate".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![everruns::ToolCall {
                    id: "call_approval_retry".into(),
                    name: "mutate".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ]),
        ))
        .tool(
            everruns::FunctionTool::new(
                "mutate",
                "Change one external value",
                serde_json::json!({"type":"object","properties":{}}),
                move |_: serde_json::Value| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    async { Ok::<_, String>(serde_json::json!({"done":true})) }
                },
            )
            .always_needs_approval(),
        )
        .durable_approvals()
        .build()
        .unwrap();
    let first = engine(profile.clone(), agent.clone());
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let message = MessageId::new();
    session.send_with_id(message, "do it").await.unwrap();
    let request = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if let Some(call) = session
                .pending_tools()
                .await
                .unwrap()
                .into_iter()
                .find(|call| call.name == "approve_tool_call")
            {
                break call;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    drop(session);
    drop(first);
    let replacement = engine(profile, agent);
    let session = replacement.resume(id).await.unwrap();
    let turn_id = replacement
        .session_status(id)
        .await
        .unwrap()
        .open_turn
        .unwrap();
    let bypass = everruns::ToolAnswer {
        tool_call_id: request.id.clone(),
        tool_name: request.name.clone(),
        result: serde_json::json!({"approved":true}),
        error: None,
    };
    assert!(
        session
            .resolve_tools(uuid::Uuid::new_v4(), turn_id, vec![bypass])
            .await
            .is_err()
    );
    // A stale member must not consume the valid permission before the whole
    // batch is validated, including on the volatile authority.
    assert!(
        session
            .approve_tools(
                uuid::Uuid::new_v4(),
                vec![
                    everruns::hosting::ApprovalCommit {
                        turn_id,
                        request_id: request.id.clone(),
                        decision: everruns::hosting::ToolApprovalDecision::Allow
                    },
                    everruns::hosting::ApprovalCommit {
                        turn_id,
                        request_id: "missing-request".into(),
                        decision: everruns::hosting::ToolApprovalDecision::Allow
                    },
                ]
            )
            .await
            .is_err()
    );
    assert_eq!(session.pending_tools().await.unwrap().len(), 1);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    let command = uuid::Uuid::new_v4();
    let decisions = vec![everruns_contracts::execution_api::ToolApprovalAnswer {
        tool_call_id: request.id.clone(),
        decision: everruns::hosting::ToolApprovalDecision::Allow,
    }];
    let receipt = session
        .submit_approvals(command, turn_id, decisions.clone())
        .await
        .unwrap();
    assert_eq!(receipt.resolved.len(), 1);
    assert_eq!(
        receipt,
        session
            .submit_approvals(command, turn_id, decisions)
            .await
            .unwrap()
    );
    assert!(
        session
            .submit_approvals(
                command,
                turn_id,
                vec![everruns_contracts::execution_api::ToolApprovalAnswer {
                    tool_call_id: request.id.clone(),
                    decision: everruns::hosting::ToolApprovalDecision::Reject
                }]
            )
            .await
            .is_err()
    );
    let outcome = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        session.admission(message).await.unwrap().unwrap().wait(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(outcome.success);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(session.pending_tools().await.unwrap().is_empty());
}

#[cfg(feature = "builtins")]
#[tokio::test]
async fn memory_approval_resolves_after_frontend_loss_without_duplicating_effects() {
    approval_survives_frontend_loss(MemoryHosting::default()).await;
}

#[cfg(feature = "builtins")]
async fn mixed_wait_resolution_is_atomic(profile: impl SessionHosting + Clone) {
    let effects = Arc::new(AtomicUsize::new(0));
    let counted = effects.clone();
    let agent = Agent::builder()
        .instructions("Use the tools, then finish.")
        .model(Model::simulated_with_config(
            everruns::LlmSimConfig::fixed("done").with_tool_call_sequence(vec![
                vec![
                    everruns::ToolCall {
                        id: "mutate_first".into(),
                        name: "mutate".into(),
                        arguments: serde_json::json!({}),
                    },
                    everruns::ToolCall {
                        id: "render_first".into(),
                        name: "render".into(),
                        arguments: serde_json::json!({}),
                    },
                ],
                vec![everruns::ToolCall {
                    id: "mutate_retry".into(),
                    name: "mutate".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ]),
        ))
        .client_tool(
            "render",
            "Render the result",
            serde_json::json!({"type":"object"}),
        )
        .tool(
            everruns::FunctionTool::new(
                "mutate",
                "Change one value",
                serde_json::json!({"type":"object"}),
                move |_: serde_json::Value| {
                    let counted = counted.clone();
                    async move {
                        counted.fetch_add(1, Ordering::SeqCst);
                        Ok::<_, String>(serde_json::json!({"done":true}))
                    }
                },
            )
            .always_needs_approval(),
        )
        .durable_approvals()
        .build()
        .unwrap();
    let owner = engine(profile.clone(), agent);
    let id = SessionId::new();
    let session = owner.create_registered(id, &build()).await.unwrap();
    let sent = session.send("work").await.unwrap();
    let waiting = tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let status = profile.status(id).await.unwrap();
            if status.waiting.len() == 2 {
                break status;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let turn = waiting.open_turn.unwrap();
    let approval = waiting
        .waiting
        .iter()
        .find(|call| call.name == "approve_tool_call")
        .unwrap();
    let decisions = vec![everruns::hosting::ApprovalCommit {
        turn_id: turn,
        request_id: approval.id.clone(),
        decision: everruns::hosting::ToolApprovalDecision::Allow,
    }];
    let before = session.events_after(0).await.unwrap().len();
    let result = everruns::ToolAnswer {
        tool_call_id: "render_first".into(),
        tool_name: "render".into(),
        result: serde_json::json!({"rendered":true}),
        error: None,
    };
    let mut invalid = result.clone();
    invalid.tool_call_id = "not_pending".into();
    assert!(
        session
            .resolve_waits(uuid::Uuid::new_v4(), turn, decisions.clone(), vec![invalid])
            .await
            .is_err()
    );
    assert_eq!(session.events_after(0).await.unwrap().len(), before);
    assert_eq!(session.pending_tools().await.unwrap().len(), 2);
    assert_eq!(effects.load(Ordering::SeqCst), 0);
    let command = uuid::Uuid::new_v4();
    session
        .resolve_waits(command, turn, decisions.clone(), vec![result.clone()])
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(3), sent.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    let after = session.events_after(0).await.unwrap().len();
    session
        .resolve_waits(command, turn, decisions, vec![result])
        .await
        .unwrap();
    assert_eq!(session.events_after(0).await.unwrap().len(), after);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "builtins")]
#[tokio::test]
async fn memory_mixed_wait_resolution_is_atomic() {
    mixed_wait_resolution_is_atomic(MemoryHosting::default()).await;
}

#[cfg(all(feature = "builtins", feature = "local"))]
#[tokio::test]
async fn sqlite_mixed_wait_resolution_is_atomic() {
    let dir = tempfile::tempdir().unwrap();
    mixed_wait_resolution_is_atomic(
        everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap(),
    )
    .await;
}

#[cfg(all(feature = "builtins", feature = "local"))]
#[tokio::test]
async fn native_approval_resolves_after_frontend_loss_without_duplicating_effects() {
    let directory = tempfile::tempdir().unwrap();
    approval_survives_frontend_loss(
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
    )
    .await;
}

#[cfg(feature = "local")]
#[tokio::test]
async fn retained_environment_reopens_attachments_and_rejects_unconfigured_workspace_before_creation()
 {
    let directory = tempfile::tempdir().unwrap();
    let authority =
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let configured = Agent::builder()
        .model(Model::simulated("Hello!"))
        .instructions("Read the attached report.")
        .workspace(directory.path().join("workspace"))
        .build()
        .unwrap();
    let unsupported = engine(authority.clone(), configured.clone());
    let rejected = SessionId::new();
    assert!(
        unsupported
            .create_registered(rejected, &build())
            .await
            .is_err()
    );
    assert!(unsupported.session_binding(rejected).await.is_err());
    let first = Engine::builder().hosting(authority.clone()).build();
    first
        .register_agent_with_environment(
            build(),
            configured.clone(),
            None,
            "workspace-v1",
            configured.environment_factory(),
        )
        .unwrap();
    let id = SessionId::new();
    let session = first.create_registered(id, &build()).await.unwrap();
    let part = session
        .attach(
            "report.txt",
            Some("text/plain"),
            b"retained report".to_vec(),
        )
        .await
        .unwrap();
    session.start().await.unwrap();
    let head = session.workspace_head().unwrap().binding().clone();
    assert_eq!(
        first
            .session_binding(id)
            .await
            .unwrap()
            .environment
            .as_deref(),
        Some("workspace-v1")
    );
    drop(session);
    drop(first);
    let rebuilt = Agent::builder()
        .model(Model::simulated("Hello!"))
        .instructions("Read the attached report.")
        .workspace(directory.path().join("workspace"))
        .build()
        .unwrap();
    let replacement = Engine::builder().hosting(authority.clone()).build();
    replacement
        .register_agent_with_environment(
            build(),
            rebuilt.clone(),
            None,
            "workspace-v1",
            rebuilt.environment_factory(),
        )
        .unwrap();
    let session = replacement.resume(id).await.unwrap();
    session.start().await.unwrap();
    assert_eq!(session.workspace_head().unwrap().binding(), &head);
    let everruns::ContentPart::File(file) = part else {
        panic!("expected a file reference")
    };
    let path = everruns_contracts::runtime::attachments::attachment_path(
        &file.file_id.to_string(),
        file.filename.as_deref(),
        "file",
    );
    let reopened = session.workspace_head().unwrap();
    let files = everruns_contracts::runtime::mount_fs::scoped_prompt_file_store(
        reopened.file_system(),
        reopened.workspace_id(),
    );
    let stored = files.read_file(id, &path).await.unwrap().unwrap();
    assert_eq!(stored.content.as_deref(), Some("retained report"));
    assert!(stored.is_readonly);
    // A changed reference cannot silently widen permissions or run newer
    // environment configuration, while canonical history remains readable.
    let changed = Engine::builder().hosting(authority).build();
    changed
        .register_agent_with_environment(
            build(),
            rebuilt.clone(),
            None,
            "workspace-v2",
            rebuilt.environment_factory(),
        )
        .unwrap();
    let reader = changed.resume(id).await.unwrap();
    assert!(reader.events_after(0).await.unwrap().is_empty());
    assert!(reader.start().await.is_err());
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_startup_discovers_a_parked_approval_deadline_without_client_wakeup() {
    use everruns_core::events::{
        EventContext, EventRequest, ToolCallRequestedData, TurnStartedData,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let id = SessionId::new();
    let message = MessageId::new();
    let turn = everruns::TurnId::new();
    {
        let profile = everruns::hosting::SqliteHosting::open(&path).unwrap();
        let first = engine(External(profile.clone()), agent());
        let session = first.create_registered(id, &build()).await.unwrap();
        session
            .send_with_id(message, "wait for approval")
            .await
            .unwrap();
        // Simulate the canonical boundary committed by an owner before loss.
        let activation = profile
            .claim(id, "previous-owner", std::time::Duration::from_secs(5))
            .await
            .unwrap()
            .unwrap();
        let epoch = activation.epoch;
        let log = epoch.backends().event_log;
        let context = EventContext::turn(turn, message);
        log.append(EventRequest::new(
            id,
            context.clone(),
            serde_json::from_value::<TurnStartedData>(
                serde_json::json!({"turn_id":turn,"input_message_id":message}),
            )
            .unwrap(),
        ))
        .await
        .unwrap();
        let now = everruns_contracts::rt::utc_now();
        let deadline = now + chrono::Duration::milliseconds(150);
        let request = everruns::ToolCall {
            id: "tool_approval_original".into(),
            name: "approve_tool_call".into(),
            arguments: serde_json::json!({"code":"tool_approval_required","error":"waiting","tool_call_id":"original","tool":"mutate","arguments":{},"fingerprint":"test","risk":"policy","mode":"normal","asked_at":now.to_rfc3339(),"expires_at":deadline.to_rfc3339()}),
        };
        let paused = log
            .append(EventRequest::new(
                id,
                context,
                ToolCallRequestedData::with_definitions(&[request], &[]),
            ))
            .await
            .unwrap();
        epoch
            .commit(
                &format!("assignment/{message}"),
                serde_json::json!({"turn_id":turn,"steered":false}),
                true,
            )
            .await
            .unwrap();
        epoch
            .schedule(Some(deadline.timestamp_millis()))
            .await
            .unwrap();
        epoch
            .actor_store()
            .release_actor(
                id,
                "previous-owner",
                activation.actor.fence,
                i64::from(paused.sequence.unwrap()),
            )
            .await
            .unwrap();
        assert!(!profile.status(id).await.unwrap().runnable);
    }
    let profile = everruns::hosting::SqliteHosting::open(&path).unwrap();
    let replacement = engine(profile.clone(), agent());
    let stopped = everruns::CancellationToken::new();
    let stop = stopped.clone();
    let host = replacement.clone();
    let supervisor = tokio::spawn(async move { profile.supervise(host, stop).await });
    let session = replacement.resume(id).await.unwrap();
    let admission = session.admission(message).await.unwrap().unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(5), admission.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(outcome.success);
    let events = session.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "tool.completed"
                && event.canonical_json()["data"]["status"] == "timeout"
                && event.canonical_json()["data"]["error"]
                    .as_str()
                    .is_some_and(|error| error.contains("expired")))
            .count(),
        1
    );
    assert!(session.pending_tools().await.unwrap().is_empty());
    stopped.cancel();
    supervisor.await.unwrap().unwrap();
}

#[cfg(feature = "local")]
#[tokio::test]
#[ignore = "subprocess fixture for native_process_loss_preserves_admission_and_finalization"]
async fn native_process_fixture() {
    let directory =
        std::path::PathBuf::from(std::env::var("EVERRUNS_PROCESS_FIXTURE_DIR").unwrap());
    let mode = std::env::var("EVERRUNS_PROCESS_FIXTURE_MODE").unwrap();
    let id: SessionId = std::env::var("EVERRUNS_PROCESS_FIXTURE_SESSION")
        .unwrap()
        .parse()
        .unwrap();
    let message: MessageId = std::env::var("EVERRUNS_PROCESS_FIXTURE_MESSAGE")
        .unwrap()
        .parse()
        .unwrap();
    let marker = directory.join("completion-effects");
    let definition = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("finished"))
        .on_completion(move |_| {
            let marker = marker.clone();
            async move {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(marker)
                    .unwrap();
                file.write_all(b"effect\n").unwrap();
                file.sync_all().unwrap();
                std::future::pending::<()>().await;
            }
        })
        .build()
        .unwrap();
    let profile = everruns::hosting::SqliteHosting::open(directory.join("authority.db")).unwrap();
    let host = engine(External(profile.clone()), definition);
    if mode == "frontend" {
        let session = host.create_registered(id, &build()).await.unwrap();
        session.send_with_id(message, "finish").await.unwrap();
        std::fs::write(directory.join("accepted"), b"accepted").unwrap();
        std::future::pending::<()>().await;
    } else if mode == "owner" {
        host.activate(id).await.unwrap();
        panic!(
            "owner returned before blocked completion: {:?}",
            profile.status(id).await.unwrap()
        );
    } else {
        profile
            .supervise(host, everruns::CancellationToken::new())
            .await
            .unwrap();
    }
}

#[cfg(feature = "local")]
#[tokio::test]
async fn native_process_loss_preserves_admission_and_finalization() {
    struct Child(std::process::Child);
    impl Drop for Child {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let id = SessionId::new();
    let message = MessageId::new();
    let spawn = |mode: &str| {
        let log = std::fs::File::create(directory.path().join(format!("{mode}.log"))).unwrap();
        Child(
            std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--ignored",
                    "--exact",
                    "native_process_fixture",
                    "--nocapture",
                ])
                .env("EVERRUNS_PROCESS_FIXTURE_DIR", directory.path())
                .env("EVERRUNS_PROCESS_FIXTURE_MODE", mode)
                .env("EVERRUNS_PROCESS_FIXTURE_SESSION", id.to_string())
                .env("EVERRUNS_PROCESS_FIXTURE_MESSAGE", message.to_string())
                .stdout(std::process::Stdio::from(log.try_clone().unwrap()))
                .stderr(std::process::Stdio::from(log))
                .spawn()
                .unwrap(),
        )
    };
    let wait_for = |name: &'static str| {
        let path = directory.path().join(name);
        async move {
            let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while !path.exists() {
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await;
            if result.is_err() {
                let parent = path.parent().unwrap();
                let diagnostics = ["frontend", "owner", "replacement"]
                    .into_iter()
                    .map(|mode| {
                        format!(
                            "{mode}: {}",
                            std::fs::read_to_string(parent.join(format!("{mode}.log")))
                                .unwrap_or_default()
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!("missing {name}: {diagnostics}");
            }
        }
    };
    let frontend = spawn("frontend");
    wait_for("accepted").await;
    drop(frontend);
    let owner = spawn("owner");
    wait_for("completion-effects").await;
    drop(owner);
    let replacement = spawn("replacement");
    // No client calls at all while the independent startup supervisor discovers
    // the expired lease and finalizes the outcome after a real process kill.
    tokio::time::sleep(std::time::Duration::from_secs(12)).await;
    let profile =
        everruns::hosting::SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let reader = Engine::builder()
        .hosting(profile)
        .build()
        .resume(id)
        .await
        .unwrap();
    let admission = reader.admission(message).await.unwrap().unwrap();
    let AdmissionState::Complete { turn: outcome } = admission.state().await.unwrap() else {
        panic!("replacement process did not finalize independently")
    };
    assert_eq!(outcome.response, "finished");
    assert!(outcome.success);
    assert_eq!(
        std::fs::read_to_string(directory.path().join("completion-effects")).unwrap(),
        "effect\n"
    );
    let events = reader.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "input.message")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "reason.started")
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.completed")
            .count(),
        1
    );
    drop(replacement);
}
