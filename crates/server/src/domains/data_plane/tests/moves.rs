//! Moving a running v1 session onto its org's data plane (`moves.rs`), on a
//! real database against the stub data plane: switched first, then its
//! conversation, storage, files and SQL databases imported, its v1 log
//! kept, and the session opened to messages last.

use base64::Engine as _;
use everruns_core::message::{ContentPart, RuntimeMessage};

use super::*;
use crate::domains::data_plane::moves::{self, Moved};
use crate::storage::{
    CreateEventRow, CreateSessionFileRow, UpsertSessionKeyValue, UpsertSessionSecret,
};

/// A v1 session of `ORG` that talked: a question, an answer that called a
/// tool, a second question; a value, a secret, a bound secret, two files.
async fn v1_session(cp: &ControlPlane) -> crate::storage::SessionRow {
    let harness = harness(&cp.db, ORG).await;
    let mut row = crate::storage::test_database::test_session_row(ORG);
    row.harness_id = Some(harness);
    row.system_prompt = Some("Answer briefly.".into());
    let session = cp.db.create_session(row).await.unwrap();
    let mut answer = RuntimeMessage::assistant("first answer");
    answer.content.push(ContentPart::tool_call(
        "call_1",
        "search",
        serde_json::json!({}),
    ));
    for (event_type, data) in [
        ("turn.started", json!({})),
        (
            "input.message",
            json!({ "message": RuntimeMessage::user("first question") }),
        ),
        ("output.message.completed", json!({ "message": answer })),
        ("turn.completed", json!({})),
        (
            "input.message",
            json!({ "message": RuntimeMessage::user("second question") }),
        ),
    ] {
        cp.db
            .create_event(CreateEventRow {
                session_id: session.id,
                event_type: event_type.into(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data,
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
    }
    cp.db
        .upsert_session_key_value(UpsertSessionKeyValue {
            session_id: session.id,
            key: "plan".into(),
            value: "step 2".into(),
        })
        .await
        .unwrap();
    let encryption = cp.encryption().unwrap();
    for (name, binding) in [
        ("API_TOKEN", None),
        (
            "BOUND_TOKEN",
            Some(
                serde_json::from_value(json!({
                    "url": "https://api.example.com",
                    "auth": { "type": "bearer" },
                }))
                .unwrap(),
            ),
        ),
    ] {
        cp.db
            .upsert_session_secret(UpsertSessionSecret {
                session_id: session.id,
                name: name.into(),
                value_encrypted: encryption.encrypt_string("s3cret").unwrap(),
                binding,
            })
            .await
            .unwrap();
    }
    let workspace = everruns_contracts::typed_id::SessionId::from_uuid(session.workspace_id);
    for (path, content) in [
        ("/notes.md", b"# Notes".to_vec()),
        ("/data.bin", vec![0xff, 0x00, 0xfe]),
    ] {
        cp.db
            .create_session_file(CreateSessionFileRow {
                session_id: workspace,
                path: path.into(),
                content: Some(content),
                is_directory: false,
                is_readonly: false,
            })
            .await
            .unwrap();
    }
    let sqldb = crate::session_sqldb::PostgresSqlDbStore::new(cp.db.database().clone());
    use everruns_contracts::session_sqldb::SessionSqlDbStore as _;
    sqldb
        .sql_execute(
            session.id,
            "notes",
            "CREATE TABLE notes (body TEXT); INSERT INTO notes VALUES ('kept');",
        )
        .await
        .unwrap();
    session
}

fn commands(seen: &Stub, session: &str, command: &str) -> Vec<Value> {
    seen.lock()
        .commands
        .iter()
        .filter(|(s, c, _)| s == session && c == command)
        .map(|(_, _, body)| body.clone())
        .collect()
}

#[tokio::test]
async fn a_running_v1_session_moves_onto_its_data_plane_with_its_conversation_storage_and_files() {
    let (cp, seen, _) = control_plane().await;
    let session = v1_session(&cp).await;
    let id = session.id;
    let public = id.to_string();
    assert!(!sessions::runs_on_data_plane(&cp, ORG, id).await.unwrap());

    let Moved::Done(report) = moves::move_session(&cp, ORG, id, Duration::from_secs(5))
        .await
        .unwrap()
    else {
        panic!("moved");
    };

    // Created under its own id, with its own settings as a layer.
    {
        let seen = seen.lock();
        let (_, body, _) = seen.sessions.last().expect("created on the data plane");
        assert_eq!(body["id"], json!(public));
        assert!(
            body["definition"].to_string().contains("Answer briefly."),
            "{}",
            body["definition"]
        );
    }
    // The conversation, without the tool call; the v1 log stays.
    let seeded = commands(&seen, &public, "seed");
    assert_eq!(seeded.len(), 1);
    let messages = seeded[0]["messages"].as_array().unwrap();
    assert_eq!(messages.len(), 3);
    assert!(!seeded[0].to_string().contains("call_1"));
    assert_eq!(report.messages, 3);
    assert_eq!(
        cp.db
            .list_events(id, None, None, &[], &[], None, None)
            .await
            .unwrap()
            .len(),
        5,
        "the v1 log is kept"
    );
    // Storage: the value, the secret sealed for the session; the bound
    // secret stays behind.
    let storage = commands(&seen, &public, "storage");
    assert!(
        storage
            .iter()
            .any(|b| b["kind"] == "set_value" && b["value"] == "step 2")
    );
    let sealed = storage
        .iter()
        .find(|b| b["kind"] == "set_secret")
        .expect("the secret is copied");
    assert_eq!(sealed["name"], "API_TOKEN");
    let org = cp.db.dp_org(ORG).await.unwrap().unwrap();
    let opened = everruns_contracts::dataplane_seal::open_session_secret(
        &cp.data_key(&org).unwrap(),
        &org.data_key_id,
        &org_public(ORG),
        &public,
        "API_TOKEN",
        sealed["sealed"].as_str().unwrap(),
    )
    .unwrap();
    assert_eq!(opened, "s3cret");
    assert!(!storage.iter().any(|b| b["name"] == "BOUND_TOKEN"));
    assert_eq!((report.values, report.secrets), (1, 1));
    assert!(
        report.left_behind.iter().any(|l| l.contains("BOUND_TOKEN")),
        "{report:?}"
    );
    // Files, a binary one base64 encoded.
    let files = commands(&seen, &public, "fs");
    let written: Vec<&Value> = files
        .iter()
        .flat_map(|b| b["files"].as_array().unwrap())
        .collect();
    assert_eq!(written.len(), 2);
    assert!(
        written
            .iter()
            .any(|f| f["path"] == "/data.bin" && f["encoding"] == "base64")
    );
    assert_eq!(report.files, 2);
    // The SQL database, whole: its pages in order are its SQLite file.
    let databases = commands(&seen, &public, "sqldb");
    assert_eq!(databases.len(), 1);
    assert_eq!(databases[0]["op"], "import");
    assert_eq!(databases[0]["name"], "notes");
    let file = base64::engine::general_purpose::STANDARD
        .decode(databases[0]["file"].as_str().unwrap())
        .unwrap();
    assert!(file.starts_with(b"SQLite format 3\0"));
    assert_eq!(report.databases, 1);
    // Created closed to messages, and opened only once everything is in.
    {
        let seen = seen.lock();
        let (_, body, _) = seen.sessions.last().unwrap();
        assert_eq!(body["importing"], json!(true));
        let order: Vec<&str> = seen
            .commands
            .iter()
            .filter(|(s, _, _)| s == &public)
            .map(|(_, c, _)| c.as_str())
            .collect();
        assert_eq!(order.last(), Some(&"activate"), "{order:?}");
        assert_eq!(order.iter().filter(|c| **c == "activate").count(), 1);
    }

    // Served from the data plane now, with the move on record.
    assert!(sessions::runs_on_data_plane(&cp, ORG, id).await.unwrap());
    assert!(!cp.db.dp_session_moving(&public).await.unwrap());
    let moved = cp.db.dp_session_move(&public).await.unwrap().unwrap();
    assert_eq!(moved.state, "done");
    assert!(moved.from_sequence.is_some());
    assert_eq!(
        moves::move_session(&cp, ORG, id, Duration::from_secs(5))
            .await
            .unwrap(),
        Moved::Skipped("already moved".into())
    );
}

#[tokio::test]
async fn a_session_in_a_turn_is_not_moved_and_an_org_moves_the_rest() {
    let (cp, seen, _) = control_plane().await;
    let idle = v1_session(&cp).await;
    let busy = v1_session(&cp).await;
    cp.db
        .set_session_status(ORG, busy.id, "waiting_for_tool_results")
        .await
        .unwrap();

    let outcomes = moves::move_org(&cp, ORG, false, Duration::from_secs(5))
        .await
        .unwrap();
    let of = |id| {
        outcomes
            .iter()
            .find(|(session, _)| *session == id)
            .map(|(_, outcome)| outcome.as_ref().unwrap())
    };
    assert!(matches!(of(idle.id), Some(Moved::Done(_))));
    let Some(Moved::Skipped(reason)) = of(busy.id) else {
        panic!("{outcomes:?}");
    };
    assert!(reason.contains("tool results"), "{reason}");
    // Nothing happened to the busy one: still v1, nothing sent.
    assert!(
        !sessions::runs_on_data_plane(&cp, ORG, busy.id)
            .await
            .unwrap()
    );
    assert!(commands(&seen, &busy.id.to_string(), "seed").is_empty());
}

#[tokio::test]
async fn a_batch_enables_v1_orgs_moves_their_sessions_and_leaves_other_planes_alone() {
    let (cp, _, _) = control_plane().await;
    let idle = v1_session(&cp).await;
    // ORG is on dp-1 already; ORG + 1 is still v1; ORG + 2 is on dp-2.
    let v1_org = ORG + 1;
    let stub = cp.db.dp_data_plane("dp-1").await.unwrap().unwrap().url;
    cp.db
        .dp_upsert_data_plane("dp-2", &stub, &stub)
        .await
        .unwrap();
    enable_org(&cp, ORG + 2, "dp-2").await.unwrap();

    let reports = moves::move_orgs(
        &cp,
        "dp-1",
        &[ORG, v1_org, ORG + 2],
        false,
        Duration::from_secs(5),
    )
    .await;
    assert_eq!(reports.len(), 3);
    assert_eq!(
        (reports[0].moved, reports[0].enabled),
        (1, false),
        "{reports:?}"
    );
    assert!(reports[0].error.is_none(), "{reports:?}");
    assert!(
        sessions::runs_on_data_plane(&cp, ORG, idle.id)
            .await
            .unwrap()
    );
    // The v1 org is enabled on the plane (it had no sessions to move).
    assert!(reports[1].enabled, "{reports:?}");
    assert_eq!(
        cp.db.dp_org(v1_org).await.unwrap().unwrap().data_plane_id,
        "dp-1"
    );
    // The org on another plane is reported and not touched.
    assert!(
        reports[2].error.as_deref().unwrap().contains("dp-2"),
        "{reports:?}"
    );
    assert_eq!(
        cp.db.dp_org(ORG + 2).await.unwrap().unwrap().data_plane_id,
        "dp-2"
    );

    // Again: nothing left to move, nothing enabled twice.
    let again = moves::move_orgs(&cp, "dp-1", &[ORG, v1_org], false, Duration::from_secs(5)).await;
    assert!(
        again
            .iter()
            .all(|r| r.moved == 0 && !r.enabled && r.error.is_none()),
        "{again:?}"
    );
}

#[tokio::test]
async fn a_moving_session_refuses_messages_until_its_import_is_done() {
    let (cp, _, _) = control_plane().await;
    let session = v1_session(&cp).await;
    let public = session.id.to_string();
    assert!(
        cp.db
            .dp_begin_session_move(ORG, &public, "dp-1", "agent-key")
            .await
            .unwrap()
    );
    assert!(cp.db.dp_session_moving(&public).await.unwrap());
    assert!(
        sessions::runs_on_data_plane(&cp, ORG, session.id)
            .await
            .unwrap(),
        "its messages no longer reach the platform's workers"
    );
    // Switching again does nothing; the move resumes from where it stopped.
    assert!(
        !cp.db
            .dp_begin_session_move(ORG, &public, "dp-1", "agent-key")
            .await
            .unwrap()
    );
    assert!(matches!(
        moves::move_session(&cp, ORG, session.id, Duration::from_secs(5))
            .await
            .unwrap(),
        Moved::Done(_)
    ));
    assert!(!cp.db.dp_session_moving(&public).await.unwrap());
}

/// A fork of a v2 session gets its parent's files, as a v1 fork copies its
/// workspace: from one snapshot, or file by file past a snapshot's limit.
#[tokio::test]
async fn a_v2_forks_files_come_from_a_snapshot_or_file_by_file() {
    let (cp, seen, _) = control_plane().await;
    let now = chrono::Utc::now();
    let ids: Vec<String> = (0..3)
        .map(|_| everruns_contracts::typed_id::SessionId::new().to_string())
        .collect();
    for id in &ids {
        cp.db
            .dp_upsert_session_summary(crate::storage::DpSessionSummary {
                session_id: id,
                org_id: ORG,
                data_plane_id: "dp-1",
                agent_id: "agent",
                session_version: 0,
                title: "",
                status: "started",
                turns: 0,
                cost_micros: 0,
                created_at: now,
                last_activity_at: now,
            })
            .await
            .unwrap();
    }
    let plane = |id: &str| {
        let cp = cp.clone();
        let id = everruns_contracts::typed_id::SessionId::parse(id).unwrap();
        async move {
            crate::domains::data_plane::session_plane::SessionPlane::with(&cp, ORG, id)
                .await
                .unwrap()
                .unwrap()
        }
    };
    seen.lock().files.insert(
        ids[0].clone(),
        vec![
            json!({ "path": "/notes.md", "encoding": "text", "content": "# Notes", "readonly": false }),
            json!({ "path": "/data.bin", "encoding": "base64", "content": "/wD+", "readonly": true }),
        ],
    );
    let written = |session: &str| -> Vec<Value> {
        commands(&seen, session, "fs")
            .iter()
            .filter(|b| b["op"] == "seed")
            .flat_map(|b| b["files"].as_array().unwrap().clone())
            .collect()
    };

    let copied = moves::copy_files(&plane(&ids[0]).await, &plane(&ids[1]).await)
        .await
        .unwrap();
    assert_eq!(copied, 2);
    let child = written(&ids[1]);
    assert_eq!(child.len(), 2, "{child:?}");
    assert!(child.iter().any(|f| f["path"] == "/data.bin"
        && f["encoding"] == "base64"
        && f["is_readonly"] == true));

    seen.lock().snapshot_refused = true;
    let copied = moves::copy_files(&plane(&ids[0]).await, &plane(&ids[2]).await)
        .await
        .unwrap();
    assert_eq!(copied, 2, "file by file, the directory skipped");
    let child = written(&ids[2]);
    assert!(
        child
            .iter()
            .any(|f| f["path"] == "/notes.md" && f["content"] == "# Notes")
    );
    assert!(
        child
            .iter()
            .any(|f| f["path"] == "/data.bin" && f["is_readonly"] == true)
    );
}
