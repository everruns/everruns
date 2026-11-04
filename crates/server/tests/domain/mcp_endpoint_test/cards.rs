//! Agent-card MCP Apps response and session-count regression coverage.

use super::*;

/// Extract the text fallback from a card tool result.
fn card_summary_text(resp: &Value) -> String {
    resp["result"]["content"]
        .as_array()
        .and_then(|items| {
            items
                .iter()
                .find(|item| item["type"] == "text")
                .and_then(|item| item["text"].as_str())
        })
        .unwrap_or("")
        .to_string()
}

// Regression test for the `agent_get_card` session-count bug: the count must
// be keyed by the agent's internal id, not its public id. Public and internal
// ids are distinct UUIDs (see `crates/server/src/storage/repositories/agents/mod.rs`
// where `id` and `public_id` are populated independently), and
// `sessions.agent_id` references `agents.id`. If the count keyed by public_id
// instead, the assertion below would observe `0` sessions despite one existing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mcp_agent_get_card_counts_sessions_by_internal_id() {
    let server = TestServer::new().await;

    // Create an agent via REST (auto-generated public_id != internal_id).
    let agent: Value = server
        .post(
            "/v1/agents",
            json!({
                "name": format!("card-count-agent-{}", unique_suffix()),
                "display_name": "Card Count Agent",
                "system_prompt": "Test"
            }),
        )
        .await
        .assert_success()
        .json();
    let agent_public_id = agent["id"].as_str().unwrap().to_string();

    // Baseline: no sessions yet, card should report 0.
    let resp = mcp_tool_call_latest(
        &server,
        "agent_get_card",
        json!({ "agent_id": agent_public_id }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "agent_get_card failed: {}",
        tool_text(&resp)
    );
    assert!(
        card_summary_text(&resp).contains("0 session(s)"),
        "expected 0 sessions in summary, got: {}",
        card_summary_text(&resp)
    );

    assert_eq!(
        resp["result"]["structuredContent"]["agent_card"]["public_id"],
        agent_public_id
    );
    assert!(
        resp["result"]["content"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["type"] == "text")
    );

    // Create a session bound to this agent (REST resolves public_id ->
    // internal_id and stores the internal id in `sessions.agent_id`).
    let _session: Value = server
        .post(
            "/v1/sessions",
            json!({
                "harness_id": server.seed_base_harness_id,
                "agent_id": agent_public_id,
            }),
        )
        .await
        .assert_success()
        .json();

    // Now the card must reflect 1 session. With the old (buggy) code that
    // counted by public_id, this would still be 0.
    let resp = mcp_tool_call_latest(
        &server,
        "agent_get_card",
        json!({ "agent_id": agent_public_id }),
    )
    .await;
    assert!(
        !tool_is_error(&resp),
        "agent_get_card failed: {}",
        tool_text(&resp)
    );
    assert!(
        card_summary_text(&resp).contains("1 session(s)"),
        "expected 1 session in summary, got: {}",
        card_summary_text(&resp)
    );
}
