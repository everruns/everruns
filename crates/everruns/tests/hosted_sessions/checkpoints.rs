//! EVE-1339: conversation checkpoints over the branch-aware SQLite journal.
//! A host drives create, list, preview, restore and redo purely through
//! `Engine::checkpoints`; the journal is never rewritten.
use super::*;
use everruns::checkpoints::{Checkpoints, RestoreMode};
use everruns::hosting::{SessionBinding, SqliteHosting};
use everruns::{FunctionTool, LlmSimConfig, ToolCall, Turn};
use serde_json::json;
use std::sync::OnceLock;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(10);

fn build() -> AgentBuild {
    AgentBuild::new("assistant", "checkpoints-v1")
}

fn hosted(profile: SqliteHosting) -> Engine {
    Engine::builder()
        .hosting(profile)
        .register(build(), agent())
        .build()
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

fn raw_len(profile: &SqliteHosting, id: SessionId) -> usize {
    let mut out = Vec::new();
    profile.export_jsonl(id, &mut out).unwrap()
}

/// Acceptance: checkpoint after A and after B, restore A as a branch, run C,
/// restart, redo B, then truncate back to A. Branch restores keep the undone
/// tail addressable; a truncating restore drops it and its checkpoints.
#[tokio::test]
async fn restore_branch_keeps_the_undone_tail_and_truncate_drops_it() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("authority.db");
    let profile = SqliteHosting::open(&path).unwrap();
    let engine = hosted(profile.clone());
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let checkpoints = engine.checkpoints(id);

    let empty = checkpoints.create_labeled("start").await.unwrap();
    assert_eq!((empty.turn_index, empty.message_id), (0, None));
    assert!(turn(&session, "alpha question").await.success);
    let after_a = checkpoints.create_labeled("after\nalpha").await.unwrap();
    assert_eq!(after_a.turn_index, 1);
    assert_eq!(after_a.label.as_deref(), Some("after alpha"));
    assert_eq!(after_a.preview.as_deref(), Some("alpha question"));
    assert!(after_a.message_id.is_some() && after_a.on_active_branch);
    assert!(turn(&session, "beta question").await.success);
    let after_b = checkpoints.create().await.unwrap();
    assert_eq!(after_b.turn_index, 2);
    let listed: Vec<_> = checkpoints
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(listed, [empty.id, after_a.id, after_b.id]);

    let preview = checkpoints.preview(after_a.id).await.unwrap();
    assert!(preview.changes);
    assert_eq!(preview.active_branch, "main");
    assert_eq!(preview.dropped_total, 1);
    assert_eq!(preview.dropped[0].input.as_deref(), Some("beta question"));
    assert_eq!(preview.dropped[0].reply.as_deref(), Some("Hello!"));
    assert_eq!(preview.restored_total, 0);
    assert!(!checkpoints.preview(after_b.id).await.unwrap().changes);

    let restored = checkpoints
        .restore(after_a.id, RestoreMode::Branch)
        .await
        .unwrap();
    assert!(restored.changed);
    assert_eq!(restored.undone.as_deref(), Some("main"));
    assert_eq!(transcript(&session).await, ["alpha question", "Hello!"]);
    assert!(
        !checkpoints
            .restore(after_a.id, RestoreMode::Branch)
            .await
            .unwrap()
            .changed,
        "restoring the current point changes nothing"
    );
    assert!(turn(&session, "gamma question").await.success);

    // B's checkpoint stays reachable on the retained branch and previews the
    // turns it would bring back.
    let summaries = checkpoints.list().await.unwrap();
    let b = summaries.iter().find(|s| s.id == after_b.id).unwrap();
    assert!(!b.on_active_branch);
    let preview = checkpoints.preview(after_b.id).await.unwrap();
    assert_eq!(preview.dropped_total, 1);
    assert_eq!(preview.dropped[0].input.as_deref(), Some("gamma question"));
    assert_eq!(preview.restored_total, 1);
    assert_eq!(preview.restored[0].input.as_deref(), Some("beta question"));

    // Restart: checkpoints and branches survive a new connection and Engine.
    drop(session);
    drop(checkpoints);
    drop(engine);
    let profile = SqliteHosting::open(&path).unwrap();
    let engine = hosted(profile.clone());
    let session = engine.resume(id).await.unwrap();
    let checkpoints = engine.checkpoints(id);
    assert_eq!(checkpoints.list().await.unwrap().len(), 3);
    assert_eq!(
        transcript(&session).await,
        ["alpha question", "Hello!", "gamma question", "Hello!"]
    );

    let redo = checkpoints
        .redo(restored.undone.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(redo.branch, "main");
    assert_eq!(redo.undone.as_deref(), Some(restored.branch.as_str()));
    assert_eq!(
        transcript(&session).await,
        ["alpha question", "Hello!", "beta question", "Hello!"]
    );

    let truncated = checkpoints
        .restore(after_a.id, RestoreMode::Truncate)
        .await
        .unwrap();
    assert!(truncated.changed && truncated.undone.is_none());
    assert_eq!(transcript(&session).await, ["alpha question", "Hello!"]);
    let lineage = engine.history_lineage(id).await.unwrap();
    assert!(
        lineage.branch("main").is_none(),
        "the truncated tail is no longer addressable"
    );
    assert!(lineage.branch(&restored.branch).is_some());
    let remaining: Vec<_> = checkpoints
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(
        remaining,
        [empty.id, after_a.id],
        "checkpoints on the dropped tail are hidden"
    );
    assert!(checkpoints.preview(after_b.id).await.is_err());
    assert!(checkpoints.redo("main").await.is_err());
    // The raw journal still holds every event for audit and export.
    let before = raw_len(&profile, id);
    assert!(turn(&session, "delta question").await.success);
    assert!(raw_len(&profile, id) > before);

    // Restoring the empty start keeps nothing and never errs on the journal.
    checkpoints
        .restore(empty.id, RestoreMode::Branch)
        .await
        .unwrap();
    assert!(transcript(&session).await.is_empty());
    assert!(checkpoints.delete(empty.id).await.unwrap());
    assert!(!checkpoints.delete(empty.id).await.unwrap());
}

fn binding() -> SessionBinding {
    SessionBinding {
        framework: 1,
        behavior: AgentBuild::new("checkpoints", uuid::Uuid::new_v4().to_string()),
        harness: None,
        environment: None,
        metadata: serde_json::Value::Null,
    }
}

/// Checkpoint ids are scoped to their session, owed work refuses a restore
/// and volatile authorities refuse checkpoints outright.
#[tokio::test]
async fn checkpoints_are_session_scoped_and_refuse_busy_or_volatile_sessions() {
    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let engine = Engine::builder().hosting(profile.clone()).build();
    let (first, second) = (SessionId::new(), SessionId::new());
    profile.create(first, &binding()).await.unwrap();
    profile.create(second, &binding()).await.unwrap();
    let checkpoint = engine.checkpoints(first).create().await.unwrap();
    let other = engine.checkpoints(second);
    assert!(other.get(checkpoint.id).await.is_err());
    assert!(other.preview(checkpoint.id).await.is_err());
    assert!(
        other
            .restore(checkpoint.id, RestoreMode::Truncate)
            .await
            .is_err()
    );
    assert!(!other.delete(checkpoint.id).await.unwrap());
    assert!(engine.checkpoints(SessionId::new()).list().await.is_err());

    profile
        .admit(
            first,
            everruns::hosting::spi::RuntimeMessage::user("queued"),
            false,
        )
        .await
        .unwrap();
    let error = engine
        .checkpoints(first)
        .restore(checkpoint.id, RestoreMode::Branch)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("quiescent"), "{error}");
    // A checkpoint taken now ends before the queued input.
    let queued = engine.checkpoints(first).create().await.unwrap();
    assert!(queued.ranges.is_empty() && queued.turn_index == 0);

    let volatile = MemoryHosting::default();
    let id = SessionId::new();
    volatile.create(id, &binding()).await.unwrap();
    let engine = Engine::builder().hosting(volatile).build();
    let error = engine.checkpoints(id).create().await.unwrap_err();
    assert!(error.to_string().contains("does not support"), "{error}");
}

