//! Moving a running v1 session onto its org's data plane (`moves.rs`), on a
//! real database against the stub data plane: switched first, then its
//! conversation, storage and files imported, its v1 log kept.

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
