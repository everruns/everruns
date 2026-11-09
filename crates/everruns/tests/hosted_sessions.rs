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

#[tokio::test]
async fn unsupported_runtime_format_suspends_before_executing_known_agent() {
    let profile = MemoryHosting::default();
    let engine = engine(profile.clone(), agent());
    let id = SessionId::new();
    profile
        .create(
            id,
            &everruns::hosting::SessionBinding {
                framework: u32::MAX,
                behavior: build(),
                harness: None,
                environment: None,
                metadata: serde_json::Value::Null,
            },
        )
        .await
        .unwrap();
    let session = engine.resume(id).await.unwrap();
    let admission = session
        .send("must not run incompatible state")
        .await
        .unwrap();
    assert!(matches!(admission.wait().await, Err(RunError::Suspended)));
    assert_eq!(
        engine
            .session_status(id)
            .await
            .unwrap()
            .suspension_reason
            .as_deref(),
        Some("unsupported_runtime_version")
    );
    assert!(
        !session
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .any(|event| event.event_type() == "reason.started")
    );
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
        ..Default::default()
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

/// A finished in-process turn reaches its waiter through the host's change
/// hint, not the admission receipt poll. Time is paused, so any wait on the
/// poll would advance the clock by its 50 ms interval; before the hint, every
/// `send_and_wait` on an Engine took at least that long (turn_backends bench:
/// ~51 ms per instant turn).
async fn completed_turns_wake_waiters_without_polling(profile: impl SessionHosting) {
    let engine = engine(profile, agent());
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    for _ in 0..3 {
        session.send_and_wait("hello").await.unwrap();
    }
    assert!(
        started.elapsed() < std::time::Duration::from_millis(50),
        "three instant turns waited {:?} of virtual time",
        started.elapsed()
    );
}

#[tokio::test(start_paused = true)]
async fn memory_completed_turns_wake_waiters_without_polling() {
    completed_turns_wake_waiters_without_polling(MemoryHosting::default()).await;
}

#[cfg(feature = "local")]
#[tokio::test(start_paused = true)]
async fn sqlite_completed_turns_wake_waiters_without_polling() {
    let dir = tempfile::tempdir().unwrap();
    completed_turns_wake_waiters_without_polling(
        everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap(),
    )
    .await;
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

/// EVE-1344: a targeted cancel ends only the turn its admission reaches. A
/// message admitted behind it runs as the next turn instead of being consumed
/// by the cancel with its waiter left pending forever. Activation is manual,
/// so the cancel lands before either admission is consumed on every authority.
async fn cancel_keeps_later_admissions(profile: impl SessionHosting + 'static) {
    let engine = Engine::builder()
        .hosting(External(profile))
        .register(build(), agent())
        .build();
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let first = session.send("cancel this").await.unwrap();
    let queued = session.send("keep this").await.unwrap();
    first.turn().cancel().await.unwrap();
    engine.activate(id).await.unwrap();
    assert_eq!(
        first.wait().await.unwrap().stop_reason,
        everruns::TurnStopReason::Cancelled
    );
    engine.activate(id).await.unwrap();
    let next = tokio::time::timeout(std::time::Duration::from_secs(10), queued.wait())
        .await
        .expect("the queued admission's waiter resolves")
        .unwrap();
    assert!(next.success, "{:?}", next.error);
    assert_eq!(next.response, "Hello!");
}

/// EVE-1344: the same while the turn is running. An authority without a renew
/// poke (SQLite) reports the cancel only at the turn's next inbox boundary,
/// after the queued message arrived; the turn must end there without taking
/// that message as steering.
async fn cancel_of_a_running_turn_keeps_later_admissions(profile: impl SessionHosting) {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let config = everruns::LlmSimConfig::fixed("done")
        .with_response_delay(std::time::Duration::from_millis(500))
        .with_message_capture(calls.clone());
    let agent = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated_with_config(config))
        .build()
        .unwrap();
    let engine = engine(profile, agent);
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    let deadline = std::time::Duration::from_secs(20);
    let first = session.send("first").await.unwrap();
    // Send once the first model call is in flight, past the turn's first take.
    tokio::time::timeout(deadline, async {
        while calls.lock().unwrap().is_empty() {
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the first model call starts");
    let queued = session.send("second").await.unwrap();
    first.turn().cancel().await.unwrap();
    let cancelled = tokio::time::timeout(deadline, first.wait())
        .await
        .expect("cancelled turn resolves")
        .unwrap();
    assert_eq!(cancelled.stop_reason, everruns::TurnStopReason::Cancelled);
    let next = tokio::time::timeout(deadline, queued.wait())
        .await
        .expect("the queued admission's waiter resolves")
        .unwrap();
    assert!(next.success, "{:?}", next.error);
    assert_ne!(next.turn_id, cancelled.turn_id);
}

#[tokio::test]
async fn memory_cancel_keeps_later_admissions() {
    cancel_keeps_later_admissions(MemoryHosting::default()).await;
    cancel_of_a_running_turn_keeps_later_admissions(MemoryHosting::default()).await;
}

#[cfg(feature = "local")]
#[tokio::test]
async fn sqlite_cancel_keeps_later_admissions() {
    let dir = tempfile::tempdir().unwrap();
    cancel_keeps_later_admissions(
        everruns::hosting::SqliteHosting::open(dir.path().join("authority.db")).unwrap(),
    )
    .await;
    cancel_of_a_running_turn_keeps_later_admissions(
        everruns::hosting::SqliteHosting::open(dir.path().join("running.db")).unwrap(),
    )
    .await;
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

#[cfg(feature = "local")]
#[path = "hosted_sessions/branches.rs"]
mod branches;
#[cfg(feature = "local")]
#[path = "hosted_sessions/checkpoints.rs"]
mod checkpoints;
#[path = "hosted_sessions/controls.rs"]
mod controls;
#[cfg(feature = "local")]
#[path = "hosted_sessions/cutover.rs"]
mod cutover;

#[cfg(feature = "otel")]
#[path = "hosted_sessions/causal.rs"]
mod causal;
#[cfg(feature = "local")]
#[path = "hosted_sessions/members.rs"]
mod members;

/// Data plane v2 step 1: an Engine keeps a bounded number of idle hosted
/// sessions in memory; a dropped one is rebuilt from the authority.
#[tokio::test]
async fn idle_hosted_sessions_past_the_cache_capacity_reopen_from_the_authority() {
    let profile = MemoryHosting::default();
    let engine = Engine::builder()
        .hosting(profile.clone())
        .register(build(), agent())
        .session_cache_capacity(2)
        .build();
    let held = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    assert!(held.send_and_wait("held open").await.unwrap().success);
    let mut idle = Vec::new();
    for _ in 0..6 {
        let id = SessionId::new();
        let session = engine.create_registered(id, &build()).await.unwrap();
        assert!(session.send_and_wait("first").await.unwrap().success);
        idle.push(id);
    }
    let cached = format!("{engine:?}");
    assert!(
        cached == "Engine { sessions: 2 }" || cached == "Engine { sessions: 3 }",
        "the cache stays near its capacity: {cached}"
    );
    let completed = |events: Vec<everruns::SessionEvent>| {
        events
            .iter()
            .filter(|event| event.event_type() == "output.message.completed")
            .count()
    };
    let first = engine.resume(idle[0]).await.unwrap();
    assert!(first.send_and_wait("second").await.unwrap().success);
    assert_eq!(completed(first.events_after(0).await.unwrap()), 2);
    // The open handle kept its session cached and working.
    assert!(held.send_and_wait("again").await.unwrap().success);
    assert_eq!(completed(held.events_after(0).await.unwrap()), 2);
}

/// Data plane v2 step 1: a host whose watchers are elsewhere gets the live
/// token deltas, fenced to the epoch and the stream they belong to.
#[tokio::test]
async fn live_deltas_reach_the_host_fenced_to_epoch_and_stream() {
    #[derive(Clone, Default)]
    struct Collect(Arc<std::sync::Mutex<Vec<everruns::hosting::LiveDelta>>>);
    impl everruns::hosting::LiveDeltaSink for Collect {
        fn publish(&self, delta: everruns::hosting::LiveDelta) {
            self.0.lock().unwrap().push(delta);
        }
    }
    let deltas = Collect::default();
    let profile = MemoryHosting::default().with_live_deltas(deltas.clone());
    let engine = engine(profile, agent());
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    assert!(
        session
            .send_and_wait("stream please")
            .await
            .unwrap()
            .success
    );
    let events = session.events_after(0).await.unwrap();
    let completed = events
        .iter()
        .find(|event| event.event_type() == "output.message.completed")
        .expect("an answer");
    let deltas = deltas.0.lock().unwrap().clone();
    assert!(!deltas.is_empty(), "the answer streamed");
    for (index, delta) in deltas.iter().enumerate() {
        assert_eq!(delta.session, id);
        assert_eq!(delta.fence, 1, "the first epoch's fence");
        assert_eq!(delta.seq, index as u64, "in order, none skipped");
        assert_eq!(delta.event.event_type, "output.message.delta");
        assert!(delta.event.sequence.is_none(), "never stored");
        assert!(delta.attempt.turn.is_some());
    }
    let message = completed.canonical_json()["data"]["message"]["id"]
        .as_str()
        .expect("message id")
        .to_string();
    assert!(deltas.iter().all(|delta| delta.attempt.stream == message));
}

/// An authority that hands every activation a newer agent build, as one that
/// moved the session to a new revision would.
#[derive(Clone)]
/// A hosting profile whose activations run another agent build when one is
/// set, and forward the trusted host extensions it holds.
struct Rebind<H>(
    H,
    Arc<std::sync::Mutex<Option<AgentBuild>>>,
    everruns_core::host::HostExtensions,
);
#[async_trait::async_trait]
impl<H: SessionHosting> SessionHosting for Rebind<H> {
    async fn command(
        &self,
        session: SessionId,
        command: everruns::hosting::spi::session_commands::SessionCommand,
    ) -> everruns::hosting::spi::error::Result<serde_json::Value> {
        self.0.command(session, command).await
    }
    async fn query_snapshot(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::SessionQuerySnapshot> {
        self.0.query_snapshot(id).await
    }
    async fn status(
        &self,
        id: SessionId,
    ) -> everruns_contracts::error::Result<everruns::hosting::HostingStatus> {
        self.0.status(id).await
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
    fn changes(&self, session: SessionId) -> Option<Arc<tokio::sync::Notify>> {
        self.0.changes(session)
    }
    fn definitions(&self) -> Option<&dyn everruns::hosting::AgentDefinitions> {
        self.0.definitions()
    }
    fn process_scheduler(&self) -> bool {
        self.0.process_scheduler()
    }
    async fn claim(
        &self,
        id: SessionId,
        holder: &str,
        ttl: std::time::Duration,
    ) -> everruns_contracts::error::Result<Option<everruns::hosting::SessionActivation>> {
        let mut activation = self.0.claim(id, holder, ttl).await?;
        if let (Some(activation), Some(build)) = (&mut activation, self.1.lock().unwrap().clone()) {
            activation.binding.behavior = build;
        }
        Ok(activation)
    }
    async fn prepare_activation(
        &self,
        engine: &Engine,
        session: SessionId,
        environment: &everruns::Environment,
        backends: everruns_core::host::HostBackends,
    ) -> everruns_contracts::error::Result<everruns_core::host::HostBackends> {
        let backends = self
            .0
            .prepare_activation(engine, session, environment, backends)
            .await?;
        Ok(backends.with_extensions(self.2.clone()))
    }
}

/// Data plane v2 step 1: a turn finishes with the agent build it started
/// with. An activation that brings another one to an unfinished turn
/// suspends the session instead of finishing it under different code.
#[tokio::test]
async fn an_unfinished_turn_never_resumes_under_another_agent_build() {
    let profile = MemoryHosting::default();
    let newer = AgentBuild::new("assistant", "test-v2");
    let engine = Engine::builder()
        .hosting(Rebind(
            profile.clone(),
            Arc::new(std::sync::Mutex::new(Some(newer.clone()))),
            Default::default(),
        ))
        .register(build(), agent())
        .register(newer, agent())
        .build();
    let id = SessionId::new();
    engine.create_registered(id, &build()).await.unwrap();
    // Admitted straight at the authority, so no executor starts at once.
    let input = everruns_contracts::runtime::message::RuntimeMessage::user("hello");
    let message = input.id;
    profile.admit(id, input, false).await.unwrap();
    // An executor that started the turn under v1, then crashed.
    let lease = std::time::Duration::from_millis(200);
    let crashed = profile.claim(id, "crashed", lease).await.unwrap().unwrap();
    let started = everruns::hosting::TurnAssignment {
        turn_id: everruns::TurnId::new(),
        steered: false,
        revision: Some(everruns::hosting::TurnRevision {
            behavior: build(),
            framework: crashed.binding.framework,
        }),
    };
    assert!(
        crashed
            .epoch
            .commit(
                &format!("assignment/{message}"),
                serde_json::to_value(&started).unwrap(),
                true,
            )
            .await
            .unwrap()
    );
    tokio::time::sleep(lease * 2).await;
    // The driver reports the refusal through the session, not the call.
    let _ = engine.activate(id).await;
    let status = engine.session_status(id).await.unwrap();
    assert_eq!(
        status.suspension_reason.as_deref(),
        Some("turn_revision_mismatch")
    );
}

/// A finished turn's session moves to the newer build for its next turn.
#[tokio::test]
async fn a_new_turn_runs_the_build_the_authority_moved_the_session_to() {
    let newer = AgentBuild::new("assistant", "test-v2");
    let moved = Arc::new(std::sync::Mutex::new(None));
    let engine = Engine::builder()
        .hosting(Rebind(
            MemoryHosting::default(),
            moved.clone(),
            Default::default(),
        ))
        .register(build(), agent())
        .register(
            newer.clone(),
            Agent::builder()
                .instructions("Answer briefly.")
                .model(Model::simulated("Hello from v2!"))
                .build()
                .unwrap(),
        )
        .build();
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    assert_eq!(
        session.send_and_wait("hi").await.unwrap().response,
        "Hello!"
    );
    *moved.lock().unwrap() = Some(newer);
    assert_eq!(
        session.send_and_wait("hi").await.unwrap().response,
        "Hello from v2!"
    );
}

/// Data plane v2 step 1: the trusted host that admits an input says who it
/// acts for, and the context stays with the input for the turn's host code.
#[tokio::test]
async fn an_input_keeps_who_it_acts_for() {
    let engine = engine(MemoryHosting::default(), agent());
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let context = everruns::hosting::InputContext {
        author: Some("principal_alice".into()),
        participants: vec!["principal_alice".into(), "principal_bob".into()],
        grants: vec!["mcp:alice-github".into()],
    };
    let message = MessageId::new();
    let sent = session
        .send_as(message, "hi", context.clone())
        .await
        .unwrap();
    assert!(sent.wait().await.unwrap().success);
    assert_eq!(
        engine.input_context(id, message).await.unwrap(),
        Some(context)
    );
    // An input admitted without one has none; a retry under another
    // context is a different command.
    let plain = MessageId::new();
    assert!(
        session
            .send_with_id(plain, "hello")
            .await
            .unwrap()
            .wait()
            .await
            .unwrap()
            .success
    );
    assert_eq!(engine.input_context(id, plain).await.unwrap(), None);
    assert!(
        session
            .send_as(message, "hi", everruns::hosting::InputContext::default())
            .await
            .is_err()
    );
}

/// Data plane v2 step 1: an Engine that lacks an execution-critical protocol
/// capability the session's journal uses hands the epoch back untouched,
/// without suspending the session, so an executor that has it runs it.
#[tokio::test]
async fn an_engine_lacking_a_required_protocol_capability_hands_the_session_back() {
    let profile = MemoryHosting::default().requiring(&["approval.v9"]);
    let lacking = engine(profile.clone(), agent());
    let id = SessionId::new();
    lacking.create_registered(id, &build()).await.unwrap();
    profile
        .admit(
            id,
            everruns_contracts::runtime::message::RuntimeMessage::user("hello"),
            false,
        )
        .await
        .unwrap();
    let refused = lacking.activate(id).await.unwrap_err();
    assert!(refused.to_string().contains("approval.v9"), "{refused}");
    let status = lacking.session_status(id).await.unwrap();
    assert!(status.suspension_reason.is_none());
    assert!(status.runnable, "the input waits for a capable executor");
    let capable = engine(profile.requiring(&[]), agent());
    capable.activate(id).await.unwrap();
    assert!(!lacking.session_status(id).await.unwrap().runnable);
}

/// Data plane v2 step 1 gate: with steering, every provider call attempt is
/// settled exactly once. Input admitted while a model call runs must neither
/// lose that call's `llm.generation` (its usage) nor run it twice.
#[tokio::test]
async fn with_steering_every_model_call_is_settled_exactly_once() {
    let engine = engine(
        MemoryHosting::default(),
        Agent::builder()
            .instructions("Answer briefly.")
            .model(Model::simulated_with_config(
                everruns::LlmSimConfig::lorem(60).with_latency(),
            ))
            .build()
            .unwrap(),
    );
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    let mut sent = Vec::new();
    for round in 0..3 {
        sent.push(session.send(format!("start {round}")).await.unwrap());
        for steer in 0..3 {
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
            sent.push(
                session
                    .send(format!("steer {round}.{steer}"))
                    .await
                    .unwrap(),
            );
        }
        for message in sent.drain(..) {
            message.wait().await.unwrap();
        }
    }
    let events = session.events_after(0).await.unwrap();
    let exec = |event: &everruns::SessionEvent| {
        event.canonical_json()["context"]["exec_id"]
            .as_str()
            .map(str::to_owned)
    };
    let mut started: Vec<_> = events
        .iter()
        .filter(|event| event.event_type() == "reason.started")
        .map(exec)
        .collect();
    let mut settled: Vec<_> = events
        .iter()
        .filter(|event| event.event_type() == "llm.generation")
        .map(exec)
        .collect();
    let steered = events
        .iter()
        .filter(|event| event.event_type() == "input.message")
        .count();
    assert_eq!(steered, 12);
    assert!(
        started.len() > 3,
        "steering added model calls: {}",
        started.len()
    );
    started.sort();
    settled.sort();
    assert_eq!(started, settled, "one settlement per model call");
}

/// Data plane v2 step 1: a provider call is recorded before it is
/// dispatched. `reason.started` commits behind the context assembly, so on
/// an authority whose behind writes are slow the call must wait for it;
/// otherwise an executor killed during the call leaves no record of it.
#[tokio::test]
async fn a_model_call_is_in_the_log_before_it_is_dispatched() {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let engine = engine(
        MemoryHosting::default().with_behind_latency(std::time::Duration::from_millis(300)),
        Agent::builder()
            .instructions("Answer briefly.")
            .model(Model::simulated_with_config(
                everruns::LlmSimConfig::fixed("done")
                    .with_message_capture(calls.clone())
                    .with_response_delay(std::time::Duration::from_secs(3600)),
            ))
            .build()
            .unwrap(),
    );
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    let _turn = session.send("hello").await.unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while calls.lock().unwrap().is_empty() {
        assert!(std::time::Instant::now() < deadline, "no model call");
        tokio::time::sleep(std::time::Duration::from_millis(1)).await;
    }
    // The call is out: the authority already holds its record.
    let events = session.events_after(0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type() == "reason.started"),
        "dispatched before its reason.started was stored: {:?}",
        events.iter().map(|e| e.event_type()).collect::<Vec<_>>()
    );
}

struct Exhausted;
#[async_trait::async_trait]
impl everruns_core::tool_execution::BudgetChecker for Exhausted {
    async fn check_budgets(
        &self,
        _session_id: &str,
    ) -> everruns_contracts::error::Result<everruns_contracts::runtime::budget::BudgetToolResponse>
    {
        Ok(everruns_contracts::runtime::budget::BudgetToolResponse {
            status: "exhausted".into(),
            budgets: Vec::new(),
            hint: None,
        })
    }
}

/// Data plane v2 step 2: a trusted host forwards platform services to the
/// turn through its activation hook. Its budget authority reaches the gate
/// before every model call, so an exhausted budget ends the turn without one.
#[tokio::test]
async fn a_trusted_hosts_budget_authority_stops_a_turn_before_the_model_call() {
    let calls = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mut extensions = everruns_core::host::HostExtensions::default();
    extensions.authorities.budget = Some(Arc::new(|_| {
        Some(Arc::new(Exhausted) as Arc<dyn everruns_core::tool_execution::BudgetChecker>)
    }));
    let engine = engine(
        Rebind(MemoryHosting::default(), Arc::default(), extensions),
        Agent::builder()
            .instructions("Answer.")
            .model(Model::simulated_with_config(
                everruns::LlmSimConfig::fixed("spent").with_message_capture(calls.clone()),
            ))
            .build()
            .unwrap(),
    );
    let session = engine
        .create_registered(SessionId::new(), &build())
        .await
        .unwrap();
    let outcome = session.send("hello").await.unwrap().wait().await.unwrap();
    assert!(!outcome.success, "{outcome:?}");
    assert!(calls.lock().unwrap().is_empty(), "no model call");
    let events = session.events_after(0).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type() == "budget.exhausted"),
        "{:?}",
        events.iter().map(|e| e.event_type()).collect::<Vec<_>>()
    );
}
