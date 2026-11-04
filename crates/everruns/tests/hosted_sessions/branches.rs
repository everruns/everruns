//! EVE-1305: conversation branch selection and compaction lineage under the
//! coherent SQLite authority. The journal stays immutable; the authority owns
//! which branch is context, and checkpoints follow the branch they summarized.
#![allow(deprecated)] // The lineage import rides the legacy cutover (EVE-1298).
use super::*;
use everruns::hosting::spi::{
    CompactionCheckpoint, CompactionCheckpointPayload, EventReadLimit, EventReadRequest,
    HostBackends,
};
use everruns::hosting::{
    ConversationLineage, CutoverOptions, HistoryBranch, SequenceRange, SessionBinding,
    SqliteHosting,
};
use everruns::llm::Message;
use everruns::{LlmSimConfig, Turn};
use std::sync::Mutex;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

type Captured = Arc<Mutex<Vec<Vec<Message>>>>;

fn capturing_agent() -> (Agent, Captured) {
    let captured: Captured = Arc::default();
    let mut config = LlmSimConfig::fixed("Hello!");
    config.message_capture = Some(captured.clone());
    let agent = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated_with_config(config))
        .build()
        .unwrap();
    (agent, captured)
}

fn last_context(captured: &Captured) -> String {
    captured
        .lock()
        .unwrap()
        .last()
        .expect("a model call")
        .iter()
        .map(|message| message.content.to_text())
        .collect::<Vec<_>>()
        .join("\n")
}

async fn turn(session: &everruns::Session, text: &str) -> Turn {
    let sent = session.send(text).await.unwrap();
    tokio::time::timeout(WAIT, sent.wait())
        .await
        .unwrap()
        .unwrap()
}

async fn transcript(session: &everruns::Session) -> Vec<String> {
    session
        .history()
        .page()
        .await
        .unwrap()
        .messages
        .iter()
        .map(|message| message.text())
        .collect()
}

/// (id, sequence, type) of every event a context/history read returns.
async fn visible(session: &everruns::Session) -> Vec<(String, i32, String)> {
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

/// (id, sequence) of every raw journal row, abandoned branches included.
fn raw(profile: &SqliteHosting, id: SessionId) -> Vec<(String, i32)> {
    let mut out = Vec::new();
    profile.export_jsonl(id, &mut out).unwrap();
    String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|line| {
            let event: serde_json::Value = serde_json::from_str(line).unwrap();
            (
                event["id"].as_str().unwrap().to_owned(),
                event["sequence"].as_i64().unwrap() as i32,
            )
        })
        .collect()
}

/// Last sequence of each turn, in order. A turn ends after its terminal
/// (`session.idled` still belongs to it), and branches select whole turns.
fn turn_ends<'a>(
    events: impl IntoIterator<Item = (i32, Option<&'a serde_json::Value>)>,
) -> Vec<i32> {
    let mut ends: Vec<(String, i32)> = Vec::new();
    for (sequence, turn) in events {
        let Some(turn) = turn.and_then(serde_json::Value::as_str) else {
            continue;
        };
        match ends.iter_mut().find(|(id, _)| id == turn) {
            Some(end) => end.1 = sequence,
            None => ends.push((turn.to_owned(), sequence)),
        }
    }
    ends.into_iter().map(|(_, end)| end).collect()
}

fn build() -> AgentBuild {
    AgentBuild::new("assistant", "branches-v1")
}

fn hosted(profile: SqliteHosting, agent: Agent) -> Engine {
    Engine::builder()
        .hosting(profile)
        .register(build(), agent)
        .build()
}

