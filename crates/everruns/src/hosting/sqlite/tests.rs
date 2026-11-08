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

#[tokio::test]
async fn human_wait_survives_sqlite_reopen_and_command_replay() {
    use everruns_core::{
        events::{ToolCallRequestedData, ToolCompletedData, TurnStartedData},
        session_commands::SessionCommand,
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wait.db");
    let hosting = SqliteHosting::open(&path).unwrap();
    let session = SessionId::new();
    hosting
        .create(
            session,
            &SessionBinding {
                framework: 1,
                behavior: AgentBuild::new("waits", "v1"),
                harness: None,
                environment: None,
                metadata: Value::Null,
            },
        )
        .await
        .unwrap();
    let turn = TurnId::new();
    let input = MessageId::new();
    let context = EventContext::turn(turn, input);
    let started_at = everruns_contracts::rt::utc_now() - chrono::Duration::seconds(30);
    hosting.transaction(session, |tx,state| {
        let mut start = EventRequest::new(session, context.clone(), TurnStartedData {
            turn_id: turn, input_message_id: input, input_content: None,
            agent_id: None, agent_name: None, agent_description: None, origin: None,
        });
        everruns_core::causal::attach_turn_trace(&mut start,"00-1234567890abcdef1234567890abcdef-1234567890abcdef-01");
        append(tx,state,start,None)?;
        let mut ask = EventRequest::new(session,context.clone(),ToolCallRequestedData::with_definitions(&[crate::ToolCall {
            id:"q".into(), name:"ask_user".into(), arguments:json!({"questions":[],"expires_at":(everruns_contracts::rt::utc_now()+chrono::Duration::hours(1)).to_rfc3339()}),
        }],&[]));
        ask.ts=started_at;
        append(tx,state,ask,None)?;
        Ok(())
    }).unwrap();
    drop(hosting);
    let hosting = SqliteHosting::open(&path).unwrap();
    let command = SessionCommand::Resolve {
        id: "answer-once".into(),
        turn,
        results: vec![ToolCompletedData::success(
            "q".into(),
            "ask_user".into(),
            vec![everruns_core::message::ContentPart::tool_result_text(
                &json!({"status":"declined","answered_by":"user","answers":[]}),
            )],
            None,
        )],
    };
    hosting.execute_command(session, command.clone()).unwrap();
    hosting.execute_command(session, command).unwrap();
    let db = hosting.db.lock().unwrap();
    let mut query = db
        .prepare("SELECT value FROM framework_events WHERE session=? AND type='tool.completed'")
        .unwrap();
    let events: Vec<Event> = query
        .query_map([session.to_string()], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|row| decode(row.unwrap()).unwrap())
        .collect();
    assert_eq!(events.len(), 1);
    let wait = &events[0].metadata.as_ref().unwrap()[everruns_core::human_wait::METADATA_KEY][0];
    assert_eq!(wait["outcome"], "declined");
    assert_eq!(wait["started_at"], json!(started_at));
    assert_eq!(
        wait["turn_traceparent"],
        "00-1234567890abcdef1234567890abcdef-1234567890abcdef-01"
    );
}
