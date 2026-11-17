//! A v2 session's reads and recorded events for the control plane (data
//! plane v2, step 11.6): the control plane serves a session's Trace view,
//! reactions, ratings, context meter and export for a v2 session from the
//! session's events here, and records a reaction on the session's own log.
//! The session's own `/v1` events take the console's search filters and
//! summary, and its storage is read and changed by the control plane, with
//! secrets that arrive sealed.

#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod support;

use everruns_cell_testkit::fake_cp::FakeCp;
use everruns_contracts::typed_id::{EventId, SessionId};
use everruns_core::{EventContext, EventData, EventRequest, MessageReactionData};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use support::Fleet;
use support::org::{DATA_KEY, DATA_KEY_ID, ORG, command, org_executor, org_fleet, session};

/// One of the session's reads, as the control plane asks it.
async fn read(fleet: &Fleet, cp: &FakeCp, session: &str, read: &str) -> (StatusCode, Value) {
    cp.call(
        &fleet.base(),
        Method::GET,
        &format!("/dp/v1/orgs/{ORG}/sessions/{session}/{read}"),
        &Value::Null,
    )
    .await
    .unwrap()
}

fn reaction(session: &str, message: &str) -> EventRequest {
    let data: MessageReactionData = serde_json::from_value(json!({
        "message_id": message,
        "emoji": "👍",
        "actor": { "kind": "user", "id": "user_1", "name": "Ann" },
        "source": "console",
    }))
    .unwrap();
    EventRequest::new(
        SessionId::parse(session).unwrap(),
        EventContext::empty(),
        EventData::message_reaction(data, true),
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn the_control_plane_reads_a_v2_session_and_records_a_reaction() {
    let (fleet, cp) = org_fleet(&[]).await;
    let session = session(&fleet, &cp).await;
    let message = json!({ "id": everruns_contracts::typed_id::MessageId::new(), "text": "ping" });
    let (status, _) = command(&fleet, &cp, ORG, &session, "messages", message).await;
    assert_eq!(status, StatusCode::OK);
    fleet.turns(&session, 1).await;

    // The Trace view's rows: one completed turn.
    let (status, trace) = read(&fleet, &cp, &session, "trace").await;
    assert_eq!(status, StatusCode::OK, "{trace}");
    assert_eq!(trace["turn_count"], 1, "{trace}");
    assert_eq!(trace["turns"][0]["status"], "completed", "{trace}");

    // The agent's reply, by type, then by its message id.
    let (_, replies) = read(
        &fleet,
        &cp,
        &session,
        "events?types=output.message.completed&last=1",
    )
    .await;
    let reply = replies[0]["data"]["message"]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let (_, found) = read(&fleet, &cp, &session, &format!("events?messages={reply}")).await;
    assert_eq!(found.as_array().unwrap().len(), 1, "{found}");
    assert_eq!(found[0]["event_type"], "output.message.completed");

    // A reaction is recorded on the session's log once, however often it
    // is sent; the control plane may record nothing else.
    let body = json!({ "event_id": EventId::new(), "event": reaction(&session, &reply) });
    for _ in 0..2 {
        let (status, recorded) = command(&fleet, &cp, ORG, &session, "record", body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{recorded}");
        assert_eq!(recorded["type"], "message.reaction.added", "{recorded}");
    }
    let (_, reactions) = read(&fleet, &cp, &session, "events?types=message.reaction.added").await;
    assert_eq!(reactions.as_array().unwrap().len(), 1, "{reactions}");
    let mut forged = reaction(&session, &reply);
    forged.event_type = "input.message".into();

    let (status, _) = command(
        &fleet,
        &cp,
        ORG,
        &session,
        "record",
        json!({ "event_id": EventId::new(), "event": forged }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    let other = self::session(&fleet, &cp).await;
    let (status, _) = command(
        &fleet,
        &cp,
        ORG,
        &other,
        "record",
        json!({ "event_id": EventId::new(), "event": reaction(&session, &reply) }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "another session's event");

    // The whole log, for an export: every event, deltas left out.
    let (status, log) = read(&fleet, &cp, &session, "log?after=0").await;
    assert_eq!(status, StatusCode::OK, "{log}");
    let log = log.as_array().unwrap();
    assert!(log.iter().any(|e| e["type"] == "message.reaction.added"));
    assert!(
        log.iter()
            .all(|e| !e["type"].as_str().unwrap().ends_with(".delta"))
    );

    // The console's search and summary on the session's own events.
    let search = fleet
        .get(&format!(
            "/v1/sessions/{session}/events?q=PING&types=input.message"
        ))
        .await;
    assert_eq!(search["data"].as_array().unwrap().len(), 1, "{search}");
    let summary = fleet
        .get(&format!("/v1/sessions/{session}/events/summary"))
        .await;
    assert_eq!(summary["turn_count"], 1, "{summary}");
    assert_eq!(summary["error_count"], 0, "{summary}");

    // Another org's session reads as missing; an unknown read is refused.
    let (status, _) = cp
        .call(
            &fleet.base(),
            Method::GET,
            &format!("/dp/v1/orgs/org_other/sessions/{session}/trace"),
            &Value::Null,
        )
        .await
        .unwrap();
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) = read(&fleet, &cp, &session, "secrets").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test(flavor = "multi_thread")]
async fn the_control_plane_reads_and_changes_a_v2_sessions_storage() {
    use everruns_contracts::dataplane_seal as seal;
    let (fleet, cp) = org_fleet(&[]).await;
    let session = session(&fleet, &cp).await;
    let storage = |body: Value| command(&fleet, &cp, ORG, &session, "storage", body);

    let (status, set) =
        storage(json!({ "kind": "set_value", "key": "tool_approval/mode", "value": "auto" })).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    let sealed =
        seal::seal_session_secret(&DATA_KEY, DATA_KEY_ID, ORG, &session, "TOKEN", "hunter2")
            .unwrap();
    let (status, set) =
        storage(json!({ "kind": "set_secret", "name": "TOKEN", "sealed": sealed })).await;
    assert_eq!(status, StatusCode::OK, "{set}");

    // A secret that is not sealed for this session never enters its log.
    let (status, _) =
        storage(json!({ "kind": "set_secret", "name": "PLAIN", "sealed": "hunter2" })).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let other = self::session(&fleet, &cp).await;
    let elsewhere =
        seal::seal_session_secret(&DATA_KEY, DATA_KEY_ID, ORG, &other, "TOKEN", "x").unwrap();
    let (status, _) =
        storage(json!({ "kind": "set_secret", "name": "TOKEN", "sealed": elsewhere })).await;
    assert_eq!(status, StatusCode::CONFLICT, "sealed for another session");

    // Values and secret names, never a secret's value.
    let (status, listed) = read(&fleet, &cp, &session, "storage").await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["values"]["tool_approval/mode"], "auto", "{listed}");
    assert_eq!(listed["secrets"], json!(["TOKEN"]), "{listed}");
    assert!(!listed.to_string().contains("ciphertext"), "{listed}");

    let (_, deleted) = storage(json!({ "kind": "delete_secret", "name": "TOKEN" })).await;
    assert_eq!(deleted, json!(true));
    let (_, deleted) = storage(json!({ "kind": "delete_secret", "name": "TOKEN" })).await;
    assert_eq!(deleted, json!(false));
    let (_, listed) = read(&fleet, &cp, &session, "storage").await;
    assert_eq!(listed["secrets"], json!([]), "{listed}");

    // The executor opens the org's data key that session secrets are
    // sealed with.
    let (key, id) = org_executor(&fleet).data_key(ORG).await.unwrap();
    assert_eq!((key, id.as_str()), (DATA_KEY, DATA_KEY_ID));
}
