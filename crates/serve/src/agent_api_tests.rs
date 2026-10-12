//! The Agent Execution API over a real socket: one agent's sessions under its
//! agent base URL, confined to that agent. Uses the agents of `wire_tests`
//! (`tester`, the default, and `asker`).

use serde_json::{Value, json};

use crate::app::Mode;
use crate::host::Host;
use crate::wire_tests::{Server, app, serve, text_message};

async fn create(server: &Server, agent: &str) -> Value {
    let response = server
        .post(
            &format!("/v1/channels/{agent}/sessions"),
            json!({ "title": "From code", "metadata": { "ticket": "T-1" } }),
        )
        .await;
    assert_eq!(response.status(), 201);
    let location = response.headers()["location"].to_str().unwrap().to_string();
    let session: Value = response.json().await.unwrap();
    assert_eq!(
        location,
        format!(
            "/v1/channels/{agent}/sessions/{}",
            session["id"].as_str().unwrap()
        )
    );
    session
}

#[tokio::test]
async fn the_card_names_the_agent_and_its_session_collection() {
    let server = serve(Host::new(app(), Mode::Eval, None).unwrap()).await;
    let card: Value = server
        .get("/v1/channels/tester")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(card["name"], "tester");
    assert_eq!(card["streaming"], true);
    assert_eq!(
        card["input"],
        json!({ "text": true, "images": false, "files": false })
    );
    assert_eq!(card["auth"], json!([]));
    assert_eq!(card["links"]["sessions"], "/v1/channels/tester/sessions");
    // The card is the shared contract type, so it parses as one.
    let typed: everruns::execution_api::AgentCard = serde_json::from_value(card).unwrap();
    assert_eq!(typed.name, "tester");

    assert_eq!(server.get("/v1/channels/nobody").await.status(), 404);
}

#[tokio::test]
async fn a_session_runs_the_agent_its_base_url_names() {
    let server = serve(Host::new(app(), Mode::Eval, None).unwrap()).await;
    // `asker` is not the default agent; the URL alone picks it.
    let session = create(&server, "asker").await;
    let id = session["id"].as_str().unwrap();
    assert_eq!(session["agent_name"], "asker");
    assert_eq!(session["title"], "From code");
    assert_eq!(session["metadata"], json!({ "ticket": "T-1" }));

    let base = format!("/v1/channels/asker/sessions/{id}");
    let sent = server
        .post(&format!("{base}/messages"), text_message("deploy"))
        .await;
    assert_eq!(sent.status(), 201);
    let waiting = server
        .wait_until(id, |s| {
            !s["pending_questions"].as_array().unwrap().is_empty()
        })
        .await;
    let call = waiting["pending_questions"][0]["tool_call_id"].clone();
    let question = waiting["pending_questions"][0]["questions"][0]["id"].clone();
    let answered = server
        .post(
            &format!("{base}/question-answers"),
            json!({
                "tool_call_id": call,
                "answers": [{ "id": question, "selected": ["Staging"] }],
            }),
        )
        .await;
    assert_eq!(answered.status(), 200);
    server.wait_turns(id, 1).await;

    let read: Value = server.get(&base).await.json().await.unwrap();
    assert_eq!(read["id"], id);
    let events: Value = server
        .get(&format!("{base}/events?types=turn.completed"))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(events["data"].as_array().unwrap().len(), 1);
    let cancelled: Value = server
        .post(&format!("{base}/cancel"), json!({}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(cancelled["status"], "no_op");
}

#[tokio::test]
async fn another_agents_base_url_cannot_reach_a_session() {
    let server = serve(Host::new(app(), Mode::Eval, None).unwrap()).await;
    let session = create(&server, "asker").await;
    let id = session["id"].as_str().unwrap();
    let other = format!("/v1/channels/tester/sessions/{id}");
    assert_eq!(server.get(&other).await.status(), 404);
    assert_eq!(server.get(&format!("{other}/events")).await.status(), 404);
    assert_eq!(server.get(&format!("{other}/sse")).await.status(), 404);
    assert_eq!(
        server
            .post(&format!("{other}/messages"), text_message("hi"))
            .await
            .status(),
        404
    );
    assert_eq!(
        server
            .post(&format!("{other}/cancel"), json!({}))
            .await
            .status(),
        404
    );
    assert_eq!(
        server
            .post(
                &format!("{other}/tool-approvals"),
                json!({ "decisions": [{ "tool_call_id": "x", "decision": "allow" }] }),
            )
            .await
            .status(),
        404
    );
    // An unknown agent is as absent as an unknown session.
    assert_eq!(
        server
            .post("/v1/channels/nobody/sessions", json!({}))
            .await
            .status(),
        404
    );
}

#[tokio::test]
async fn the_session_list_holds_only_that_agents_sessions() {
    let server = serve(Host::new(app(), Mode::Eval, None).unwrap()).await;
    let first = create(&server, "asker").await;
    create(&server, "tester").await;
    let second = create(&server, "asker").await;
    let list: Value = server
        .get("/v1/channels/asker/sessions")
        .await
        .json()
        .await
        .unwrap();
    let ids: Vec<&Value> = list["data"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| &s["id"])
        .collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&&first["id"]) && ids.contains(&&second["id"]));
    assert_eq!(
        server
            .get("/v1/channels/asker/sessions?limit=0")
            .await
            .status(),
        400
    );
}

