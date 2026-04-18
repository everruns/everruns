//! Eval write-back integration tests.
//!
//! Focused on deferred external scoring: update completed results, persist scorer
//! metadata, and recompute run summaries without re-running sessions.

mod test_harness;

use axum::http::StatusCode;
use serde_json::json;
use test_harness::TestServer;

use everruns_core::eval::{Eval, EvalCase};
use everruns_core::typed_id::{EvalResultId, EvalRunId};
use everruns_server::storage::models::{
    CreateEvalCaseResultRow, CreateEvalRunRow, UpdateEvalCaseResultRow,
};

const TEST_ORG_ID: i64 = 1;
const TEST_HARNESS_ID: &str = "harness_01933b5a000070008000000000000601";

#[tokio::test]
async fn test_update_eval_result_scores_recomputes_run_summary() {
    let server = TestServer::in_memory().await;
    let seeded = seed_completed_run(&server).await;
    let completed_at_before = run_completed_at(&server, &seeded.run_id).await;

    let response = server
        .patch(
            &format!(
                "/v1/evals/{}/runs/{}/results/{}/scores",
                seeded.eval_id, seeded.run_id, seeded.result_ids[0]
            ),
            json!({
                "scores": [{"pass": true, "value": 1.0, "reason": "external harness passed"}],
                "metadata": {
                    "scorer": "swebench-docker-harness",
                    "scored_at": "2026-04-17T12:00:00Z"
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    let body = response.json_value();
    assert_eq!(body["status"], "passed");
    assert_eq!(body["metadata"]["scorer"], "swebench-docker-harness");
    assert_eq!(body["scores"][0]["value"], 1.0);

    let run = server
        .get(&format!(
            "/v1/evals/{}/runs/{}",
            seeded.eval_id, seeded.run_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json_value();

    assert_eq!(run["summary"]["passed"], 2);
    assert_eq!(run["summary"]["failed"], 0);
    assert_eq!(run["summary"]["errored"], 0);
    assert_eq!(run["summary"]["pass_rate"], 1.0);
    assert_eq!(run["summary"]["avg_score"], 1.0);
    assert_eq!(
        run_completed_at(&server, &seeded.run_id).await,
        completed_at_before
    );
}

#[tokio::test]
async fn test_bulk_update_eval_run_scores_applies_shared_metadata() {
    let server = TestServer::in_memory().await;
    let seeded = seed_completed_run(&server).await;

    let response = server
        .patch(
            &format!("/v1/evals/{}/runs/{}/scores", seeded.eval_id, seeded.run_id),
            json!({
                "results": [
                    {
                        "result_id": seeded.result_ids[0],
                        "scores": [{"pass": true, "value": 1.0, "reason": "passed externally"}]
                    },
                    {
                        "result_id": seeded.result_ids[1],
                        "scores": [{"pass": false, "value": 0.4, "reason": "manual review failed"}],
                        "status": "errored"
                    }
                ],
                "metadata": {
                    "scorer": "manual-review",
                    "reviewer": "qa@example.com"
                }
            }),
        )
        .await
        .assert_status(StatusCode::OK);

    let body = response.json_value();
    let results = body["data"].as_array().expect("bulk response data");
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["metadata"]["scorer"], "manual-review");
    assert_eq!(results[1]["metadata"]["reviewer"], "qa@example.com");

    let run = server
        .get(&format!(
            "/v1/evals/{}/runs/{}",
            seeded.eval_id, seeded.run_id
        ))
        .await
        .assert_status(StatusCode::OK)
        .json_value();

    assert_eq!(run["summary"]["passed"], 1);
    assert_eq!(run["summary"]["failed"], 0);
    assert_eq!(run["summary"]["errored"], 1);
    assert_eq!(run["summary"]["pass_rate"], 0.5);
    assert_eq!(run["summary"]["avg_score"], 0.5);
    assert_eq!(run["summary"]["avg_latency_ms"], 25);
    assert_eq!(run["summary"]["total_input_tokens"], 10);
    assert_eq!(run["summary"]["total_output_tokens"], 20);
}

#[tokio::test]
async fn test_update_eval_result_scores_rejects_non_completed_run() {
    let server = TestServer::in_memory().await;
    let seeded = seed_completed_run(&server).await;

    let run_row = server
        .db
        .get_eval_run_by_public_id(TEST_ORG_ID, &seeded.run_id)
        .await
        .expect("load run")
        .expect("run exists");
    server
        .db
        .update_eval_run_status(run_row.id, "running", run_row.summary)
        .await
        .expect("set running");

    let response = server
        .patch(
            &format!(
                "/v1/evals/{}/runs/{}/results/{}/scores",
                seeded.eval_id, seeded.run_id, seeded.result_ids[0]
            ),
            json!({
                "scores": [{"pass": true, "value": 1.0, "reason": "should fail"}]
            }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response
            .text()
            .contains("can only write scores for completed eval runs")
    );
}

#[tokio::test]
async fn test_update_eval_result_scores_rejects_non_object_metadata() {
    let server = TestServer::in_memory().await;
    let seeded = seed_completed_run(&server).await;

    let response = server
        .patch(
            &format!(
                "/v1/evals/{}/runs/{}/results/{}/scores",
                seeded.eval_id, seeded.run_id, seeded.result_ids[0]
            ),
            json!({
                "scores": [{"pass": true, "value": 1.0, "reason": "should fail"}],
                "metadata": ["not", "an", "object"]
            }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(response.text().contains("metadata must be a JSON object"));
}

#[tokio::test]
async fn test_update_eval_result_scores_rejects_out_of_range_values() {
    let server = TestServer::in_memory().await;
    let seeded = seed_completed_run(&server).await;

    let response = server
        .patch(
            &format!(
                "/v1/evals/{}/runs/{}/results/{}/scores",
                seeded.eval_id, seeded.run_id, seeded.result_ids[0]
            ),
            json!({
                "scores": [{"pass": true, "value": 1.1, "reason": "invalid"}]
            }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response
            .text()
            .contains("score at index 0 must have a value between 0.0 and 1.0")
    );
}

#[tokio::test]
async fn test_update_eval_result_scores_rejects_score_count_mismatch() {
    let server = TestServer::in_memory().await;
    let seeded = seed_completed_run(&server).await;

    let response = server
        .patch(
            &format!(
                "/v1/evals/{}/runs/{}/results/{}/scores",
                seeded.eval_id, seeded.run_id, seeded.result_ids[0]
            ),
            json!({
                "scores": [
                    {"pass": true, "value": 1.0, "reason": "first"},
                    {"pass": true, "value": 1.0, "reason": "second"}
                ]
            }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response
            .text()
            .contains("score count (2) must match configured scorer count (1)")
    );
}

#[tokio::test]
async fn test_bulk_update_run_scores_rejects_duplicate_result_ids() {
    let server = TestServer::in_memory().await;
    let seeded = seed_completed_run(&server).await;

    let response = server
        .patch(
            &format!("/v1/evals/{}/runs/{}/scores", seeded.eval_id, seeded.run_id),
            json!({
                "results": [
                    {
                        "result_id": seeded.result_ids[0],
                        "scores": [{"pass": true, "value": 1.0, "reason": "first"}]
                    },
                    {
                        "result_id": seeded.result_ids[0],
                        "scores": [{"pass": true, "value": 1.0, "reason": "duplicate"}]
                    }
                ]
            }),
        )
        .await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        response
            .text()
            .contains("duplicate result_id in bulk request")
    );
}

struct SeededRun {
    eval_id: String,
    run_id: String,
    result_ids: Vec<String>,
}

async fn seed_completed_run(server: &TestServer) -> SeededRun {
    let eval: Eval = server
        .post(
            "/v1/evals",
            json!({
                "name": "deferred scoring eval",
                "description": "integration test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let case_one: EvalCase = server
        .post(
            &format!("/v1/evals/{}/cases", eval.public_id),
            json!({
                "name": "case-one",
                "conversation": [{"content": "hello"}],
                "scorers": [{"type": "contains", "text": "ok", "weight": 1.0}]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let case_two: EvalCase = server
        .post(
            &format!("/v1/evals/{}/cases", eval.public_id),
            json!({
                "name": "case-two",
                "conversation": [{"content": "hello"}],
                "scorers": [{"type": "contains", "text": "ok", "weight": 1.0}]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let eval_row = server
        .db
        .get_eval_by_public_id(TEST_ORG_ID, &eval.public_id.to_string())
        .await
        .expect("load eval")
        .expect("eval exists");
    let case_one_row = server
        .db
        .get_eval_case_by_public_id(eval_row.id, &case_one.public_id.to_string())
        .await
        .expect("load case one")
        .expect("case one exists");
    let case_two_row = server
        .db
        .get_eval_case_by_public_id(eval_row.id, &case_two.public_id.to_string())
        .await
        .expect("load case two")
        .expect("case two exists");

    let run_public_id = EvalRunId::from_uuid(uuid::Uuid::now_v7()).to_string();
    let run_row = server
        .db
        .create_eval_run(
            TEST_ORG_ID,
            CreateEvalRunRow {
                public_id: run_public_id.clone(),
                eval_id: eval_row.id,
                target: None,
                model_override: None,
                filter_tags: None,
                triggered_by: "test".to_string(),
            },
        )
        .await
        .expect("create run");

    let target = json!({
        "type": "session",
        "harness_id": TEST_HARNESS_ID
    });
    let result_one_id = EvalResultId::from_uuid(uuid::Uuid::now_v7()).to_string();
    let result_two_id = EvalResultId::from_uuid(uuid::Uuid::now_v7()).to_string();
    let result_one = server
        .db
        .create_eval_case_result(CreateEvalCaseResultRow {
            public_id: result_one_id.clone(),
            eval_run_id: run_row.id,
            eval_case_id: case_one_row.id,
            target: Some(target.clone()),
            target_snapshot: Some(target.clone()),
            artifacts: None,
        })
        .await
        .expect("create result one");
    let result_two = server
        .db
        .create_eval_case_result(CreateEvalCaseResultRow {
            public_id: result_two_id.clone(),
            eval_run_id: run_row.id,
            eval_case_id: case_two_row.id,
            target: Some(target),
            target_snapshot: Some(json!({
                "type": "session",
                "harness_id": TEST_HARNESS_ID
            })),
            artifacts: None,
        })
        .await
        .expect("create result two");

    server
        .db
        .update_eval_case_result(
            result_one.id,
            UpdateEvalCaseResultRow {
                status: Some("failed".to_string()),
                scores: Some(json!([{ "pass": false, "value": 0.0, "reason": "initial fail" }])),
                turns: Some(2),
                latency_ms: Some(50),
                input_tokens: Some(10),
                output_tokens: Some(20),
                ..Default::default()
            },
        )
        .await
        .expect("seed result one");
    server
        .db
        .update_eval_case_result(
            result_two.id,
            UpdateEvalCaseResultRow {
                status: Some("passed".to_string()),
                scores: Some(json!([{ "pass": true, "value": 1.0, "reason": "initial pass" }])),
                turns: Some(4),
                latency_ms: Some(150),
                input_tokens: Some(30),
                output_tokens: Some(40),
                ..Default::default()
            },
        )
        .await
        .expect("seed result two");
    server
        .db
        .update_eval_run_status(
            run_row.id,
            "completed",
            Some(json!({
                "total": 2,
                "passed": 1,
                "failed": 1,
                "errored": 0,
                "pass_rate": 0.5,
                "avg_score": 0.5,
                "avg_turns": 3.0,
                "avg_latency_ms": 100,
                "total_input_tokens": 40,
                "total_output_tokens": 60
            })),
        )
        .await
        .expect("complete run");

    SeededRun {
        eval_id: eval.public_id.to_string(),
        run_id: run_public_id,
        result_ids: vec![result_one_id, result_two_id],
    }
}

async fn run_completed_at(server: &TestServer, run_id: &str) -> String {
    server
        .db
        .get_eval_run_by_public_id(TEST_ORG_ID, run_id)
        .await
        .expect("load run")
        .expect("run exists")
        .completed_at
        .expect("completed_at set")
        .to_rfc3339()
}