/// Acceptance: A -> B, rewind to A, produce C, restart. Context is A+C, the
/// raw journal keeps A+B+C with unchanged identities, and B stays redoable.
#[tokio::test]
async fn rewind_continues_the_selected_branch_and_keeps_the_abandoned_one() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let profile = SqliteHosting::open(&path).unwrap();
    let (agent, captured) = capturing_agent();
    let engine = hosted(profile.clone(), agent);
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    assert!(turn(&session, "alpha question").await.success);
    assert!(turn(&session, "beta question").await.success);
    let before = visible(&session).await;
    let events = session.events_after(0).await.unwrap();
    let envelopes: Vec<_> = events.iter().map(|event| event.canonical_json()).collect();
    let a_end = turn_ends(envelopes.iter().map(|event| {
        (
            event["sequence"].as_i64().unwrap() as i32,
            event["context"].get("turn_id"),
        )
    }))[0];

    let lineage = engine.history_lineage(id).await.unwrap();
    assert_eq!((lineage.revision, lineage.active.as_str()), (0, "main"));
    let rewind = lineage.rewind_ranges(a_end).unwrap();
    let command = uuid::Uuid::new_v4();
    let selected = engine
        .select_history_branch(id, command, 0, "rewind-1", Some(rewind.clone()))
        .await
        .unwrap();
    assert_eq!((selected.revision, selected.changed), (1, true));
    // A lost reply replays; a concurrent change at the old revision refuses.
    assert_eq!(
        engine
            .select_history_branch(id, command, 0, "rewind-1", Some(rewind.clone()))
            .await
            .unwrap(),
        selected
    );
    assert!(
        engine
            .select_history_branch(id, uuid::Uuid::new_v4(), 0, "other", Some(rewind))
            .await
            .is_err(),
        "a stale revision must not change the branch"
    );
    assert_eq!(
        transcript(&session).await,
        ["alpha question", "Hello!"],
        "history follows the selected branch"
    );

    assert!(turn(&session, "gamma question").await.success);
    let context = last_context(&captured);
    assert!(context.contains("alpha question") && context.contains("gamma question"));
    assert!(
        !context.contains("beta question"),
        "abandoned input must not re-enter model context: {context}"
    );

    // Restart: a new connection and Engine reopen the same selection.
    drop(session);
    drop(engine);
    let profile = SqliteHosting::open(&path).unwrap();
    let (agent, captured) = capturing_agent();
    let engine = hosted(profile.clone(), agent);
    let session = engine.resume(id).await.unwrap();
    assert_eq!(
        transcript(&session).await,
        ["alpha question", "Hello!", "gamma question", "Hello!"]
    );
    let after = visible(&session).await;
    assert!(
        after
            .iter()
            .all(|event| !before[a_end as usize..].contains(event)),
        "no abandoned event is visible on the selected branch"
    );
    // Raw export keeps every source identity and sequence, B included.
    let journal = raw(&profile, id);
    let originals: Vec<(String, i32)> = before
        .iter()
        .map(|(id, sequence, _)| (id.clone(), *sequence))
        .collect();
    assert_eq!(journal[..originals.len()], originals[..]);
    assert_eq!(
        journal.len(),
        originals.len() + after.len() - a_end as usize
    );

    let lineage = engine.history_lineage(id).await.unwrap();
    assert_eq!((lineage.revision, lineage.active.as_str()), (1, "rewind-1"));
    let head = journal.last().unwrap().1;
    assert_eq!(
        lineage.active_ranges(),
        [
            SequenceRange::new(1, a_end),
            SequenceRange::new(before.len() as i32 + 1, head)
        ]
    );
    assert_eq!(
        lineage.branch("main").unwrap().ranges,
        [SequenceRange::new(1, before.len() as i32)],
        "the abandoned branch is retained at its extent"
    );

    // Redo: reselecting the retained branch restores B and retains C.
    let redo = engine
        .select_history_branch(id, uuid::Uuid::new_v4(), 1, "main", None)
        .await
        .unwrap();
    assert_eq!(redo.revision, 2);
    assert_eq!(
        transcript(&session).await,
        ["alpha question", "Hello!", "beta question", "Hello!"]
    );
    assert!(turn(&session, "delta question").await.success);
    let context = last_context(&captured);
    assert!(context.contains("beta question") && !context.contains("gamma question"));
    let lineage = engine.history_lineage(id).await.unwrap();
    assert_eq!(
        lineage.branch("rewind-1").unwrap().ranges,
        [
            SequenceRange::new(1, a_end),
            SequenceRange::new(before.len() as i32 + 1, head)
        ]
    );
}

fn binding() -> SessionBinding {
    SessionBinding {
        framework: 1,
        behavior: AgentBuild::new("branches", uuid::Uuid::new_v4().to_string()),
        harness: None,
        environment: None,
        metadata: serde_json::Value::Null,
    }
}

