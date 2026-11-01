//! Eval runs as case tasks: repeated trials, pass^k summaries, cancellation.
//!
//! No model answers in these tests, so cases end by their own short timeout.
//! That still drives the whole lifecycle: one task per (case, trial), each
//! recording its outcome, and the last one to settle completing the run with
//! a summary rebuilt from the stored rows.

use crate::test_harness;

use axum::http::StatusCode;
use serde_json::{Value, json};
use test_harness::TestServer;

use everruns_server::domains::evals::record::{Eval, EvalCase};

async fn seed_eval(server: &TestServer, cases: usize, timeout_seconds: u32) -> Eval {
    let harness: Value = server
        .post("/v1/harnesses", json!({"name": "trials-harness"}))
        .await
        .assert_success()
        .json();
    let harness_id = harness["id"].as_str().expect("harness id");
    let eval: Eval = server
        .post(
            "/v1/evals",
            json!({
                "name": "trials eval",
                "target": {"type": "session", "harness_id": harness_id},
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    for index in 0..cases {
        let _case: EvalCase = server
            .post(
                &format!("/v1/evals/{}/cases", eval.public_id),
                json!({
                    "name": format!("case-{index}"),
                    "conversation": [{"content": "hello"}],
                    "scorers": [{"type": "contains", "text": "ok", "weight": 1.0}],
                    "timeout_seconds": timeout_seconds,
                }),
            )
            .await
            .assert_status(StatusCode::CREATED)
            .json();
    }
    eval
}

async fn get_run(server: &TestServer, eval: &Eval, run_id: &str) -> Value {
    server
        .get(&format!("/v1/evals/{}/runs/{run_id}", eval.public_id))
        .await
        .assert_status(StatusCode::OK)
        .json()
}

async fn wait_for_status(server: &TestServer, eval: &Eval, run_id: &str, status: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..120 {
        last = get_run(server, eval, run_id).await;
        if last["status"] == status {
            return last;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    panic!("run {run_id} never reached {status}: {last}");
}

#[tokio::test]
async fn trials_outside_bounds_are_rejected() {
    let server = TestServer::in_memory().await;
    let eval = seed_eval(&server, 1, 1).await;
    for trials in [0, 21] {
        let response = server
            .post(
                &format!("/v1/evals/{}/runs", eval.public_id),
                json!({"trials": trials}),
            )
            .await;
        assert_eq!(
            response.status(),
            StatusCode::BAD_REQUEST,
            "trials={trials}"
        );
        let _ = response.text();
    }
}

#[tokio::test]
async fn each_trial_is_a_result_and_the_last_one_completes_the_run() {
    let server = TestServer::in_memory().await;
    let eval = seed_eval(&server, 2, 1).await;

    let run: Value = server
        .post(
            &format!("/v1/evals/{}/runs", eval.public_id),
            json!({"trials": 3}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(run["trials"], 3);
    let run_id = run["id"].as_str().expect("run id").to_string();

    let done = wait_for_status(&server, &eval, &run_id, "completed").await;
    let results = done["results"].as_array().expect("results");
    assert_eq!(results.len(), 6, "2 cases x 3 trials");
    let mut trials: Vec<u64> = results
        .iter()
        .map(|r| r["trial"].as_u64().unwrap_or_default())
        .collect();
    trials.sort_unstable();
    assert_eq!(trials, vec![0, 0, 1, 1, 2, 2]);
    // No model answers, so every case ran out its one-second timeout.
    assert!(
        results.iter().all(|r| r["status"] == "timeout"),
        "{results:?}"
    );

    let summary = &done["summary"];
    assert_eq!(summary["total"], 6);
    assert_eq!(summary["errored"], 6);
    assert_eq!(summary["trials"], 3);
    assert_eq!(summary["cases"], 2);
    assert_eq!(summary["cases_passed_every_trial"], 0);
    assert_eq!(summary["cases_passed_any_trial"], 0);
}

#[tokio::test]
async fn cancelling_skips_unfinished_cases_and_keeps_the_run_cancelled() {
    let server = TestServer::in_memory().await;
    // Long timeout: the cases are still waiting when the run is cancelled.
    let eval = seed_eval(&server, 2, 60).await;
    let run: Value = server
        .post(
            &format!("/v1/evals/{}/runs", eval.public_id),
            json!({"trials": 2}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let run_id = run["id"].as_str().expect("run id").to_string();
    wait_for_status(&server, &eval, &run_id, "running").await;

    server
        .post(
            &format!("/v1/evals/{}/runs/{run_id}/cancel", eval.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);

    let cancelled = get_run(&server, &eval, &run_id).await;
    assert_eq!(cancelled["status"], "cancelled");
    let results = cancelled["results"].as_array().expect("results");
    assert_eq!(results.len(), 4);
    assert!(
        results.iter().all(|r| r["status"] == "skipped"),
        "{results:?}"
    );

    // The case tasks notice within a few polls and leave everything as is.
    tokio::time::sleep(std::time::Duration::from_secs(7)).await;
    let later = get_run(&server, &eval, &run_id).await;
    assert_eq!(later["status"], "cancelled");
    assert!(
        later["results"]
            .as_array()
            .expect("results")
            .iter()
            .all(|r| r["status"] == "skipped")
    );
}