/// A conversational tool reads and creates checkpoints while its turn runs;
/// the checkpoint marks the end of the previous, finished turn.
#[tokio::test]
async fn a_tool_creates_checkpoints_while_its_turn_runs() {
    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let handle: Arc<OnceLock<Checkpoints>> = Arc::default();
    let tool_handle = handle.clone();
    let tool = FunctionTool::new(
        "checkpoint",
        "Save a conversation checkpoint.",
        json!({ "type": "object", "properties": {} }),
        move |_args: serde_json::Value| {
            let checkpoints = tool_handle.get().cloned();
            async move {
                let checkpoints = checkpoints.ok_or("no session")?;
                let created = checkpoints
                    .create_labeled("from tool")
                    .await
                    .map_err(|e| e.to_string())?;
                let count = checkpoints.list().await.map_err(|e| e.to_string())?.len();
                Ok::<_, String>(json!({ "turn_index": created.turn_index, "count": count }))
            }
        },
    );
    let model =
        Model::simulated_with_config(LlmSimConfig::fixed("Saved.").with_tool_call_sequence(vec![
            vec![],
            vec![ToolCall {
                id: "call_checkpoint_1".into(),
                name: "checkpoint".into(),
                arguments: json!({}),
            }],
            vec![],
        ]));
    let agent = Agent::builder()
        .instructions("Save checkpoints when asked.")
        .model(model)
        .tool(tool)
        .build()
        .unwrap();
    let engine = Engine::builder()
        .hosting(profile)
        .register(build(), agent)
        .build();
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let _ = handle.set(engine.checkpoints(id));
    assert!(turn(&session, "hello").await.success);
    let saved = turn(&session, "save a checkpoint").await;
    assert!(saved.success && saved.tool_calls == 1, "{saved:?}");
    let listed = engine.checkpoints(id).list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].turn_index, 1, "the running turn is not included");
    assert_eq!(listed[0].preview.as_deref(), Some("hello"));
    let preview = engine.checkpoints(id).preview(listed[0].id).await.unwrap();
    assert_eq!(
        preview.dropped[0].input.as_deref(),
        Some("save a checkpoint")
    );
}

