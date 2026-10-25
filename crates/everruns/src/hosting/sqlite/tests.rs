use super::*;

#[test]
fn startup_discovery_uses_indexes_and_selects_only_due_unowned_work_once() {
    let dir = tempfile::tempdir().unwrap();
    let hosting = SqliteHosting::open(dir.path().join("authority.db")).unwrap();
    let db = hosting.db.lock().unwrap();
    for (name, runnable, wake_at, expires, suspended) in [
        ("idle", false, None, 0, false),
        ("runnable", true, None, 0, false),
        ("due", false, Some(99), 0, false),
        ("both", true, Some(99), 0, false),
        ("future", false, Some(101), 0, false),
        ("owned", true, Some(99), 101, false),
        ("suspended", true, Some(99), 0, true),
    ] {
        db.execute(
            "INSERT INTO framework_authority VALUES (?,?)",
            params![name, json!({"runnable":runnable,"wake_at":wake_at,"expires":expires,"suspended":suspended}).to_string()],
        ).unwrap();
    }
    let mut query = db.prepare(DISCOVER_RUNNABLE).unwrap();
    let found: Vec<String> = query
        .query_map([100], |row| row.get(0))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    assert_eq!(found, ["both", "due", "runnable"]);

    let mut plan = db
        .prepare(&format!("EXPLAIN QUERY PLAN {DISCOVER_RUNNABLE}"))
        .unwrap();
    let plan = plan
        .query_map([100], |row| row.get::<_, String>(3))
        .unwrap()
        .map(|row| row.unwrap())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(plan.contains("framework_runnable"), "{plan}");
    assert!(plan.contains("framework_timers"), "{plan}");
    assert!(!plan.contains("SCAN framework_authority"), "{plan}");
}

/// EVE-1305: installs keep per-lineage slots; a rewind sees the ancestor
/// checkpoint and never the abandoned suffix's replacement, and redo restores it.
#[tokio::test]
async fn compaction_checkpoints_follow_the_selected_branch() {
    use everruns_core::{CompactionCheckpoint, CompactionCheckpointPayload};
    let dir = tempfile::tempdir().unwrap();
    let hosting = SqliteHosting::open(dir.path().join("authority.db")).unwrap();
    let id = SessionId::new();
    let binding = SessionBinding {
        framework: 1,
        behavior: AgentBuild::new("branches", "v1"),
        harness: None,
        environment: None,
        metadata: Value::Null,
    };
    hosting.create(id, &binding).await.unwrap();
    let seeded = vec![
        RuntimeMessage::user("a"),
        RuntimeMessage::assistant("a!"),
        RuntimeMessage::user("b"),
        RuntimeMessage::assistant("b!"),
    ];
    assert!(
        hosting
            .seed_history(id, uuid::Uuid::new_v4(), seeded)
            .await
            .unwrap()
    );
    hosting
        .admit(id, RuntimeMessage::user("run"), false)
        .await
        .unwrap();
    let activation = hosting
        .claim(id, "holder", Duration::from_secs(30))
        .await
        .unwrap()
        .unwrap();
    let store = activation.epoch.backends().compaction_checkpoint_store;
    let checkpoint = |source: i64| CompactionCheckpoint {
        id: uuid::Uuid::now_v7(),
        session_id: id,
        source_sequence: source,
        provider_type: "sim".into(),
        model: "m".into(),
        format_version: 1,
        payload: CompactionCheckpointPayload::Summary {
            text: source.to_string(),
        },
    };
    assert!(store.install(checkpoint(2)).await.unwrap());
    assert!(store.install(checkpoint(4)).await.unwrap());
    assert!(
        !store.install(checkpoint(3)).await.unwrap(),
        "an older checkpoint on the same branch is redundant"
    );
    activation
        .epoch
        .actor_store()
        .release_actor(id, "holder", activation.actor.fence, 5)
        .await
        .unwrap();
    let latest = || async {
        hosting
            .reader(id)
            .compaction_checkpoint_store
            .get_latest(id, "sim", "m")
            .await
            .unwrap()
            .map(|checkpoint| checkpoint.source_sequence)
    };
    assert_eq!(latest().await, Some(4));
    let lineage = hosting.history_lineage(id).await.unwrap();
    hosting
        .select_history_branch(
            id,
            uuid::Uuid::new_v4(),
            lineage.revision,
            "rewound",
            lineage.rewind_ranges(2),
        )
        .await
        .unwrap();
    assert_eq!(latest().await, Some(2), "the ancestor stays eligible");
    hosting
        .select_history_branch(id, uuid::Uuid::new_v4(), 1, "main", None)
        .await
        .unwrap();
    assert_eq!(
        latest().await,
        Some(4),
        "redo restores the branch's checkpoint"
    );
}
