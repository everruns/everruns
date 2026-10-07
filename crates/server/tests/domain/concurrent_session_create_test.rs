//! Sessions created at once for one agent.
//!
//! Session creation makes the agent's server-managed memory on first use. Two
//! creations that both miss the existing row race on the insert; the loser
//! must reuse the winner's row rather than fail with 409 (found by the llmsim
//! load test, which opens many sessions for one agent at once).

use crate::test_harness;

use serde_json::json;
use test_harness::TestServer;

#[tokio::test]
async fn parallel_sessions_for_one_new_agent_all_succeed() {
    let server = TestServer::new().await;
    let agent = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("parallel-sessions-{}", uuid::Uuid::now_v7().simple()),
                "system_prompt": "Test",
            }),
        )
        .await
        .assert_success()
        .json_value();
    let agent_id = agent["id"].as_str().unwrap().to_string();

    let body = json!({
        "harness_id": server.seed_base_harness_id,
        "agent_id": agent_id,
    });
    let responses =
        futures::future::join_all((0..32).map(|_| server.post("/v1/sessions", body.clone()))).await;

    let statuses: Vec<_> = responses.iter().map(|r| r.status().as_u16()).collect();
    assert!(
        statuses.iter().all(|s| (200..300).contains(s)),
        "every parallel session creation succeeds, got {statuses:?}"
    );

    // One agent memory row, shared by all of them.
    let memories: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM memories WHERE scope = 'agent' AND owner_agent_id = $1 AND status = 'active'",
    )
    .bind(
        agent_id
            .parse::<everruns_contracts::typed_id::AgentId>()
            .unwrap()
            .uuid(),
    )
    .fetch_one(&server.pool)
    .await
    .unwrap();
    assert_eq!(memories, 1);
}
