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

#[path = "support/session_hosting.rs"]
mod session_hosting;
use session_hosting::External;

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
    // The first owner's in-lease write races its wall-clock lease, so the lease must
    // comfortably outlive one SQLite transaction on a loaded CI runner (a 5ms lease
    // expired before the write landed). The replacement then polls until the lease
    // has actually lapsed instead of guessing a fixed sleep.
    let lease = std::time::Duration::from_millis(500);
    let stale = first
        .claim(id, "first-owner", lease)
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
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let current = loop {
        if let Some(current) = second
            .claim(id, "replacement", std::time::Duration::from_secs(10))
            .await
            .unwrap()
        {
            break current;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "first owner's lease never expired"
        );
        tokio::time::sleep(lease / 10).await;
    };
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

#[path = "hosted_sessions/controls.rs"]
mod controls;
