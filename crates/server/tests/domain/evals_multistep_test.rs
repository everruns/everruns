//! Multistep eval cases: a simulated user and tool fixtures stored on a
//! case, validated on write, and a run limited to one case ("Try once").
//!
//! No model answers in these tests, so the conversation itself is verified
//! against real models on a local stack; here the harness has no model stack
//! and a simulated case reports that instead of hanging.

use crate::test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

async fn seed_eval(server: &TestServer) -> String {
    let harness: Value = server
        .post("/v1/harnesses", json!({"name": "multistep-harness"}))
        .await
        .assert_success()
        .json();
    let eval: Value = server
        .post(
            "/v1/evals",
            json!({
                "name": "multistep eval",
                "target": {"type": "session", "harness_id": harness["id"]},
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    eval["id"].as_str().unwrap().to_string()
}

fn simulated_case() -> Value {
    json!({
        "name": "refund, angry customer",
        "simulated_user": {
            "persona": "Small shop owner charged twice for the Pro plan.",
            "goal": "Get the duplicate charge refunded.",
            "done_when": "A refund ticket exists with the right order number.",
            "max_turns": 4,
        },
        "tool_fixtures": [{
            "name": "create_ticket",
            "description": "Open a helpdesk ticket.",
            "parameters": {"type": "object", "properties": {"order": {"type": "string"}}},
            "response": {"ticket": "T-1"},
        }],
        "scorers": [{
            "type": "tool_called",
            "tool": "create_ticket",
            "arguments_contain": "ORD-48213",
        }],
        "timeout_seconds": 2,
    })
}

#[tokio::test]
async fn multistep_cases_round_trip_and_validate() {
    let server = TestServer::in_memory().await;
    let eval_id = seed_eval(&server).await;
    let cases = format!("/v1/evals/{eval_id}/cases");

    let case: Value = server
        .post(&cases, simulated_case())
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(case["conversation"], json!([]));
    assert_eq!(case["simulated_user"]["max_turns"], 4);
    assert_eq!(case["tool_fixtures"][0]["name"], "create_ticket");
    assert_eq!(case["scorers"][0]["arguments_contain"], "ORD-48213");
    let case_id = case["id"].as_str().unwrap().to_string();

    // Each refusal names what is wrong.
    let refusals = [
        (
            json!({"name": "nothing to say", "scorers": []}),
            "conversation",
        ),
        (
            {
                let mut c = simulated_case();
                c["post"] = json!([{"content": "did it work?"}]);
                c
            },
            "post",
        ),
        (
            {
                let mut c = simulated_case();
                c["simulated_user"]["max_turns"] = json!(31);
                c
            },
            "max_turns",
        ),
        (
            {
                let mut c = simulated_case();
                c["simulated_user"]["goal"] = json!("  ");
                c
            },
            "goal",
        ),
        (
            {
                let mut c = simulated_case();
                c["tool_fixtures"][0]["name"] = json!("billing.lookup");
                c
            },
            "name",
        ),
    ];
    for (body, mention) in refusals {
        let response = server.post(&cases, body).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let text = response.text();
        assert!(text.contains(mention), "{mention}: {text}");
    }

    // Clearing the simulator leaves a case with nothing to send: refused.
    let cleared = server
        .patch(
            &format!("{cases}/{case_id}"),
            json!({"simulated_user": null}),
        )
        .await;
    assert_eq!(cleared.status(), StatusCode::BAD_REQUEST);
    let _ = cleared.text();
    // With a scripted message it is a plain case again.
    let scripted: Value = server
        .patch(
            &format!("{cases}/{case_id}"),
            json!({
                "simulated_user": null,
                "tool_fixtures": null,
                "conversation": [{"content": "I was charged twice"}],
            }),
        )
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert!(
        scripted.get("simulated_user").is_none_or(Value::is_null),
        "{scripted}"
    );
    assert!(
        scripted.get("tool_fixtures").is_none_or(Value::is_null),
        "{scripted}"
    );
}

#[tokio::test]
async fn try_once_runs_only_the_selected_case() {
    let server = TestServer::in_memory().await;
    let eval_id = seed_eval(&server).await;
    let cases = format!("/v1/evals/{eval_id}/cases");
    let simulated: Value = server
        .post(&cases, simulated_case())
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    server
        .post(
            &cases,
            json!({
                "name": "scripted",
                "conversation": [{"content": "hello"}],
                "scorers": [],
                "timeout_seconds": 1,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED);

    let unknown = server
        .post(
            &format!("/v1/evals/{eval_id}/runs"),
            json!({"case_ids": ["evalcase_00000000000000000000000000000000"]}),
        )
        .await;
    assert_eq!(unknown.status(), StatusCode::BAD_REQUEST);
    let _ = unknown.text();

    let run: Value = server
        .post(
            &format!("/v1/evals/{eval_id}/runs"),
            json!({"case_ids": [simulated["id"]]}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let path = format!("/v1/evals/{eval_id}/runs/{}", run["id"].as_str().unwrap());
    let mut done = Value::Null;
    for _ in 0..80 {
        done = server.get(&path).await.assert_status(StatusCode::OK).json();
        if done["status"] != "running" && done["status"] != "pending" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let results = done["results"].as_array().unwrap();
    assert_eq!(results.len(), 1, "{done}");
    assert_eq!(results[0]["case_name"], "refund, angry customer");
    // No model stack here: the case says so before it starts a session.
    assert_eq!(results[0]["status"], "errored", "{done}");
    assert!(
        results[0]["error_message"]
            .as_str()
            .unwrap_or_default()
            .contains("simulated user"),
        "{done}"
    );
    assert!(results[0]["session_id"].is_null(), "{done}");
}