/// Branch restores reuse an identical retained branch, use collision-proof
/// names, and retained branches can be deleted (hiding their checkpoints).
#[tokio::test]
async fn branch_restores_reuse_branches_and_retained_branches_can_be_deleted() {
    let directory = tempfile::tempdir().unwrap();
    let profile = SqliteHosting::open(directory.path().join("authority.db")).unwrap();
    let engine = hosted(profile.clone());
    let id = SessionId::new();
    let session = engine.create_registered(id, &build()).await.unwrap();
    let checkpoints = engine.checkpoints(id);
    assert!(turn(&session, "alpha question").await.success);
    let after_a = checkpoints.create().await.unwrap();
    assert!(turn(&session, "beta question").await.success);
    let after_b = checkpoints.create().await.unwrap();

    let first = checkpoints
        .restore(after_a.id, RestoreMode::Branch)
        .await
        .unwrap();
    assert!(first.branch.starts_with("restore-") && first.branch.len() > 20);
    checkpoints.redo("main").await.unwrap();
    let second = checkpoints
        .restore(after_a.id, RestoreMode::Branch)
        .await
        .unwrap();
    assert_eq!(second.branch, first.branch, "identical branch reused");
    assert_eq!(
        engine.history_lineage(id).await.unwrap().branches.len(),
        2,
        "no new branch per repeated restore"
    );

    assert!(checkpoints.delete_branch(&second.branch).await.is_err());
    assert!(checkpoints.delete_branch("main").await.unwrap());
    assert!(!checkpoints.delete_branch("main").await.unwrap());
    assert_eq!(engine.history_lineage(id).await.unwrap().branches.len(), 1);
    let listed: Vec<_> = checkpoints
        .list()
        .await
        .unwrap()
        .into_iter()
        .map(|summary| summary.id)
        .collect();
    assert_eq!(
        listed,
        [after_a.id],
        "after_b only lived on the deleted branch"
    );
    assert!(checkpoints.preview(after_b.id).await.is_err());
    assert_eq!(transcript(&session).await, ["alpha question", "Hello!"]);

    // Review finding B1: `delete_branch` needs no quiescence, so a client-side
    // prune (lineage read, list, delete in separate transactions) could delete
    // a checkpoint a tool committed on newer turns in between. Deleting a
    // branch therefore only hides its checkpoints; the authority prunes them
    // inside the transaction that commits the next checkpoint.
    let stored = |profile: SqliteHosting| async move {
        profile
            .conversation_checkpoints(id)
            .await
            .unwrap()
            .into_iter()
            .map(|checkpoint| checkpoint.id)
            .collect::<Vec<_>>()
    };
    assert_eq!(stored(profile.clone()).await, [after_a.id, after_b.id]);
    let next = checkpoints.create().await.unwrap();
    assert_eq!(
        stored(profile).await,
        [after_a.id, next.id],
        "the next create prunes atomically"
    );
}