/// The server's batch approval shape: named decisions apply, and silence
/// never approves.
#[tokio::test]
async fn tool_approvals_take_the_servers_batch_shape() {
    let server = serve(Host::new(app(), Mode::Eval, None).unwrap()).await;
    let session = create(&server, "tester").await;
    let id = session["id"].as_str().unwrap();
    let base = format!("/v1/channels/tester/sessions/{id}");
    server
        .post(&format!("{base}/messages"), text_message("first"))
        .await;
    server.wait_turns(id, 1).await;
    server
        .post(&format!("{base}/messages"), text_message("second"))
        .await;
    let waiting = server
        .wait_until(id, |s| s["status"] == "waitingfortoolresults")
        .await;
    let call = waiting["pending_approvals"][0]["tool_call_id"]
        .as_str()
        .unwrap()
        .to_string();

    let empty = server
        .post(
            &format!("{base}/tool-approvals"),
            json!({ "decisions": [] }),
        )
        .await;
    assert_eq!(empty.status(), 400);
    let unknown = server
        .post(
            &format!("{base}/tool-approvals"),
            json!({ "decisions": [{ "tool_call_id": "call_nope", "decision": "allow" }] }),
        )
        .await;
    assert_eq!(unknown.status(), 404);
    let twice = server
        .post(
            &format!("{base}/tool-approvals"),
            json!({ "decisions": [
                { "tool_call_id": call, "decision": "allow" },
                { "tool_call_id": call, "decision": "reject" },
            ] }),
        )
        .await;
    assert_eq!(twice.status(), 400);

    let response = server
        .post(
            &format!("{base}/tool-approvals"),
            json!({ "decisions": [{ "tool_call_id": call, "decision": "allow" }] }),
        )
        .await;
    assert_eq!(response.status(), 200);
    let body: everruns::execution_api::SubmitToolApprovalsResponse = response.json().await.unwrap();
    assert_eq!(body.resolved.len(), 1);
    assert_eq!(body.resolved[0].tool, "guarded");
    assert_eq!(body.resolved[0].outcome, "allow");

    let events = server.wait_turns(id, 2).await;
    let completed = events
        .iter()
        .rev()
        .find(|e| e["type"] == "tool.completed")
        .unwrap();
    assert_eq!(completed["data"]["tool_name"], "guarded");
    assert_eq!(completed["data"]["success"], true);
}
