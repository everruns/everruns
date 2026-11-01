//! Experiments: an eval's cases at one dataset version against several
//! variants, compared case by case, with a gate for CI.
//!
//! No model answers in these tests, so cases end by their own short timeout.
//! That still drives each variant's run to completion and the comparison and
//! gate over the stored results.

use crate::test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

async fn seed_eval(server: &TestServer, cases: usize) -> Value {
    let harness: Value = server
        .post("/v1/harnesses", json!({"name": "experiments-harness"}))
        .await
        .assert_success()
        .json();
    let eval: Value = server
        .post(
            "/v1/evals",
            json!({
                "name": "experiments eval",
                "target": {"type": "session", "harness_id": harness["id"]},
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    for index in 0..cases {
        server
            .post(
                &format!("/v1/evals/{}/cases", eval["id"].as_str().unwrap()),
                json!({
                    "name": format!("case-{index}"),
                    "conversation": [{"content": "hello"}],
                    "scorers": [{"type": "contains", "text": "ok", "weight": 1.0}],
                    "timeout_seconds": 1,
                }),
            )
            .await
            .assert_status(StatusCode::CREATED);
    }
    server
        .get(&format!("/v1/evals/{}", eval["id"].as_str().unwrap()))
        .await
        .assert_status(StatusCode::OK)
        .json()
}

#[tokio::test]
async fn an_experiment_runs_each_variant_and_gates_on_the_comparison() {
    let server = TestServer::in_memory().await;
    let eval = seed_eval(&server, 2).await;
    let eval_id = eval["id"].as_str().unwrap().to_string();
    // Created at 1, bumped by each of the two cases.
    assert_eq!(eval["version"], 3);

    let experiment: Value = server
        .post(
            &format!("/v1/evals/{eval_id}/experiments"),
            json!({
                "variants": [
                    {"label": "base"},
                    {"label": "luna", "model_override": "gpt-6-luna"},
                ],
                "trials": 2,
                "max_cost_usd": 4.0,
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(experiment["name"], "base vs luna");
    assert_eq!(experiment["baseline"], "base");
    assert_eq!(experiment["dataset_version"], 3);
    assert_eq!(experiment["status"], "running");
    let experiment_id = experiment["id"].as_str().unwrap().to_string();
    assert!(experiment_id.starts_with("evalexp_"), "{experiment_id}");
    let variants = experiment["variants"].as_array().unwrap();
    assert_eq!(variants.len(), 2);

    // Each variant is an ordinary run with half the cap, tagged with the
    // experiment and the dataset version.
    let run: Value = server
        .get(&format!(
            "/v1/evals/{eval_id}/runs/{}",
            variants[1]["run_id"].as_str().unwrap()
        ))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(run["experiment_id"], experiment_id.as_str());
    assert_eq!(run["variant_label"], "luna");
    assert_eq!(run["dataset_version"], 3);
    assert_eq!(run["max_cost_usd"], 2.0);
    assert_eq!(run["model_override"], "gpt-6-luna");

    let gate = format!("/v1/evals/{eval_id}/experiments/{experiment_id}/gate");
    let early = server.get(&gate).await;
    assert_eq!(early.status(), StatusCode::CONFLICT);
    let _ = early.text();

    let path = format!("/v1/evals/{eval_id}/experiments/{experiment_id}");
    let mut done = Value::Null;
    for _ in 0..120 {
        done = server.get(&path).await.assert_status(StatusCode::OK).json();
        if done["status"] != "running" {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    assert_eq!(done["status"], "completed", "{done}");
    let comparison = &done["comparison"];
    assert_eq!(comparison["baseline"], "base");
    let cases = comparison["cases"].as_array().unwrap();
    assert_eq!(cases.len(), 2);
    assert_eq!(cases[0]["cells"][1]["label"], "luna");
    assert_eq!(cases[0]["cells"][1]["trials"], 2);
    assert_eq!(cases[0]["cells"][1]["passed"], 0);
    assert_eq!(cases[0]["changed"], false);
    // Two cases are too few to tell noise from a change.
    assert_eq!(comparison["verdicts"][0]["verdict"], "not_enough_data");

    // Every case timed out, so a pass-rate bar fails and says why.
    let missed = server.get(&format!("{gate}?min_pass_rate=0.5")).await;
    assert_eq!(missed.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = missed.text();
    assert!(body.contains("pass rate 0% is under 50%"), "{body}");
    // With no bar, a finished run passes.
    let passed: Value = server
        .get(&format!("{gate}?variant=base"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(passed["passed"], true);
    assert_eq!(passed["variant"], "base");

    let mine: Value = server
        .get(&format!("/v1/evals/{eval_id}/experiments"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    let all: Value = server
        .get("/v1/evals/experiments")
        .await
        .assert_status(StatusCode::OK)
        .json();
    for list in [&mine, &all] {
        let items = list["data"].as_array().unwrap_or_else(|| panic!("{list}"));
        assert_eq!(items[0]["id"], experiment_id.as_str());
        assert!(items[0].get("comparison").is_none());
    }

    // Changing a case is a new dataset version; the experiment keeps its own.
    let case_id = server
        .get(&format!("/v1/evals/{eval_id}/cases"))
        .await
        .assert_status(StatusCode::OK)
        .json::<Value>()["data"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    server
        .patch(
            &format!("/v1/evals/{eval_id}/cases/{case_id}"),
            json!({"name": "renamed"}),
        )
        .await
        .assert_status(StatusCode::OK);
    let eval: Value = server
        .get(&format!("/v1/evals/{eval_id}"))
        .await
        .assert_status(StatusCode::OK)
        .json();
    assert_eq!(eval["version"], 4);
}

#[tokio::test]
async fn experiments_outside_bounds_are_rejected() {
    let server = TestServer::in_memory().await;
    let eval = seed_eval(&server, 1).await;
    let path = format!("/v1/evals/{}/experiments", eval["id"].as_str().unwrap());
    let bad = [
        json!({"variants": []}),
        json!({"variants": [{"label": "a"}, {"label": "b"}, {"label": "c"}, {"label": "d"}, {"label": "e"}]}),
        json!({"variants": [{"label": "a"}, {"label": "a"}]}),
        json!({"variants": [{"label": "a"}], "trials": 0}),
        json!({"variants": [{"label": "a"}], "max_cost_usd": -1}),
    ];
    for body in bad {
        let response = server.post(&path, body.clone()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        let _ = response.text();
    }
}
