//! Eval runs as case tasks: repeated trials, pass^k summaries, cancellation,
//! spending caps, and cases running with the authority of the person who
//! started the run.
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

const TEST_ORG_ID: i64 = 1;

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

#[tokio::test]
async fn cases_run_as_the_person_who_started_the_run() {
    let server = TestServer::in_memory().await;
    let eval = seed_eval(&server, 1, 1).await;
    let run: Value = server
        .post(&format!("/v1/evals/{}/runs", eval.public_id), json!({}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let run_id = run["id"].as_str().expect("run id").to_string();
    let done = wait_for_status(&server, &eval, &run_id, "completed").await;

    let run_row = server
        .db
        .get_eval_run_by_public_id(TEST_ORG_ID, &run_id)
        .await
        .unwrap()
        .expect("run row");
    let starter = run_row
        .started_by_user_id
        .expect("a person started this run");

    // The case's session belongs to that person, not to the system caller.
    let session_id: everruns_contracts::typed_id::SessionId = done["results"][0]["session_id"]
        .as_str()
        .expect("case session")
        .parse()
        .unwrap();
    let session = server
        .db
        .get_session(TEST_ORG_ID, session_id)
        .await
        .unwrap()
        .expect("session row");
    assert_eq!(session.resolved_owner_user_id, Some(starter));
}

#[tokio::test]
async fn run_spending_caps_outside_bounds_are_rejected() {
    let server = TestServer::in_memory().await;
    let eval = seed_eval(&server, 1, 1).await;
    for cap in [json!(0), json!(-1.5), json!(1_000_000_000.0)] {
        let response = server
            .post(
                &format!("/v1/evals/{}/runs", eval.public_id),
                json!({"max_cost_usd": cap}),
            )
            .await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "cap={cap}");
        let _ = response.text();
    }
}

#[tokio::test]
async fn a_capped_run_binds_its_case_sessions_to_one_budget() {
    let server = TestServer::in_memory().await;
    // Long timeout: the case is still waiting while its budgets are read.
    let eval = seed_eval(&server, 1, 60).await;
    let run: Value = server
        .post(
            &format!("/v1/evals/{}/runs", eval.public_id),
            json!({"max_cost_usd": 5.0}),
        )
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    assert_eq!(run["max_cost_usd"], 5.0);
    let run_id = run["id"].as_str().expect("run id").to_string();

    let budgets = server
        .db
        .list_budgets(TEST_ORG_ID, Some("eval_run"), Some(&run_id))
        .await
        .unwrap();
    assert_eq!(budgets.len(), 1, "the cap is created with the run");
    assert_eq!(budgets[0].limit, 5.0);
    assert_eq!(budgets[0].currency, "usd");

    // The case session spends from the run's budget.
    let mut session_id = None;
    for _ in 0..80 {
        let current = get_run(&server, &eval, &run_id).await;
        if let Some(id) = current["results"][0]["session_id"].as_str() {
            session_id = Some(id.to_string());
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let session_id = session_id.expect("the case created its session");
    let hierarchy = everruns_server::domains::budgets::BudgetService::new(server.db.clone())
        .list_budgets_for_session_hierarchy(TEST_ORG_ID, &session_id, None)
        .await;
    let subjects: Vec<&str> = hierarchy.iter().map(|b| b.subject_type.as_str()).collect();
    assert!(subjects.contains(&"eval_run"), "{subjects:?}");

    server
        .post(
            &format!("/v1/evals/{}/runs/{run_id}/cancel", eval.public_id),
            json!({}),
        )
        .await
        .assert_status(StatusCode::OK);
}

#[tokio::test]
async fn a_spent_budget_skips_cases_instead_of_failing_them() {
    use everruns_server::storage::CreateBudgetRow;

    let server = TestServer::in_memory().await;
    let eval = seed_eval(&server, 2, 60).await;
    // The org's money is already spent, so no case may send its message.
    let budget = server
        .db
        .create_budget(CreateBudgetRow {
            org_id: TEST_ORG_ID,
            subject_type: "org".into(),
            subject_id: everruns_core::org_public_id_from_internal(TEST_ORG_ID),
            currency: "usd".into(),
            limit: 1.0,
            soft_limit: None,
            period: None,
            metadata: None,
        })
        .await
        .unwrap();
    server
        .db
        .set_budget_status(budget.id, "exhausted")
        .await
        .unwrap();

    let run: Value = server
        .post(&format!("/v1/evals/{}/runs", eval.public_id), json!({}))
        .await
        .assert_status(StatusCode::CREATED)
        .json();
    let run_id = run["id"].as_str().expect("run id").to_string();
    // Well before the 60 s case timeout: the cases stop at the budget check.
    let done = wait_for_status(&server, &eval, &run_id, "completed").await;
    let results = done["results"].as_array().expect("results");
    assert!(
        results.iter().all(|r| r["status"] == "skipped"
            && r["error_message"]
                .as_str()
                .is_some_and(|m| m.starts_with("Stopped: "))),
        "{results:?}"
    );
    assert_eq!(done["summary"]["errored"], 0);
    assert_eq!(done["summary"]["total_cost_usd"], 0.0);
}
