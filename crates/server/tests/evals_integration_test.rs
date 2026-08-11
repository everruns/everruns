//! Eval write-back integration tests.
//!
//! Focused on deferred external scoring: update completed results, persist scorer
//! metadata, and recompute run summaries without re-running sessions.

mod test_harness;

use axum::http::StatusCode;
use serde_json::json;
use test_harness::TestServer;

use everruns_core::eval::{Eval, EvalCase, EvalDatasetStatus};
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

#[tokio::test]
async fn test_create_run_without_any_target_does_not_create_orphaned_run() {
    let server = TestServer::in_memory().await;
    let eval: Eval = server
        .post(
            "/v1/evals",
            json!({
                "name": "targetless eval",
                "description": "integration test"
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let _case: EvalCase = server
        .post(
            &format!("/v1/evals/{}/cases", eval.public_id),
            json!({
                "name": "targetless case",
                "conversation": [{"content": "hello"}],
                "scorers": [{"type": "contains", "text": "ok", "weight": 1.0}]
            }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let response = server
        .post(&format!("/v1/evals/{}/runs", eval.public_id), json!({}))
        .await;

    assert_ne!(response.status(), StatusCode::CREATED);
    let _ = response.text();

    let eval_row = server
        .db
        .get_eval_by_public_id(TEST_ORG_ID, &eval.public_id.to_string())
        .await
        .expect("load eval")
        .expect("eval exists");
    let runs = server
        .db
        .list_eval_runs(eval_row.id)
        .await
        .expect("list runs");
    assert_eq!(runs.len(), 0);
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

// ============================================================================
// Async dataset export (knowledge/evaluation/dataset-export.md, Phase 2)
// ============================================================================

use everruns_core::Caller;
use everruns_core::typed_id::{EvalDatasetId, SessionId};
use everruns_server::domains::evals::EvalService;
use everruns_server::domains::evals::dataset::ExportEvalRunDatasetRequest;
use everruns_server::storage::models::{CreateEventRow, CreatePrincipalRow, CreateSessionRow};

/// A secret embedded in the seeded assistant message; the export must scrub it.
const SEEDED_SECRET: &str = "sk-abcdef0123456789ABCDEF";

/// Seed a completed run whose single passed case has a real session with
/// `input.message` + `output.message.completed` events, so the async export
/// reconstructs the model-view trajectory. Returns (eval_id, run_id).
async fn seed_run_with_session_events(server: &TestServer) -> (String, String) {
    use uuid::Uuid;

    let eval: Eval = server
        .post(
            "/v1/evals",
            json!({ "name": "dataset export eval", "description": "e2e" }),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let case: EvalCase = server
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

    let eval_row = server
        .db
        .get_eval_by_public_id(TEST_ORG_ID, &eval.public_id.to_string())
        .await
        .expect("load eval")
        .expect("eval exists");
    let case_row = server
        .db
        .get_eval_case_by_public_id(eval_row.id, &case.public_id.to_string())
        .await
        .expect("load case")
        .expect("case exists");

    // A principal is required to own the session (FK on Postgres).
    let principal = server
        .db
        .create_principal(CreatePrincipalRow {
            id: everruns_core::PrincipalId::new(),
            org_id: TEST_ORG_ID,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "dataset_export_test" }),
        })
        .await
        .expect("create principal");

    let session = server
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: principal.id,
            resolved_owner_user_id: None,
            title: Some("Eval: case-one".to_string()),
            locale: None,
            tags: vec!["eval".to_string()],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            system_prompt: None,
            initial_files: json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .expect("create session");
    let session_id: SessionId = session.id;

    // Seed the conversation the model saw: a user turn and an assistant reply
    // that also contains a credential the export must scrub. Build real event
    // payloads so `event_to_message` reconstruction matches production.
    use everruns_core::events::{InputMessageData, OutputMessageCompletedData};
    use everruns_core::message::Message;
    server
        .db
        .create_event(CreateEventRow {
            session_id,
            event_type: "input.message".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: serde_json::to_value(InputMessageData::new(Message::user("hello")))
                .expect("serialize input event"),
            metadata: None,
            tags: None,
        })
        .await
        .expect("seed input event");
    server
        .db
        .create_event(CreateEventRow {
            session_id,
            event_type: "output.message.completed".to_string(),
            ts: chrono::Utc::now(),
            context: json!({}),
            data: serde_json::to_value(OutputMessageCompletedData::new(Message::assistant(
                format!("ok, token is {SEEDED_SECRET}"),
            )))
            .expect("serialize output event"),
            metadata: None,
            tags: None,
        })
        .await
        .expect("seed output event");

    let run_public_id = EvalRunId::from_uuid(Uuid::now_v7()).to_string();
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

    let target = json!({ "type": "session", "harness_id": TEST_HARNESS_ID });
    let result = server
        .db
        .create_eval_case_result(CreateEvalCaseResultRow {
            public_id: EvalResultId::from_uuid(Uuid::now_v7()).to_string(),
            eval_run_id: run_row.id,
            eval_case_id: case_row.id,
            target: Some(target.clone()),
            target_snapshot: Some(target),
            artifacts: None,
        })
        .await
        .expect("create result");
    server
        .db
        .update_eval_case_result(
            result.id,
            UpdateEvalCaseResultRow {
                status: Some("passed".to_string()),
                session_id: Some(session_id.uuid()),
                scores: Some(json!([{ "pass": true, "value": 1.0, "reason": "has ok" }])),
                turns: Some(1),
                latency_ms: Some(100),
                input_tokens: Some(10),
                output_tokens: Some(20),
                ..Default::default()
            },
        )
        .await
        .expect("seed result");
    server
        .db
        .update_eval_run_status(
            run_row.id,
            "completed",
            Some(json!({
                "total": 1, "passed": 1, "failed": 0, "errored": 0,
                "pass_rate": 1.0, "avg_score": 1.0, "avg_turns": 1.0,
                "avg_latency_ms": 100, "total_input_tokens": 10, "total_output_tokens": 20
            })),
        )
        .await
        .expect("complete run");

    (eval.public_id.to_string(), run_public_id)
}

/// Poll the dataset handle until it reaches a terminal state or the budget runs
/// out. The export runs in a background task, so this mirrors a real client.
async fn await_dataset(
    service: &EvalService,
    caller: &Caller,
    eval_id: &str,
    run_id: &str,
    dataset_id: &str,
) -> everruns_core::eval::EvalRunDataset {
    for _ in 0..100 {
        let ds = service
            .get_dataset(caller, eval_id, run_id, dataset_id)
            .await
            .expect("get_dataset")
            .expect("dataset exists");
        if matches!(
            ds.status,
            EvalDatasetStatus::Completed | EvalDatasetStatus::Failed
        ) {
            return ds;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("dataset export did not finish in time");
}

#[tokio::test]
async fn test_dataset_export_async_handle_produces_scrubbed_ndjson() {
    let server = TestServer::in_memory().await;
    let (eval_id, run_id) = seed_run_with_session_events(&server).await;
    let service = EvalService::new(server.db.clone());
    let caller = Caller::internal(TEST_ORG_ID);

    // POST enqueues: returns a pending handle with no body attached.
    let handle = service
        .create_dataset_export(
            &caller,
            &eval_id,
            &run_id,
            ExportEvalRunDatasetRequest::default(),
        )
        .await
        .expect("enqueue export")
        .expect("run exists");
    assert!(matches!(
        handle.status,
        EvalDatasetStatus::Pending | EvalDatasetStatus::Running | EvalDatasetStatus::Completed
    ));
    assert!(handle.body.is_none(), "enqueue must not attach body");

    // GET polls to completion and returns the NDJSON body.
    let done = await_dataset(
        &service,
        &caller,
        &eval_id,
        &run_id,
        &handle.public_id.to_string(),
    )
    .await;
    assert_eq!(done.status, EvalDatasetStatus::Completed);
    assert_eq!(done.record_count, Some(1));
    let body = done.body.expect("completed dataset has body");

    // One NDJSON record for the single passing case.
    let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1, "one record per surviving case");
    let record: serde_json::Value = serde_json::from_str(lines[0]).expect("valid NDJSON record");

    // Reward joined from the case result.
    assert_eq!(record["reward"]["pass"], json!(true));
    assert_eq!(record["reward"]["score"], json!(1.0));
    // Model-view trajectory reconstructed from the seeded session events.
    let messages = record["messages"].as_array().expect("messages array");
    assert!(messages.len() >= 2, "user + assistant messages present");
    // Always-on secret scrubbing removed the credential from the trajectory.
    assert!(
        !body.contains(SEEDED_SECRET),
        "seeded secret must be scrubbed from the exported dataset"
    );
    assert!(body.contains("[REDACTED]"), "scrubbed placeholder present");

    // Repeating an identical export reuses the durable result instead of
    // enqueueing duplicate reconstruction work or storing another body.
    let repeated = service
        .create_dataset_export(
            &caller,
            &eval_id,
            &run_id,
            ExportEvalRunDatasetRequest::default(),
        )
        .await
        .expect("repeat export")
        .expect("run exists");
    assert_eq!(repeated.public_id, handle.public_id);
}

#[tokio::test]
async fn test_dataset_export_cross_org_returns_not_found() {
    let server = TestServer::in_memory().await;
    let (eval_id, run_id) = seed_run_with_session_events(&server).await;
    let service = EvalService::new(server.db.clone());

    // Owner of the run enqueues an export.
    let owner = Caller::internal(TEST_ORG_ID);
    let handle = service
        .create_dataset_export(
            &owner,
            &eval_id,
            &run_id,
            ExportEvalRunDatasetRequest::default(),
        )
        .await
        .expect("enqueue export")
        .expect("run exists");

    // "Not reachable" means the caller gets a 404: either the run/eval does not
    // resolve for their org (`get_run` returns a not-found error), or the query
    // resolves to `Ok(None)`. Both map to 404 at the command layer.
    fn assert_not_reachable(
        result: anyhow::Result<Option<everruns_core::eval::EvalRunDataset>>,
        what: &str,
    ) {
        match result {
            Ok(None) => {}
            Ok(Some(_)) => panic!("{what} must not be reachable across org boundary"),
            Err(e) => assert!(
                e.downcast_ref::<everruns_server::errors::ResourceNotFoundError>()
                    .is_some(),
                "{what} must fail with not-found, got: {e}"
            ),
        }
    }

    // A caller from a different org cannot enqueue against this run: the run is
    // resolved through `get_run` with the caller's org, so it is not found.
    let other_org = Caller::internal(TEST_ORG_ID + 424242);
    assert_not_reachable(
        service
            .create_dataset_export(
                &other_org,
                &eval_id,
                &run_id,
                ExportEvalRunDatasetRequest::default(),
            )
            .await,
        "cross-org export enqueue",
    );

    // Nor can a foreign org fetch the owner's dataset handle by id.
    assert_not_reachable(
        service
            .get_dataset(&other_org, &eval_id, &run_id, &handle.public_id.to_string())
            .await,
        "cross-org dataset fetch",
    );

    // A completely unknown dataset id for the owner is also not-found.
    let missing = service
        .get_dataset(
            &owner,
            &eval_id,
            &run_id,
            &EvalDatasetId::from_uuid(uuid::Uuid::now_v7()).to_string(),
        )
        .await
        .expect("missing dataset get");
    assert!(missing.is_none(), "unknown dataset id is not-found");
}

// ============================================================================
// ATIF export/import (knowledge/evaluation/atif-adoption.md)
// ============================================================================

#[tokio::test]
async fn test_dataset_export_atif_format_produces_atif_trajectories() {
    use everruns_server::domains::evals::dataset::DatasetFormat;

    let server = TestServer::in_memory().await;
    let (eval_id, run_id) = seed_run_with_session_events(&server).await;
    let service = EvalService::new(server.db.clone());
    let caller = Caller::internal(TEST_ORG_ID);

    let handle = service
        .create_dataset_export(
            &caller,
            &eval_id,
            &run_id,
            ExportEvalRunDatasetRequest {
                format: DatasetFormat::Atif,
                ..Default::default()
            },
        )
        .await
        .expect("enqueue export")
        .expect("run exists");
    let done = await_dataset(
        &service,
        &caller,
        &eval_id,
        &run_id,
        &handle.public_id.to_string(),
    )
    .await;
    assert_eq!(done.status, EvalDatasetStatus::Completed);
    let body = done.body.expect("completed dataset has body");

    // One complete ATIF trajectory per line.
    let lines: Vec<&str> = body.lines().filter(|l| !l.is_empty()).collect();
    assert_eq!(lines.len(), 1);
    let record: serde_json::Value = serde_json::from_str(lines[0]).expect("valid NDJSON record");
    assert_eq!(record["schema_version"], json!("ATIF-v1.7"));
    assert_eq!(record["agent"]["name"], json!("everruns"));

    // Steps folded from the seeded events: user message + agent message.
    let steps = record["steps"].as_array().expect("steps array");
    assert_eq!(steps.len(), 2);
    assert_eq!(steps[0]["source"], json!("user"));
    assert_eq!(steps[0]["message"], json!("hello"));
    assert_eq!(steps[0]["step_id"], json!(1));
    assert_eq!(steps[1]["source"], json!("agent"));
    assert_eq!(steps[1]["step_id"], json!(2));

    // Reward and case identity live at the ATIF extension point.
    assert_eq!(record["extra"]["reward"]["pass"], json!(true));
    assert_eq!(record["extra"]["reward"]["score"], json!(1.0));
    assert!(record["extra"]["source_key"].is_string());
    assert_eq!(record["extra"]["case_name"], json!("case-one"));

    // Always-on secret scrubbing applies on the ATIF path too.
    assert!(
        !body.contains(SEEDED_SECRET),
        "seeded secret must be scrubbed from the ATIF export"
    );
    assert!(body.contains("[REDACTED]"));
}

/// Seed a completed run whose case session has `n` tool-call/result iterations
/// (each result a distinctive `BULKY_RESULT_{i}` string). Returns
/// `(eval_id, run_id, session_id)`.
async fn seed_run_with_tool_iterations(
    server: &TestServer,
    n: usize,
) -> (String, String, SessionId) {
    use everruns_core::events::{InputMessageData, OutputMessageCompletedData, ToolCompletedData};
    use everruns_core::message::{ContentPart, Message};
    use everruns_core::tool_types::ToolCall;
    use uuid::Uuid;

    let eval: Eval = server
        .post("/v1/evals", json!({ "name": "modelview eval" }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let case: EvalCase = server
        .post(
            &format!("/v1/evals/{}/cases", eval.public_id),
            json!({
                "name": "case-mv",
                "conversation": [{"content": "start"}],
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
    let case_row = server
        .db
        .get_eval_case_by_public_id(eval_row.id, &case.public_id.to_string())
        .await
        .expect("load case")
        .expect("case exists");

    let principal = server
        .db
        .create_principal(CreatePrincipalRow {
            id: everruns_core::PrincipalId::new(),
            org_id: TEST_ORG_ID,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "modelview_test" }),
        })
        .await
        .expect("create principal");
    let session = server
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: principal.id,
            resolved_owner_user_id: None,
            title: Some("Eval: case-mv".to_string()),
            locale: None,
            tags: vec!["eval".to_string()],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            system_prompt: None,
            initial_files: json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .expect("create session");
    let session_id: SessionId = session.id;

    // Seed the session's event log; `retriever.load` reconstructs the messages
    // from these, and cost-control compaction then masks the older tool results.
    let mut seeded: Vec<serde_json::Value> =
        vec![serde_json::to_value(InputMessageData::new(Message::user("start"))).unwrap()];
    let mut event_types: Vec<&str> = vec!["input.message"];
    for i in 0..n {
        let call = ToolCall {
            id: format!("call_{i}"),
            name: "fetch".to_string(),
            arguments: json!({ "n": i }),
        };
        seeded.push(
            serde_json::to_value(OutputMessageCompletedData::new(
                Message::assistant_with_tools(format!("iter {i}"), vec![call]),
            ))
            .unwrap(),
        );
        event_types.push("output.message.completed");
        seeded.push(
            serde_json::to_value(ToolCompletedData::success(
                format!("call_{i}"),
                "fetch".to_string(),
                vec![ContentPart::text(format!("BULKY_RESULT_{i}"))],
                Some(1),
            ))
            .unwrap(),
        );
        event_types.push("tool.completed");
    }
    for (event_type, data) in event_types.into_iter().zip(seeded) {
        server
            .db
            .create_event(CreateEventRow {
                session_id,
                event_type: event_type.to_string(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data,
                metadata: None,
                tags: None,
            })
            .await
            .expect("seed event");
    }

    let run_public_id = EvalRunId::from_uuid(Uuid::now_v7()).to_string();
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
    let target = json!({ "type": "session", "harness_id": TEST_HARNESS_ID });
    let result = server
        .db
        .create_eval_case_result(CreateEvalCaseResultRow {
            public_id: EvalResultId::from_uuid(Uuid::now_v7()).to_string(),
            eval_run_id: run_row.id,
            eval_case_id: case_row.id,
            target: Some(target.clone()),
            target_snapshot: Some(target),
            artifacts: None,
        })
        .await
        .expect("create result");
    server
        .db
        .update_eval_case_result(
            result.id,
            UpdateEvalCaseResultRow {
                status: Some("passed".to_string()),
                session_id: Some(session_id.uuid()),
                scores: Some(json!([{ "pass": true, "value": 1.0, "reason": "ok" }])),
                turns: Some(1),
                latency_ms: Some(100),
                input_tokens: Some(10),
                output_tokens: Some(20),
                ..Default::default()
            },
        )
        .await
        .expect("seed result");
    server
        .db
        .update_eval_run_status(run_row.id, "completed", Some(json!({ "total": 1 })))
        .await
        .expect("complete run");

    (eval.public_id.to_string(), run_public_id, session_id)
}

/// The ATIF **dataset** export folds the compaction model view, so tool results
/// the model no longer saw (masked by cost-control compaction) are absent from
/// the training rows — while the raw session export (a debug surface) still
/// carries them. This contrast is the whole point of model-view faithfulness.
#[tokio::test]
async fn test_dataset_export_atif_uses_model_view_not_raw_log() {
    use everruns_server::domains::evals::dataset::DatasetFormat;

    let server = TestServer::in_memory().await;
    // Five tool iterations: default cost-control masking keeps the two most
    // recent tool results and masks the three oldest.
    let (eval_id, run_id, session_id) = seed_run_with_tool_iterations(&server, 5).await;
    let service = EvalService::new(server.db.clone());
    let caller = Caller::internal(TEST_ORG_ID);

    let handle = service
        .create_dataset_export(
            &caller,
            &eval_id,
            &run_id,
            ExportEvalRunDatasetRequest {
                format: DatasetFormat::Atif,
                ..Default::default()
            },
        )
        .await
        .expect("enqueue export")
        .expect("run exists");
    let done = await_dataset(
        &service,
        &caller,
        &eval_id,
        &run_id,
        &handle.public_id.to_string(),
    )
    .await;
    assert_eq!(done.status, EvalDatasetStatus::Completed);
    let dataset_body = done.body.expect("completed dataset has body");

    // Model view: the three oldest tool results are masked out; the two most
    // recent survive.
    for i in 0..3 {
        assert!(
            !dataset_body.contains(&format!("BULKY_RESULT_{i}")),
            "masked tool result {i} must be absent from the ATIF dataset row",
        );
    }
    for i in 3..5 {
        assert!(
            dataset_body.contains(&format!("BULKY_RESULT_{i}")),
            "recent tool result {i} must be present in the ATIF dataset row",
        );
    }

    // Contrast: the raw session ATIF export (event-log fold) still carries every
    // result — it is a debug/backup surface, not training data.
    let raw = server
        .get(&format!("/v1/sessions/{}/export?format=atif", session_id))
        .await
        .assert_status(StatusCode::OK);
    let raw_body = raw.text();
    for i in 0..5 {
        assert!(
            raw_body.contains(&format!("BULKY_RESULT_{i}")),
            "raw session export must carry every tool result (missing {i})",
        );
    }
}

#[tokio::test]
async fn test_atif_import_creates_and_updates_cases_idempotently() {
    let server = TestServer::in_memory().await;
    let eval: Eval = server
        .post("/v1/evals", json!({ "name": "atif import eval" }))
        .await
        .assert_status(StatusCode::CREATED)
        .json();

    let trajectory = json!({
        "schema_version": "ATIF-v1.7",
        "session_id": "session_ext_1",
        "agent": {"name": "yolop", "version": "0.3.0"},
        "steps": [
            {"step_id": 1, "source": "user", "message": "What is 2+2?"},
            {"step_id": 2, "source": "agent", "message": "4"}
        ],
        "extra": {"case_name": "arith-1", "reward": {"pass": true, "score": 1.0}}
    });
    // NDJSON body: one trajectory per line.
    let ndjson = format!("{}\n", serde_json::to_string(&trajectory).unwrap());

    let response = server
        .request_raw(
            axum::http::Method::POST,
            &format!("/v1/evals/{}/atif_import", eval.public_id),
            vec![("content-type", "application/x-ndjson")],
            ndjson.clone().into_bytes(),
        )
        .await
        .assert_status(StatusCode::OK);
    let report = response.json_value();
    assert_eq!(report["created"], json!(1));
    assert_eq!(report["updated"], json!(0));

    // Re-import converges (upsert by case name) instead of duplicating.
    let response = server
        .request_raw(
            axum::http::Method::POST,
            &format!("/v1/evals/{}/atif_import", eval.public_id),
            vec![("content-type", "application/x-ndjson")],
            ndjson.into_bytes(),
        )
        .await
        .assert_status(StatusCode::OK);
    let report = response.json_value();
    assert_eq!(report["created"], json!(0));
    assert_eq!(report["updated"], json!(1));

    let cases = server
        .get(&format!("/v1/evals/{}/cases", eval.public_id))
        .await
        .assert_status(StatusCode::OK)
        .json_value();
    let items = cases["data"].as_array().expect("cases list");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["name"], json!("arith-1"));
    assert_eq!(
        items[0]["conversation"][0]["content"],
        json!("What is 2+2?")
    );
    let description = items[0]["description"].as_str().unwrap();
    assert!(description.contains("Reference final agent message"));

    // Malformed payloads are rejected with 400.
    server
        .request_raw(
            axum::http::Method::POST,
            &format!("/v1/evals/{}/atif_import", eval.public_id),
            vec![("content-type", "application/json")],
            b"{\"schema_version\": \"OTHER-v1\", \"steps\": []}".to_vec(),
        )
        .await
        .assert_status(StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_session_export_atif_format() {
    let server = TestServer::in_memory().await;
    // Reuse the dataset-export seed: it creates a session with input/output
    // message events (plus a credential the export must scrub).
    let (eval_id, run_id) = seed_run_with_session_events(&server).await;
    let service = EvalService::new(server.db.clone());
    let caller = Caller::internal(TEST_ORG_ID);
    let run = service
        .get_run(&caller, &eval_id, &run_id)
        .await
        .expect("get run")
        .expect("run exists");
    let session_id = run.results[0].session_id.expect("case session");

    let response = server
        .get(&format!("/v1/sessions/{}/export?format=atif", session_id))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/json"
    );
    let disposition = response
        .headers()
        .get("content-disposition")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    assert!(disposition.contains(&format!("{}.atif.json", session_id)));

    // Text-only session: the lossiness header must be absent, not "0".
    assert!(response.headers().get("x-atif-images-omitted").is_none());

    let body = response.text();
    let trajectory: serde_json::Value = serde_json::from_str(&body).expect("single JSON document");
    assert_eq!(trajectory["schema_version"], json!("ATIF-v1.7"));
    assert_eq!(trajectory["session_id"], json!(session_id.to_string()));
    let steps = trajectory["steps"].as_array().expect("steps");
    assert_eq!(steps[0]["source"], json!("user"));
    assert!(!body.contains(SEEDED_SECRET), "secret must be scrubbed");

    // Default format is unchanged: JSONL with one message per line.
    let response = server
        .get(&format!("/v1/sessions/{}/export", session_id))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        response.headers().get("content-type").unwrap(),
        "application/x-ndjson"
    );
}

/// Seed a bare session (principal + session row) owned by the test org and
/// insert the given raw `(event_type, data)` events, oldest first.
async fn seed_session_with_raw_events(
    server: &TestServer,
    events: Vec<(&str, serde_json::Value)>,
) -> SessionId {
    use uuid::Uuid;

    let principal = server
        .db
        .create_principal(CreatePrincipalRow {
            id: everruns_core::PrincipalId::new(),
            org_id: TEST_ORG_ID,
            kind: "system".to_string(),
            subject_id: Some(Uuid::now_v7()),
            parent_principal_id: None,
            resolved_user_id: None,
            metadata: json!({ "source": "atif_export_test" }),
        })
        .await
        .expect("create principal");

    let session = server
        .db
        .create_session(CreateSessionRow {
            source: everruns_platform::SessionSource::Api,
            org_id: TEST_ORG_ID,
            app_id: None,
            harness_id: None,
            agent_id: None,
            agent_version_id: None,
            agent_config_hash: None,
            agent_identity_id: None,
            owner_principal_id: principal.id,
            resolved_owner_user_id: None,
            title: Some("ATIF export test".to_string()),
            locale: None,
            tags: vec![],
            model_id: None,
            capabilities: json!([]),
            tools: json!([]),
            mcp_servers: json!({}),
            system_prompt: None,
            initial_files: json!([]),
            hints: None,
            network_access: None,
            max_iterations: None,
            parallel_tool_calls: None,
            blueprint_id: None,
            blueprint_config: None,
            parent_session_id: None,
            budget_root_session_id: None,
            workspace_id: None,
        })
        .await
        .expect("create session");

    for (event_type, data) in events {
        server
            .db
            .create_event(CreateEventRow {
                session_id: session.id,
                event_type: event_type.to_string(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data,
                metadata: None,
                tags: None,
            })
            .await
            .expect("seed event");
    }

    session.id
}

#[tokio::test]
async fn test_session_export_atif_image_content_multimodal() {
    use everruns_core::events::InputMessageData;
    use everruns_core::message::{ContentPart, ImageContentPart, ImageFileContentPart, Message};
    use everruns_core::typed_id::ImageId;

    let server = TestServer::in_memory().await;
    let image_id = ImageId::new();
    let mut message = Message::user("look at this");
    message
        .content
        .push(ContentPart::Image(ImageContentPart::from_url(
            "https://example.com/cat.png",
        )));
    message
        .content
        .push(ContentPart::ImageFile(ImageFileContentPart::with_filename(
            image_id, "cat.png",
        )));
    let session_id = seed_session_with_raw_events(
        &server,
        vec![(
            "input.message",
            serde_json::to_value(InputMessageData::new(message)).expect("serialize input event"),
        )],
    )
    .await;

    let response = server
        .get(&format!("/v1/sessions/{}/export?format=atif", session_id))
        .await
        .assert_status(StatusCode::OK);
    // Both images are materialized (URL + file reference), so nothing is
    // omitted and the lossiness header is absent.
    assert!(response.headers().get("x-atif-images-omitted").is_none());
    let trajectory = response.json_value();
    assert!(trajectory["extra"].get("images_omitted").is_none());
    let step = &trajectory["steps"][0];
    // `message` is now a ContentPart array with real image sources, not a
    // flattened `[image]` marker string.
    let parts = step["message"]
        .as_array()
        .expect("multimodal message array");
    assert_eq!(parts[0], json!({"type": "text", "text": "look at this"}));
    assert_eq!(
        parts[1],
        json!({"type": "image", "source": {"path": "https://example.com/cat.png"}})
    );
    assert_eq!(
        parts[2],
        json!({"type": "image", "source": {"path": format!("/v1/images/{image_id}")}})
    );
    assert!(step["extra"].get("omitted_images").is_none());

    // The default JSONL export never carries the ATIF lossiness header.
    let response = server
        .get(&format!("/v1/sessions/{}/export", session_id))
        .await
        .assert_status(StatusCode::OK);
    assert!(response.headers().get("x-atif-images-omitted").is_none());
}

#[tokio::test]
async fn test_session_export_atif_unmaterializable_image_sets_header() {
    use everruns_core::events::InputMessageData;
    use everruns_core::message::{ContentPart, ImageContentPart, Message};

    let server = TestServer::in_memory().await;
    let mut message = Message::user("look at this");
    // An inline image with neither URL nor base64 cannot be materialized, so it
    // stays a marker and is counted as omitted.
    message.content.push(ContentPart::Image(ImageContentPart {
        url: None,
        base64: None,
        media_type: Some("image/png".to_string()),
    }));
    let session_id = seed_session_with_raw_events(
        &server,
        vec![(
            "input.message",
            serde_json::to_value(InputMessageData::new(message)).expect("serialize input event"),
        )],
    )
    .await;

    let response = server
        .get(&format!("/v1/sessions/{}/export?format=atif", session_id))
        .await
        .assert_status(StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get("x-atif-images-omitted")
            .expect("lossiness header"),
        "1"
    );
    let trajectory = response.json_value();
    assert_eq!(trajectory["extra"]["images_omitted"], json!(1));
    let step = &trajectory["steps"][0];
    assert!(step["message"].as_str().unwrap().contains("[image]"));
    assert_eq!(
        step["extra"]["omitted_images"],
        json!([{"media_type": "image/png"}])
    );
}

#[tokio::test]
async fn test_session_export_atif_over_cap_returns_413() {
    use everruns_core::events::InputMessageData;
    use everruns_core::message::Message;

    // Tiny injected cap; production uses `ATIF_EXPORT_MAX_BYTES` (50 MiB).
    let server = TestServer::in_memory_with_atif_export_cap(64).await;
    let session_id = seed_session_with_raw_events(
        &server,
        vec![(
            "input.message",
            serde_json::to_value(InputMessageData::new(Message::user("hello world")))
                .expect("serialize input event"),
        )],
    )
    .await;

    let response = server
        .get(&format!("/v1/sessions/{}/export?format=atif", session_id))
        .await
        .assert_status(StatusCode::PAYLOAD_TOO_LARGE);
    let body = response.json_value();
    assert_eq!(body["status"], json!(413));
    assert_eq!(body["code"], json!("atif_export_too_large"));
    let detail = body["detail"].as_str().expect("detail");
    assert!(
        detail.contains("64-byte limit"),
        "detail must name the limit: {detail}"
    );
    assert!(
        detail.contains(&session_id.to_string()),
        "detail must name the session: {detail}"
    );
    // The dead-end 413 now signposts the recoverable segmented path.
    assert!(
        detail.contains("segmented=true"),
        "413 detail must point at segmented export: {detail}"
    );

    // The cap guards only the single-document ATIF path; JSONL is untouched.
    server
        .get(&format!("/v1/sessions/{}/export", session_id))
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn test_session_export_atif_subagent_trajectory_ref() {
    use everruns_core::events::{InputMessageData, OutputMessageCompletedData, ToolCompletedData};
    use everruns_core::message::{ContentPart, Message};
    use everruns_core::tool_types::ToolCall;
    use everruns_core::typed_id::SessionId as CoreSessionId;

    let server = TestServer::in_memory().await;
    let child = CoreSessionId::new();
    let spawn_call = ToolCall {
        id: "call_spawn".to_string(),
        name: "spawn_agent".to_string(),
        arguments: json!({"target": {"type": "subagent"}, "name": "Worker"}),
    };
    let spawn_result = json!({
        "subagent_id": child.to_string(),
        "name": "Worker",
        "status": "running",
        "task_id": "task_123",
    })
    .to_string();
    let session_id = seed_session_with_raw_events(
        &server,
        vec![
            (
                "input.message",
                serde_json::to_value(InputMessageData::new(Message::user("delegate this")))
                    .expect("serialize input event"),
            ),
            (
                "output.message.completed",
                serde_json::to_value(OutputMessageCompletedData::new(
                    Message::assistant_with_tools("spawning", vec![spawn_call]),
                ))
                .expect("serialize output event"),
            ),
            (
                "tool.completed",
                serde_json::to_value(ToolCompletedData::success(
                    "call_spawn".to_string(),
                    "spawn_agent".to_string(),
                    vec![ContentPart::text(spawn_result)],
                    Some(5),
                ))
                .expect("serialize tool event"),
            ),
        ],
    )
    .await;

    let response = server
        .get(&format!("/v1/sessions/{}/export?format=atif", session_id))
        .await
        .assert_status(StatusCode::OK);
    let trajectory = response.json_value();
    let refs = &trajectory["steps"][1]["observation"]["results"][0]["subagent_trajectory_ref"];
    assert_eq!(
        refs[0]["trajectory_path"],
        json!(format!("/v1/sessions/{child}/export?format=atif"))
    );
    assert_eq!(refs[0]["session_id"], json!(child.to_string()));
}

/// Seed a session with `n` user messages as `input.message` events, oldest
/// first, so the ATIF fold yields `n` ordered user steps.
async fn seed_session_with_n_messages(server: &TestServer, n: usize) -> SessionId {
    use everruns_core::events::InputMessageData;
    use everruns_core::message::Message;

    let events: Vec<(&str, serde_json::Value)> = (0..n)
        .map(|i| {
            (
                "input.message",
                serde_json::to_value(InputMessageData::new(Message::user(format!(
                    "segmented message {i}"
                ))))
                .expect("serialize input event"),
            )
        })
        .collect();
    seed_session_with_raw_events(server, events).await
}

#[tokio::test]
async fn test_session_export_atif_segmented_walk_reconstructs_session() {
    // Tiny cap forces one step per segment; walk the whole chain and confirm
    // the stitched steps reproduce the seeded session in order.
    let server = TestServer::in_memory_with_atif_export_cap(64).await;
    let session_id = seed_session_with_n_messages(&server, 5).await;

    let mut path = format!(
        "/v1/sessions/{}/export?format=atif&segmented=true",
        session_id
    );
    let mut collected: Vec<serde_json::Value> = Vec::new();
    let mut segment_count = 0;
    loop {
        let response = server.get(&path).await.assert_status(StatusCode::OK);
        assert_eq!(
            response.headers().get("content-type").unwrap(),
            "application/json"
        );
        let seg_index_header = response
            .headers()
            .get("x-atif-segment-index")
            .expect("segment index header")
            .to_str()
            .unwrap()
            .to_string();
        let doc = response.json_value();
        assert_eq!(doc["schema_version"], json!("ATIF-v1.7"));
        assert_eq!(doc["session_id"], json!(session_id.to_string()));
        assert_eq!(
            doc["extra"]["segment_index"].as_u64().unwrap().to_string(),
            seg_index_header,
            "extra.segment_index must match the header"
        );
        for step in doc["steps"].as_array().unwrap() {
            collected.push(step.clone());
        }
        segment_count += 1;
        assert!(segment_count < 100, "segmented walk did not terminate");

        match doc.get("continued_trajectory_ref") {
            Some(reference) => {
                let reference = reference.as_str().expect("ref is a string");
                assert!(reference.contains("segmented=true"));
                assert!(reference.contains("cursor="));
                path = reference.to_string();
            }
            None => break,
        }
    }

    assert!(segment_count >= 2, "tiny cap must yield multiple segments");
    // All five user steps reconstructed, in order, with absolute step ids.
    assert_eq!(collected.len(), 5);
    for (i, step) in collected.iter().enumerate() {
        assert_eq!(step["step_id"], json!(i + 1));
        assert_eq!(step["source"], json!("user"));
        assert_eq!(step["message"], json!(format!("segmented message {i}")));
    }
}

#[tokio::test]
async fn test_session_export_atif_segmented_bad_cursor_returns_400() {
    let server = TestServer::in_memory_with_atif_export_cap(64).await;
    let session_id = seed_session_with_n_messages(&server, 3).await;

    // Garbage cursor is rejected without panicking.
    let response = server
        .get(&format!(
            "/v1/sessions/{}/export?format=atif&segmented=true&cursor=not-a-real-cursor",
            session_id
        ))
        .await
        .assert_status(StatusCode::BAD_REQUEST);
    let body = response.json_value();
    assert_eq!(body["code"], json!("atif_cursor_invalid"));
}

#[tokio::test]
async fn test_session_export_atif_segmented_single_segment_when_small() {
    // Default (large) cap: a small session is one self-contained segment with
    // no continuation, even though segmented=true was requested.
    let server = TestServer::in_memory().await;
    let session_id = seed_session_with_n_messages(&server, 2).await;

    let response = server
        .get(&format!(
            "/v1/sessions/{}/export?format=atif&segmented=true",
            session_id
        ))
        .await
        .assert_status(StatusCode::OK);
    assert!(response.headers().get("x-atif-next-cursor").is_none());
    let doc = response.json_value();
    assert!(doc.get("continued_trajectory_ref").is_none());
    assert_eq!(doc["steps"].as_array().unwrap().len(), 2);
    assert_eq!(doc["extra"]["segment_index"], json!(0));
}