/// Owed work refuses a branch change; it is never silently discarded.
#[tokio::test]
async fn branch_changes_refuse_epochs_and_unconsumed_input() {
    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let id = SessionId::new();
    profile.create(id, &binding()).await.unwrap();
    let empty = || Some(Vec::new());
    profile
        .admit(
            id,
            everruns::hosting::spi::RuntimeMessage::user("queued"),
            false,
        )
        .await
        .unwrap();
    let refused = profile
        .select_history_branch(id, uuid::Uuid::new_v4(), 0, "empty", empty())
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("quiescent"), "{refused}");

    let activation = profile
        .claim(id, "holder", Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    assert!(
        profile
            .select_history_branch(id, uuid::Uuid::new_v4(), 0, "empty", empty())
            .await
            .is_err(),
        "a live epoch refuses"
    );
    let head = profile.query_snapshot(id).await.unwrap().high_watermark;
    activation
        .epoch
        .actor_store()
        .release_actor(id, "holder", activation.actor.fence, i64::from(head))
        .await
        .unwrap();
    let selected = profile
        .select_history_branch(id, uuid::Uuid::new_v4(), 0, "empty", empty())
        .await
        .unwrap();
    assert_eq!(selected.revision, 1);
    // Partial turns and unknown branches are refused by the shared rules.
    assert!(
        profile
            .select_history_branch(id, uuid::Uuid::new_v4(), 1, "missing", None)
            .await
            .is_err()
    );
    assert!(
        profile
            .select_history_branch(
                id,
                uuid::Uuid::new_v4(),
                1,
                "beyond",
                Some(vec![SequenceRange::new(1, head + 1)])
            )
            .await
            .is_err()
    );
    let page = profile
        .reader(id)
        .event_log
        .read_page(EventReadRequest::new(id, EventReadLimit::new(10).unwrap()))
        .await
        .unwrap();
    assert!(page.events.is_empty(), "the empty branch hides the input");
    assert_eq!(page.snapshot_high_watermark(), head);
}

#[tokio::test]
async fn volatile_authority_refuses_branch_selection() {
    let profile = MemoryHosting::default();
    let id = SessionId::new();
    profile.create(id, &binding()).await.unwrap();
    assert!(profile.history_lineage(id).await.is_err());
    let error = profile
        .select_history_branch(id, uuid::Uuid::new_v4(), 0, "b", Some(Vec::new()))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("does not support"), "{error}");
}

fn checkpoint(id: SessionId, source: i32, text: &str) -> CompactionCheckpoint {
    CompactionCheckpoint {
        id: uuid::Uuid::now_v7(),
        session_id: id,
        source_sequence: i64::from(source),
        provider_type: "sim".into(),
        model: "sim-model".into(),
        format_version: 1,
        payload: CompactionCheckpointPayload::Summary { text: text.into() },
    }
}

async fn eligible(profile: &SqliteHosting, id: SessionId) -> Option<String> {
    profile
        .reader(id)
        .compaction_checkpoint_store
        .get_latest(id, "sim", "sim-model")
        .await
        .unwrap()
        .map(|checkpoint| match checkpoint.payload {
            CompactionCheckpointPayload::Summary { text } => text,
            other => panic!("unexpected payload {other:?}"),
        })
}

