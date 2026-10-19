//! EVE-1298: lossless cutover of legacy local/custom-backend sessions into the
//! coherent SQLite authority, and the process-loss probes around it.
use super::*;
use everruns::hosting::spi::{EventReadLimit, EventReadRequest, HostBackends};
use everruns::hosting::{ImportedWorkspaceFactory, SqliteHosting};
use everruns::{ResumeError, ToolAnswer, ToolCall};
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

fn target_build() -> AgentBuild {
    AgentBuild::new("assistant", "cutover-v1")
}

fn legacy_agent(backends: &HostBackends) -> Agent {
    Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .backends(backends.clone())
        .build()
        .unwrap()
}

fn hosting_engine(profile: SqliteHosting, agent: Agent) -> Engine {
    Engine::builder()
        .hosting(profile)
        .register(target_build(), agent)
        .build()
}

/// Every persisted legacy event: id, sequence and type.
async fn legacy_journal(backends: &HostBackends, id: SessionId) -> Vec<(String, i32, String)> {
    let page = backends
        .event_log
        .read_page(EventReadRequest::new(
            id,
            EventReadLimit::new(1000).unwrap(),
        ))
        .await
        .unwrap();
    assert!(page.next_cursor.is_none());
    page.events
        .iter()
        .map(|event| {
            (
                event.id.to_string(),
                event.sequence.unwrap(),
                event.event_type.clone(),
            )
        })
        .collect()
}

async fn hosted_journal(session: &everruns::Session) -> Vec<(String, i32, String)> {
    session
        .events_after(0)
        .await
        .unwrap()
        .iter()
        .map(|event| {
            let envelope = event.canonical_json();
            (
                envelope["id"].as_str().unwrap().to_owned(),
                envelope["sequence"].as_i64().unwrap() as i32,
                event.event_type().to_owned(),
            )
        })
        .collect()
}

fn input_ids(journal: &[everruns::SessionEvent]) -> Vec<MessageId> {
    journal
        .iter()
        .filter(|event| event.event_type() == "input.message")
        .map(|event| {
            serde_json::from_value(event.canonical_json()["data"]["message"]["id"].clone()).unwrap()
        })
        .collect()
}

#[tokio::test]
async fn custom_backend_session_cuts_over_losslessly_and_never_falls_back() {
    let backends = HostBackends::in_memory();
    let legacy = legacy_agent(&backends);
    let legacy_engine = Engine::new();
    let session = legacy_engine.create(legacy.clone());
    let id = session.session_id();
    assert!(session.run("first").await.unwrap().success);
    assert!(session.run("second").await.unwrap().success);
    let before = legacy_journal(&backends, id).await;
    assert!(before.len() > 4);

    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let engine = hosting_engine(SqliteHosting::open(&path).unwrap(), agent());
    let report = engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    assert!(!report.replayed);
    assert_eq!(report.boundary.events, before.len());
    assert_eq!(report.boundary.last_sequence, before.last().unwrap().1);

    // A retried cutover (lost reply) confirms the same boundary.
    let retried = engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    assert!(retried.replayed);
    assert_eq!(retried.boundary, report.boundary);

    // Identity and the whole journal survive: ids, sequences and order.
    let hosted = engine.resume(id).await.unwrap();
    assert_eq!(hosted_journal(&hosted).await, before);
    assert_eq!(hosted.history().page().await.unwrap().messages.len(), 4);

    // Admissions made on the legacy lane resolve through the authority, and a
    // retried legacy send replays its receipt instead of admitting again.
    let journal = hosted.events_after(0).await.unwrap();
    let inputs = input_ids(&journal);
    assert_eq!(inputs.len(), 2);
    let legacy_admission = hosted.admission(inputs[0]).await.unwrap().unwrap();
    assert!(legacy_admission.wait().await.unwrap().success);
    assert_eq!(
        hosted
            .send_with_id(inputs[1], "second")
            .await
            .unwrap()
            .message_id,
        inputs[1].to_string()
    );
    assert_eq!(hosted.events_after(0).await.unwrap().len(), before.len());

    // The authority continues the same conversation after the boundary.
    let next = hosted.send("third").await.unwrap();
    let turn = tokio::time::timeout(WAIT, next.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(turn.success);
    let after = hosted_journal(&hosted).await;
    assert_eq!(after[..before.len()], before[..]);
    assert!(after[before.len()].1 > report.boundary.last_sequence);
    assert_eq!(hosted.history().page().await.unwrap().messages.len(), 6);

    // No fallback: the legacy executor refuses the session in a fresh Engine
    // and through the handle that still points at it.
    assert!(matches!(
        Engine::new().attach(id, legacy_agent(&backends)).await,
        Err(ResumeError::CutOver)
    ));
    assert!(session.run("legacy must not run").await.is_err());
    assert_eq!(legacy_journal(&backends, id).await, before);

    // A different target cannot take the session over.
    let other = Engine::builder()
        .hosting(SqliteHosting::open(&path).unwrap())
        .register(AgentBuild::new("assistant", "cutover-v2"), agent())
        .build();
    assert!(
        other
            .cut_over_legacy_session(id, &legacy, &AgentBuild::new("assistant", "cutover-v2"))
            .await
            .is_err()
    );

    // Restart: a new authority connection and Engine reopen the same state.
    drop(hosted);
    drop(engine);
    let restarted = hosting_engine(SqliteHosting::open(&path).unwrap(), agent());
    let reopened = restarted.resume(id).await.unwrap();
    assert_eq!(hosted_journal(&reopened).await, after);
    assert_eq!(
        restarted
            .cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .unwrap()
            .boundary,
        report.boundary
    );
}

/// Interrupted import: a failure inside the authority transaction commits
/// nothing, the legacy lane stays frozen (no automatic fallback), and only an
/// explicit abandon returns ownership to it.
#[tokio::test]
async fn interrupted_import_commits_nothing_and_keeps_the_legacy_lane_frozen() {
    use everruns::hosting::spi::events::EventRequest;
    use everruns::hosting::spi::{CompactionCheckpoint, CompactionCheckpointPayload};
    let backends = HostBackends::in_memory();
    let legacy = legacy_agent(&backends);
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    drop(session);
    // A checkpoint that claims a source beyond the legacy journal cannot be
    // imported; the authority rejects it after writing the journal rows.
    let compacted: everruns_core::events::ContextCompactedData =
        serde_json::from_value(serde_json::json!({
            "strategy_used": "native",
            "messages_before": 2,
            "messages_after": 1,
            "duration_ms": 1,
            "model": "sim-model",
            "provider": "sim",
        }))
        .unwrap();
    backends
        .event_log
        .append(EventRequest::new(
            id,
            everruns::hosting::spi::events::EventContext::empty(),
            compacted,
        ))
        .await
        .unwrap();
    backends
        .compaction_checkpoint_store
        .install(CompactionCheckpoint {
            id: uuid::Uuid::now_v7(),
            session_id: id,
            source_sequence: 10_000,
            provider_type: "sim".into(),
            model: "sim-model".into(),
            format_version: 1,
            payload: CompactionCheckpointPayload::Summary {
                text: "beyond the journal".into(),
            },
        })
        .await
        .unwrap();
    let before = legacy_journal(&backends, id).await;

    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let engine = hosting_engine(profile.clone(), agent());
    assert!(
        engine
            .cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .is_err()
    );
    assert!(matches!(
        profile.status(id).await,
        Err(everruns_contracts::error::AgentLoopError::SessionNotFound(
            _
        ))
    ));
    assert!(matches!(
        Engine::new().attach(id, legacy_agent(&backends)).await,
        Err(ResumeError::CutOver)
    ));
    assert_eq!(legacy_journal(&backends, id).await, before);

    // The explicit ownership decision hands it back.
    engine.abandon_legacy_cutover(id, &legacy).await.unwrap();
    let legacy_engine = Engine::new();
    legacy_engine
        .attach(id, legacy_agent(&backends))
        .await
        .unwrap();
    assert!(
        legacy_engine
            .resume(id)
            .await
            .unwrap()
            .run("legacy again")
            .await
            .unwrap()
            .success
    );
}

/// A committed cutover cannot be abandoned back to the legacy lane.
#[tokio::test]
async fn committed_cutover_cannot_be_abandoned() {
    let backends = HostBackends::in_memory();
    let legacy = legacy_agent(&backends);
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let engine = hosting_engine(
        SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
        agent(),
    );
    engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    assert!(engine.abandon_legacy_cutover(id, &legacy).await.is_err());
    assert!(matches!(
        Engine::new().attach(id, legacy_agent(&backends)).await,
        Err(ResumeError::CutOver)
    ));
}

const SECRET_KEY: &str = "k1:AAECAwQFBgcICQoLDA0ODxAREhMUFRYXGBkaGxwdHh8=";
const NEXT_SECRET_KEY: &str = "k2:ICEiIyQlJicoKSorLC0uLzAxMjM0NTY3ODk6Ozw9Pj8=";

/// Session secrets move sealed under the authority's own keys. Without keys
/// the cutover refuses before any authority write instead of dropping them.
#[tokio::test]
async fn session_secrets_cut_over_encrypted_under_the_authority_keys() {
    let backends = HostBackends::in_memory();
    let legacy = legacy_agent(&backends);
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    backends
        .storage_store
        .set_secret(id, "token", "s3cr3t-value")
        .await
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");

    let keyless = SqliteHosting::open(&path).unwrap();
    let engine = hosting_engine(keyless.clone(), agent());
    let error = engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap_err();
    assert!(error.to_string().contains("secret keys"), "{error}");
    assert!(keyless.status(id).await.is_err());

    let keyed = SqliteHosting::open(&path)
        .unwrap()
        .with_secret_keys(SECRET_KEY, &[])
        .unwrap();
    let engine = hosting_engine(keyed.clone(), agent());
    let report = engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    assert_eq!(report.secrets, 1);
    let reader = keyed.reader(id).storage_store;
    assert_eq!(
        reader.get_secret(id, "token").await.unwrap().as_deref(),
        Some("s3cr3t-value")
    );
    assert_eq!(reader.list_secrets(id).await.unwrap()[0].name, "token");

    // Never stored as plaintext.
    let raw = std::fs::read(&path).unwrap();
    assert!(!raw.windows(12).any(|window| window == b"s3cr3t-value"));

    // Rotation: a new primary key still opens values sealed with the old one.
    let rotated = SqliteHosting::open(&path)
        .unwrap()
        .with_secret_keys(NEXT_SECRET_KEY, &[SECRET_KEY])
        .unwrap();
    assert_eq!(
        rotated
            .reader(id)
            .storage_store
            .get_secret(id, "token")
            .await
            .unwrap()
            .as_deref(),
        Some("s3cr3t-value")
    );
    // A wrong key does not open it, and readers cannot write.
    let wrong = SqliteHosting::open(&path)
        .unwrap()
        .with_secret_keys(NEXT_SECRET_KEY, &[])
        .unwrap();
    assert!(
        wrong
            .reader(id)
            .storage_store
            .get_secret(id, "token")
            .await
            .is_err()
    );
    assert!(reader.set_secret(id, "token", "other").await.is_err());
}

/// Pending client tool results parked on the legacy lane reopen as durable
/// authority waits, resolve through the Session API and finish the original
/// admission, with no duplicate turn.
#[tokio::test]
async fn parked_legacy_client_calls_resolve_under_the_authority() {
    // The legacy model asks for the client tool once; the target's model is a
    // fresh simulator, so it only answers after the result.
    fn calling(backends: Option<&HostBackends>) -> Agent {
        let calls = match backends {
            Some(_) => vec![
                vec![ToolCall {
                    id: "call_one".into(),
                    name: "client_one".into(),
                    arguments: serde_json::json!({}),
                }],
                vec![],
            ],
            None => vec![vec![]],
        };
        let builder = Agent::builder()
            .instructions("Use the client tool.")
            .model(Model::simulated_with_config(
                everruns::LlmSimConfig::fixed("Resolved.").with_tool_call_sequence(calls),
            ))
            .client_tool(
                "client_one",
                "External operation",
                serde_json::json!({"type":"object","properties":{}}),
            );
        match backends {
            Some(backends) => builder.backends(backends.clone()),
            None => builder,
        }
        .build()
        .unwrap()
    }
    let backends = HostBackends::in_memory();
    let legacy = calling(Some(&backends));
    let id;
    {
        let legacy_engine = Engine::new();
        let session = legacy_engine.create(legacy.clone());
        id = session.session_id();
        let _sent = session.send("call it").await.unwrap();
        tokio::time::timeout(WAIT, async {
            loop {
                if legacy_journal(&backends, id)
                    .await
                    .iter()
                    .any(|(_, _, kind)| kind == "tool.call_requested")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        // The legacy process goes away while the turn is parked.
    }
    let directory = tempfile::tempdir().unwrap();
    let engine = hosting_engine(
        SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
        calling(None),
    );
    let report = tokio::time::timeout(WAIT, async {
        loop {
            match engine
                .cut_over_legacy_session(id, &legacy, &target_build())
                .await
            {
                Ok(report) => break report,
                // The dropped legacy actor may still be winding down.
                Err(error) if error.to_string().contains("executing") => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("{error}"),
            }
        }
    })
    .await
    .unwrap();
    assert!(!report.replayed);
    let status = engine.session_status(id).await.unwrap();
    assert_eq!(status.waiting.len(), 1);
    let turn_id = status.open_turn.expect("parked turn stays open");
    let session = engine.resume(id).await.unwrap();
    let input = input_ids(&session.events_after(0).await.unwrap())[0];
    let admission = session.admission(input).await.unwrap().unwrap();
    session
        .resolve_tools(
            uuid::Uuid::new_v4(),
            turn_id,
            vec![ToolAnswer {
                tool_call_id: "call_one".into(),
                tool_name: "client_one".into(),
                result: serde_json::json!("done"),
                error: None,
            }],
        )
        .await
        .unwrap();
    let outcome = tokio::time::timeout(WAIT, admission.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(outcome.response, "Resolved.");
    let events = session.events_after(0).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type() == "turn.started")
            .count(),
        1
    );
}

/// An uncertain external effect cut off by legacy process loss is not
/// replayed by the authority: the interrupted call is never executed again.
#[tokio::test]
async fn interrupted_legacy_tool_is_not_replayed_after_cutover() {
    use everruns::hosting::spi::events::{EventContext, EventRequest, InputMessageData};
    use everruns_core::events::{ToolStartedData, TurnStartedData};
    let runs = Arc::new(AtomicUsize::new(0));
    let counted = runs.clone();
    let target = Agent::builder()
        .instructions("Answer.")
        .model(Model::simulated("Recovered."))
        .tool(everruns::FunctionTool::new(
            "charge_card",
            "Charge the card once.",
            serde_json::json!({"type":"object","properties":{}}),
            move |_| {
                counted.fetch_add(1, Ordering::SeqCst);
                async { Ok::<_, String>(serde_json::json!("charged")) }
            },
        ))
        .build()
        .unwrap();
    let backends = HostBackends::in_memory();
    let legacy = legacy_agent(&backends);
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    drop(session);
    // The legacy process died after starting the effect, before its result.
    let input = everruns_core::RuntimeMessage::user("charge it");
    let input_id = input.id;
    let turn = everruns::TurnId::new();
    let context = EventContext::turn(turn, input_id);
    for request in [
        EventRequest::new(id, context.clone(), InputMessageData::new(input)),
        EventRequest::new(
            id,
            context.clone(),
            serde_json::from_value::<TurnStartedData>(
                serde_json::json!({"turn_id": turn, "input_message_id": input_id}),
            )
            .unwrap(),
        ),
        EventRequest::new(
            id,
            context.clone(),
            serde_json::from_value::<ToolStartedData>(serde_json::json!({
                "tool_call": {"id": "call_charge", "name": "charge_card", "arguments": {}},
            }))
            .unwrap(),
        ),
    ] {
        backends.event_log.append(request).await.unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let engine = hosting_engine(
        SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
        target,
    );
    engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    let status = engine.session_status(id).await.unwrap();
    assert_eq!(status.open_turn, Some(turn));
    assert!(!status.runnable, "the cutover itself starts no work");
    let hosted = engine.resume(id).await.unwrap();
    let next = hosted.send("continue").await.unwrap();
    tokio::time::timeout(WAIT, next.wait())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(runs.load(Ordering::SeqCst), 0, "uncertain effect replayed");
}

/// Expired-epoch writes stay fenced on an imported session.
#[tokio::test]
async fn imported_sessions_fence_writes_from_an_expired_epoch() {
    let backends = HostBackends::in_memory();
    let legacy = legacy_agent(&backends);
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let first = SqliteHosting::open(&path).unwrap();
    let engine = hosting_engine(first.clone(), agent());
    engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    first
        .admit(id, everruns_core::RuntimeMessage::user("owed"), false)
        .await
        .unwrap();
    let lease = Duration::from_millis(300);
    let stale = first.claim(id, "stale", lease).await.unwrap().unwrap();
    let second = SqliteHosting::open(&path).unwrap();
    let deadline = std::time::Instant::now() + WAIT;
    let current = loop {
        if let Some(current) = second
            .claim(id, "replacement", Duration::from_secs(10))
            .await
            .unwrap()
        {
            break current;
        }
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(lease / 10).await;
    };
    assert!(current.actor.fence > stale.actor.fence);
    assert!(
        stale
            .epoch
            .backends()
            .storage_store
            .set_value(id, "stale", "write")
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
}

/// Admission before response loss after cutover: the accepted command
/// survives the frontend, is recovered by startup supervision without client
/// traffic, and a retry with the same identity replays it.
#[tokio::test]
async fn admission_after_cutover_survives_response_loss_and_restart() {
    let backends = HostBackends::in_memory();
    let legacy = legacy_agent(&backends);
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let message = MessageId::new();
    {
        let engine = Engine::builder()
            .hosting(External(SqliteHosting::open(&path).unwrap()))
            .register(target_build(), agent())
            .build();
        // External forwards no import; cut over through a direct handle.
        hosting_engine(SqliteHosting::open(&path).unwrap(), agent())
            .cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .unwrap();
        engine
            .resume(id)
            .await
            .unwrap()
            .send_with_id(message, "after cutover")
            .await
            .unwrap();
        // Process loss before the reply reaches the client.
    }
    let profile = SqliteHosting::open(&path).unwrap();
    let replacement = hosting_engine(profile.clone(), agent());
    let stop = everruns::CancellationToken::new();
    let supervisor = {
        let engine = replacement.clone();
        let stop = stop.clone();
        tokio::spawn(async move { profile.supervise(engine, stop).await })
    };
    let resumed = replacement.resume(id).await.unwrap();
    assert_eq!(
        resumed
            .send_with_id(message, "after cutover")
            .await
            .unwrap()
            .message_id,
        message.to_string()
    );
    let outcome = tokio::time::timeout(
        WAIT,
        resumed.admission(message).await.unwrap().unwrap().wait(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(outcome.success);
    stop.cancel();
    supervisor.await.unwrap().unwrap();
    assert_eq!(
        resumed
            .events_after(0)
            .await
            .unwrap()
            .iter()
            .filter(|event| event.event_type() == "input.message")
            .count(),
        2
    );
}

/// Workspace metadata survives: the legacy head is reopened by the authority,
/// and a factory that opens another head fails instead of moving the session.
#[tokio::test]
async fn cutover_reopens_the_legacy_workspace_head() {
    let workspace = tempfile::tempdir().unwrap();
    let backends = HostBackends::in_memory();
    let legacy = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .workspace(workspace.path())
        .backends(backends.clone())
        .build()
        .unwrap();
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    let legacy_head = session.workspace_head().unwrap().binding().clone();
    drop(session);

    let target = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .workspace(workspace.path())
        .build()
        .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();

    // Without a retained Environment factory the workspace would be lost.
    let bare = hosting_engine(profile.clone(), target.clone());
    assert!(
        bare.cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .is_err()
    );
    bare.abandon_legacy_cutover(id, &legacy).await.unwrap();

    let engine = Engine::builder().hosting(profile.clone()).build();
    engine
        .register_agent_with_environment(
            target_build(),
            target.clone(),
            None,
            "imported-workspace",
            ImportedWorkspaceFactory::new(target.clone(), profile.clone()),
        )
        .unwrap();
    engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    let hosted = engine.resume(id).await.unwrap();
    let next = hosted.send("second").await.unwrap();
    assert!(
        tokio::time::timeout(WAIT, next.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    let reopened = everruns::hosting::EnvironmentFactory::open(
        &ImportedWorkspaceFactory::new(target, profile),
        id,
    )
    .await
    .unwrap();
    assert_eq!(reopened.workspace_head().binding(), &legacy_head);
}

/// EVE-1299 finding: caller-owned backends keep the workspace binding, so a
/// new Agent value over the same backends reopens the same head instead of
/// provisioning a fresh default workspace.
#[tokio::test]
async fn a_new_agent_value_over_the_same_backends_reopens_the_bound_head() {
    let workspace = tempfile::tempdir().unwrap();
    let backends = HostBackends::in_memory();
    let build = || {
        Agent::builder()
            .instructions("Answer briefly.")
            .model(Model::simulated("Hello!"))
            .workspace(workspace.path())
            .backends(backends.clone())
            .build()
            .unwrap()
    };
    let session = Engine::new().create(build());
    let id = session.session_id();
    session.run("first").await.unwrap();
    let head = session.workspace_head().unwrap().binding().clone();
    drop(session);
    let engine = Engine::new();
    engine.attach(id, build()).await.unwrap();
    let resumed = engine.resume(id).await.unwrap();
    resumed.run("second").await.unwrap();
    assert_eq!(resumed.workspace_head().unwrap().binding(), &head);
}

/// The JSONL + SQLite local profile cuts over the same way: its workspace head
/// is reopened by the authority, JSONL is never written again (the authority
/// is the only writer), and every process on the profile refuses the session
/// on the legacy lane.
#[tokio::test]
async fn local_profile_session_cuts_over_and_jsonl_stops_being_a_writer() {
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let legacy_agent = || {
        Agent::builder()
            .instructions("Answer briefly.")
            .model(Model::simulated("Hello!"))
            .local(everruns::LocalConfig::new(root.path().join("state")).workspace(&workspace))
            .build()
            .unwrap()
    };
    let legacy = legacy_agent();
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    let legacy_head = session.workspace_head().unwrap().binding().clone();
    drop(session);
    let jsonl = root.path().join("state").join("events.jsonl");
    let journal = std::fs::read(&jsonl).unwrap();

    let target = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .workspace(&workspace)
        .build()
        .unwrap();
    let profile = SqliteHosting::open(root.path().join("authority.db")).unwrap();
    let engine = Engine::builder().hosting(profile.clone()).build();
    engine
        .register_agent_with_environment(
            target_build(),
            target.clone(),
            None,
            "imported-workspace",
            ImportedWorkspaceFactory::new(target.clone(), profile.clone()),
        )
        .unwrap();
    let report = engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    assert!(!report.volatile_workspace);
    let hosted = engine.resume(id).await.unwrap();
    let next = hosted.send("second").await.unwrap();
    assert!(
        tokio::time::timeout(WAIT, next.wait())
            .await
            .unwrap()
            .unwrap()
            .success
    );
    assert_eq!(hosted.history().page().await.unwrap().messages.len(), 4);
    assert_eq!(std::fs::read(&jsonl).unwrap(), journal);
    assert_eq!(
        everruns::hosting::EnvironmentFactory::open(
            &ImportedWorkspaceFactory::new(target, profile),
            id,
        )
        .await
        .unwrap()
        .workspace_head()
        .binding(),
        &legacy_head
    );
    assert!(matches!(
        Engine::new().attach(id, legacy_agent()).await,
        Err(ResumeError::CutOver)
    ));
}

/// A pending durable approval parked on the legacy lane reopens as an
/// authority wait with its deadline wakeup, resolves through the Session API
/// and runs the approved effect exactly once.
#[tokio::test]
async fn pending_legacy_approval_resolves_under_the_authority() {
    fn approving(
        backends: Option<&HostBackends>,
        effects: Arc<AtomicUsize>,
        calls: Vec<Vec<ToolCall>>,
    ) -> Agent {
        let builder = Agent::builder()
            .instructions("Request the action, then retry only after approval.")
            .model(Model::simulated_with_config(
                everruns::LlmSimConfig::fixed("Approved action done.")
                    .with_tool_call_sequence(calls),
            ))
            .tool(
                everruns::FunctionTool::new(
                    "mutate",
                    "Change one external value",
                    serde_json::json!({"type":"object","properties":{}}),
                    move |_: serde_json::Value| {
                        effects.fetch_add(1, Ordering::SeqCst);
                        async { Ok::<_, String>(serde_json::json!({"done":true})) }
                    },
                )
                .always_needs_approval(),
            )
            .durable_approvals();
        match backends {
            Some(backends) => builder.backends(backends.clone()),
            None => builder,
        }
        .build()
        .unwrap()
    }
    let call = |id: &str| ToolCall {
        id: id.into(),
        name: "mutate".into(),
        arguments: serde_json::json!({}),
    };
    let effects = Arc::new(AtomicUsize::new(0));
    let backends = HostBackends::in_memory();
    let legacy = approving(
        Some(&backends),
        effects.clone(),
        vec![vec![call("call_first")], vec![]],
    );
    let id;
    {
        let legacy_engine = Engine::new();
        let session = legacy_engine.create(legacy.clone());
        id = session.session_id();
        let _sent = session.send("do it").await.unwrap();
        tokio::time::timeout(WAIT, async {
            loop {
                if legacy_journal(&backends, id)
                    .await
                    .iter()
                    .any(|(_, _, kind)| kind == "tool.call_requested")
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
    let directory = tempfile::tempdir().unwrap();
    let engine = hosting_engine(
        SqliteHosting::open(directory.path().join("authority.db")).unwrap(),
        approving(
            None,
            effects.clone(),
            vec![vec![call("call_retry")], vec![]],
        ),
    );
    tokio::time::timeout(WAIT, async {
        loop {
            match engine
                .cut_over_legacy_session(id, &legacy, &target_build())
                .await
            {
                Ok(report) => break report,
                Err(error) if error.to_string().contains("executing") => {
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                Err(error) => panic!("{error}"),
            }
        }
    })
    .await
    .unwrap();
    let status = engine.session_status(id).await.unwrap();
    let request = status
        .waiting
        .iter()
        .find(|call| call.name == "approve_tool_call")
        .expect("approval wait reopened")
        .clone();
    assert!(
        status.wake_at.is_some(),
        "approval deadline lost its wakeup"
    );
    let turn_id = status.open_turn.unwrap();
    let session = engine.resume(id).await.unwrap();
    let input = input_ids(&session.events_after(0).await.unwrap())[0];
    session
        .submit_approvals(
            uuid::Uuid::new_v4(),
            turn_id,
            vec![everruns_contracts::execution_api::ToolApprovalAnswer {
                tool_call_id: request.id,
                decision: everruns::hosting::ToolApprovalDecision::Allow,
            }],
        )
        .await
        .unwrap();
    let outcome = tokio::time::timeout(
        WAIT,
        session.admission(input).await.unwrap().unwrap().wait(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(outcome.success);
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

/// After cutover JSONL is a projection: rebuilt from the authority in the
/// `JsonlEventLog` format, readable by it, covering the legacy prefix and
/// later turns, and identical on every rebuild.
#[tokio::test]
async fn jsonl_is_a_rebuildable_export_of_the_authority_after_cutover() {
    use everruns::hosting::spi::EventReader;
    let root = tempfile::tempdir().unwrap();
    let workspace = root.path().join("workspace");
    let legacy = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .local(everruns::LocalConfig::new(root.path().join("state")).workspace(&workspace))
        .build()
        .unwrap();
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    session.run("first").await.unwrap();
    drop(session);
    let legacy_log =
        everruns_core::host::JsonlEventLog::open(root.path().join("state/events.jsonl"))
            .await
            .unwrap();
    let legacy_events = legacy_log
        .read_page(EventReadRequest::new(
            id,
            EventReadLimit::new(1000).unwrap(),
        ))
        .await
        .unwrap()
        .events;
    drop(legacy_log);

    let target = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .workspace(&workspace)
        .build()
        .unwrap();
    let profile = SqliteHosting::open(root.path().join("authority.db")).unwrap();
    let engine = Engine::builder().hosting(profile.clone()).build();
    engine
        .register_agent_with_environment(
            target_build(),
            target.clone(),
            None,
            "imported-workspace",
            ImportedWorkspaceFactory::new(target, profile.clone()),
        )
        .unwrap();
    engine
        .cut_over_legacy_session(id, &legacy, &target_build())
        .await
        .unwrap();
    let hosted = engine.resume(id).await.unwrap();
    let next = hosted.send("second").await.unwrap();
    tokio::time::timeout(WAIT, next.wait())
        .await
        .unwrap()
        .unwrap();
    let authority = hosted_journal(&hosted).await;

    let export = root.path().join("export/events.jsonl");
    std::fs::create_dir_all(export.parent().unwrap()).unwrap();
    let written = profile.rebuild_jsonl_export(id, &export).unwrap();
    assert_eq!(written, authority.len());
    let first = std::fs::read(&export).unwrap();
    assert_eq!(profile.rebuild_jsonl_export(id, &export).unwrap(), written);
    assert_eq!(
        std::fs::read(&export).unwrap(),
        first,
        "rebuild is deterministic"
    );

    let projected = everruns_core::host::JsonlEventLog::open(&export)
        .await
        .unwrap();
    let events = projected
        .read_page(EventReadRequest::new(
            id,
            EventReadLimit::new(1000).unwrap(),
        ))
        .await
        .unwrap()
        .events;
    let rows: Vec<_> = events
        .iter()
        .map(|event| {
            (
                event.id.to_string(),
                event.sequence.unwrap(),
                event.event_type.clone(),
            )
        })
        .collect();
    assert_eq!(rows, authority);
    for (legacy, exported) in legacy_events.iter().zip(&events) {
        assert_eq!(
            serde_json::to_value(legacy).unwrap(),
            serde_json::to_value(exported).unwrap()
        );
    }
}

mod obligations {
    use super::*;
    use everruns_core::session_task::{
        CreateSessionTask, SessionTaskRegistry, SessionTaskState, SessionTaskUpdate,
        TASK_KIND_BACKGROUND_TOOL, TaskWakePolicy,
    };

    /// A PID that belonged to a process which has exited.
    fn dead_pid() -> u32 {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        pid
    }

    fn owner(pid: u32) -> String {
        format!("background-tool:{pid}:{}", uuid::Uuid::new_v4().simple())
    }

    async fn running_task(
        registry: &dyn SessionTaskRegistry,
        session: SessionId,
        worker: String,
        reattachable: bool,
    ) -> String {
        let task = registry
            .create(CreateSessionTask {
                session_id: session,
                id: None,
                kind: TASK_KIND_BACKGROUND_TOOL.into(),
                display_name: "Background clock".into(),
                spec: serde_json::json!({
                    "tool": "get_current_time",
                    "args": {},
                    "reattachable": reattachable,
                }),
                state: SessionTaskState::Running,
                links: Default::default(),
                wake_policy: TaskWakePolicy::OnTerminal,
            })
            .await
            .unwrap();
        registry
            .update(
                session,
                &task.id,
                SessionTaskUpdate {
                    worker_id: Some(worker),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        task.id
    }

    fn legacy_with_tasks() -> (HostBackends, Arc<everruns::local::LocalSessionTaskRegistry>) {
        let registry = Arc::new(
            everruns::local::LocalSessionTaskRegistry::new(
                everruns::local::SqliteDb::open_in_memory().unwrap(),
            )
            .unwrap(),
        );
        (
            HostBackends::in_memory().with_session_task_registry(registry.clone()),
            registry,
        )
    }

    async fn wait_for_task(
        profile: &SqliteHosting,
        session: SessionId,
        task: &str,
        done: impl Fn(&everruns_core::session_task::SessionTask) -> bool,
    ) -> everruns_core::session_task::SessionTask {
        let registry = profile.reader(session).session_task_registry.unwrap();
        tokio::time::timeout(WAIT, async {
            loop {
                if let Some(found) = registry.get(session, task).await.unwrap()
                    && done(&found)
                {
                    return found;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap()
    }

    /// Background work whose legacy owner died moves with the session; the
    /// first activation settles it conservatively (never re-running a
    /// side-effecting tool) and wakes the session exactly once, durably.
    #[tokio::test]
    async fn orphaned_legacy_background_task_is_settled_and_wakes_once() {
        let (backends, registry) = legacy_with_tasks();
        let legacy = legacy_agent(&backends);
        let session = Engine::new().create(legacy.clone());
        let id = session.session_id();
        session.run("first").await.unwrap();
        let task = running_task(registry.as_ref(), id, owner(dead_pid()), false).await;

        let directory = tempfile::tempdir().unwrap();
        let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
        let engine = hosting_engine(profile.clone(), agent());
        let report = engine
            .cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .unwrap();
        assert_eq!(report.tasks, 1);

        let hosted = engine.resume(id).await.unwrap();
        let next = hosted.send("continue").await.unwrap();
        tokio::time::timeout(WAIT, next.wait())
            .await
            .unwrap()
            .unwrap();
        let settled = wait_for_task(&profile, id, &task, |task| task.state.is_terminal()).await;
        assert_eq!(settled.state, SessionTaskState::Failed);
        assert_eq!(
            settled.error.as_ref().map(|error| error.kind.as_str()),
            Some("execution_status_unknown")
        );
        // The wake is an admission with a task-derived identity: it runs once.
        tokio::time::timeout(WAIT, async {
            loop {
                let inputs = hosted
                    .events_after(0)
                    .await
                    .unwrap()
                    .iter()
                    .filter(|event| {
                        event.event_type() == "input.message"
                            && event
                                .canonical_json()
                                .to_string()
                                .contains("Background clock")
                    })
                    .count();
                if inputs == 1 && !engine.session_status(id).await.unwrap().runnable {
                    break;
                }
                assert!(inputs <= 1, "wake delivered twice");
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
    }

    /// A task that claims to be safe to run again gets one fenced re-attach
    /// under the authority. The executor re-verifies the claim against the
    /// live tool (the default clock tool is not background-capable), refuses,
    /// and the task settles once, conservatively, on the new attempt.
    #[tokio::test]
    async fn reattach_is_fenced_and_falls_back_conservatively_after_owner_loss() {
        let (backends, registry) = legacy_with_tasks();
        let legacy = legacy_agent(&backends);
        let session = Engine::new().create(legacy.clone());
        let id = session.session_id();
        session.run("first").await.unwrap();
        let task = running_task(registry.as_ref(), id, owner(dead_pid()), true).await;
        let directory = tempfile::tempdir().unwrap();
        let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
        let engine = hosting_engine(profile.clone(), agent());
        engine
            .cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .unwrap();
        let hosted = engine.resume(id).await.unwrap();
        tokio::time::timeout(WAIT, hosted.send("continue").await.unwrap().wait())
            .await
            .unwrap()
            .unwrap();
        let resumed = wait_for_task(&profile, id, &task, |task| task.state.is_terminal()).await;
        assert_eq!(resumed.state, SessionTaskState::Failed, "{resumed:?}");
        assert_eq!(resumed.attempt, 2, "exactly one re-attach claim");
        assert_eq!(resumed.worker_id.as_deref(), Some("authority"));
        assert_eq!(
            resumed.error.as_ref().map(|error| error.kind.as_str()),
            Some("orphaned_execution")
        );
    }

    /// Work still owned by a live legacy process cannot move.
    #[tokio::test]
    async fn task_owned_by_a_live_process_refuses_cutover() {
        let (backends, registry) = legacy_with_tasks();
        let legacy = legacy_agent(&backends);
        let session = Engine::new().create(legacy.clone());
        let id = session.session_id();
        session.run("first").await.unwrap();
        running_task(registry.as_ref(), id, owner(std::process::id()), false).await;
        let directory = tempfile::tempdir().unwrap();
        let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
        let error = hosting_engine(profile.clone(), agent())
            .cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .unwrap_err();
        assert!(error.to_string().contains("still running"), "{error}");
        assert!(profile.status(id).await.is_err());
    }
}

mod schedules {
    use super::*;

    /// An active legacy schedule moves with the session; after cutover the
    /// authority's supervisor fires it as an ordinary admission, without
    /// client traffic, and the legacy lane never runs it.
    #[tokio::test]
    async fn active_legacy_schedule_fires_under_the_authority_after_cutover() {
        let db = everruns::local::SqliteDb::open_in_memory().unwrap();
        let owner = everruns_contracts::typed_id::PrincipalId::from_seed(1);
        let store = everruns::local::LocalScheduleStore::new(db.clone(), 1, owner).unwrap();
        let backends =
            HostBackends::in_memory().with_schedule_store_factory(Arc::new(move |org| {
                Arc::new(everruns::local::LocalScheduleStore::new(db.clone(), org, owner).unwrap())
                    as Arc<dyn everruns_core::session_services::SessionScheduleStore>
            }));
        let legacy = legacy_agent(&backends);
        let session = Engine::new().create(legacy.clone());
        let id = session.session_id();
        session.run("first").await.unwrap();
        use everruns_core::session_services::SessionScheduleStore;
        let scheduled = store
            .create_schedule(
                id,
                "scheduled check-in".into(),
                None,
                Some(chrono::Utc::now() + chrono::Duration::seconds(2)),
                "UTC".into(),
            )
            .await
            .unwrap();
        assert!(scheduled.enabled);

        let directory = tempfile::tempdir().unwrap();
        let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
        let engine = hosting_engine(profile.clone(), agent());
        let report = engine
            .cut_over_legacy_session(id, &legacy, &target_build())
            .await
            .unwrap();
        assert_eq!(report.schedules, 1);

        let stop = everruns::CancellationToken::new();
        let supervisor = {
            let engine = engine.clone();
            let profile = profile.clone();
            let stop = stop.clone();
            tokio::spawn(async move { profile.supervise(engine, stop).await })
        };
        let hosted = engine.resume(id).await.unwrap();
        tokio::time::timeout(WAIT, async {
            loop {
                let fired = hosted.events_after(0).await.unwrap().iter().any(|event| {
                    event.event_type() == "input.message"
                        && event
                            .canonical_json()
                            .to_string()
                            .contains("scheduled check-in")
                });
                let status = engine.session_status(id).await.unwrap();
                if fired && !status.runnable && !status.executing {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        supervisor.await.unwrap().unwrap();
        assert_eq!(hosted.history().page().await.unwrap().messages.len(), 4);
    }
}