/// A downstream product imports its saved A -> B history already rewound to
/// A, with a checkpoint per branch. B's replacement summary never appears
/// under A or C; the ancestor checkpoint stays eligible, and redo restores B's.
#[tokio::test]
async fn imported_lineage_selects_branch_checkpoints_and_retries_idempotently() {
    let backends = HostBackends::in_memory();
    let legacy = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .backends(backends.clone())
        .build()
        .unwrap();
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    assert!(session.run("alpha question").await.unwrap().success);
    assert!(session.run("beta question").await.unwrap().success);
    drop(session);
    let page = backends
        .event_log
        .read_page(EventReadRequest::new(
            id,
            EventReadLimit::new(1000).unwrap(),
        ))
        .await
        .unwrap();
    let turns: Vec<_> = page
        .events
        .iter()
        .map(|event| {
            (
                event.sequence.unwrap(),
                event.context.turn_id.map(|turn| serde_json::json!(turn)),
            )
        })
        .collect();
    let ends = turn_ends(
        turns
            .iter()
            .map(|(sequence, turn)| (*sequence, turn.as_ref())),
    );
    let outputs: Vec<i32> = page
        .events
        .iter()
        .filter(|event| event.event_type == "output.message.completed")
        .map(|event| event.sequence.unwrap())
        .collect();
    let head = page.events.last().unwrap().sequence.unwrap();
    let lineage = ConversationLineage::new(
        "rewound",
        vec![
            HistoryBranch {
                id: "rewound".into(),
                ranges: vec![SequenceRange::new(1, ends[0])],
            },
            HistoryBranch {
                id: "main".into(),
                ranges: vec![SequenceRange::new(1, head)],
            },
        ],
    )
    .with_checkpoint("main", checkpoint(id, outputs[0], "ancestor summary"))
    .with_checkpoint("main", checkpoint(id, outputs[1], "beta summary"));

    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let (agent, captured) = capturing_agent();
    let engine = Engine::builder()
        .hosting(profile.clone())
        .register(AgentBuild::new("assistant", "cutover-v1"), agent)
        .build();
    let target = AgentBuild::new("assistant", "cutover-v1");
    let options = CutoverOptions::new().lineage(lineage.clone());
    let report = engine
        .cut_over_legacy_session_with_options(id, &legacy, &target, options.clone())
        .await
        .unwrap();
    assert_eq!((report.branches, report.replayed), (2, false));
    // The same import after an interrupted reply replays without duplicates.
    let retried = engine
        .cut_over_legacy_session_with_options(id, &legacy, &target, options)
        .await
        .unwrap();
    assert!(retried.replayed);
    assert_eq!(raw(&profile, id).len(), page.events.len());
    let mut changed = lineage.clone();
    changed.active = "main".into();
    assert!(
        engine
            .cut_over_legacy_session_with_options(
                id,
                &legacy,
                &target,
                CutoverOptions::new().lineage(changed)
            )
            .await
            .is_err(),
        "a different lineage cannot reuse the committed cutover"
    );

    let hosted = engine.resume(id).await.unwrap();
    assert_eq!(transcript(&hosted).await, ["alpha question", "Hello!"]);
    assert_eq!(
        eligible(&profile, id).await.as_deref(),
        Some("ancestor summary")
    );
    assert!(turn(&hosted, "gamma question").await.success);
    assert!(!last_context(&captured).contains("beta question"));
    assert_eq!(
        eligible(&profile, id).await.as_deref(),
        Some("ancestor summary"),
        "B's replacement cannot appear under C"
    );

    let lineage = engine.history_lineage(id).await.unwrap();
    engine
        .select_history_branch(id, uuid::Uuid::new_v4(), lineage.revision, "main", None)
        .await
        .unwrap();
    assert_eq!(
        eligible(&profile, id).await.as_deref(),
        Some("beta summary")
    );
    assert_eq!(
        transcript(&hosted).await,
        ["alpha question", "Hello!", "beta question", "Hello!"]
    );
}

/// A branch over part of a turn would strand its tool results or terminal.
#[tokio::test]
async fn imported_lineage_refuses_split_turns_before_any_write() {
    let backends = HostBackends::in_memory();
    let legacy = Agent::builder()
        .instructions("Answer briefly.")
        .model(Model::simulated("Hello!"))
        .backends(backends.clone())
        .build()
        .unwrap();
    let session = Engine::new().create(legacy.clone());
    let id = session.session_id();
    assert!(session.run("alpha question").await.unwrap().success);
    drop(session);
    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let engine = Engine::builder()
        .hosting(profile.clone())
        .register(AgentBuild::new("assistant", "cutover-v1"), agent())
        .build();
    let split = ConversationLineage::new(
        "main",
        vec![HistoryBranch {
            id: "main".into(),
            // Through `turn.started`, but not the rest of the turn.
            ranges: vec![SequenceRange::new(1, 4)],
        }],
    );
    let error = engine
        .cut_over_legacy_session_with_options(
            id,
            &legacy,
            &AgentBuild::new("assistant", "cutover-v1"),
            CutoverOptions::new().lineage(split),
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("whole turns"), "{error}");
    assert!(profile.status(id).await.is_err(), "nothing was committed");
}
