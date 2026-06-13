//! Guardrails integration tests
//!
//! Covers the `guardrails` capability surfacing in the capability listing
//! and the `POST /v1/capabilities/guardrails/dry-run` endpoint.

mod test_harness;

use axum::http::StatusCode;
use serde_json::json;
use test_harness::TestServer;

#[tokio::test]
async fn test_guardrails_capability_appears_with_marker() {
    let server = TestServer::new().await;

    let resp = server.get("/v1/capabilities").await.assert_success();
    let body = resp.json_value();
    let capabilities = body["data"].as_array().unwrap();

    let guardrails = capabilities
        .iter()
        .find(|c| c["id"] == "guardrails")
        .expect("guardrails capability should be listed");
    assert_eq!(guardrails["is_guardrail"], true);
    // Built-in prompt canary guardrail is also flagged.
    let canary = capabilities
        .iter()
        .find(|c| c["id"] == "prompt_canary_guardrail")
        .expect("prompt_canary_guardrail should be listed");
    assert_eq!(canary["is_guardrail"], true);
    // A non-guardrail capability omits the flag (serde skips false).
    let current_time = capabilities
        .iter()
        .find(|c| c["id"] == "current_time")
        .expect("current_time should be listed");
    assert!(current_time.get("is_guardrail").is_none());
}

#[tokio::test]
async fn test_dry_run_blocks_on_match() {
    let server = TestServer::new().await;

    let resp = server
        .post(
            "/v1/capabilities/guardrails/dry-run",
            json!({
                "config": {
                    "checks": [
                        {"id": "no_ssn", "stage": "output", "type": "regex",
                         "patterns": ["\\b\\d{3}-\\d{2}-\\d{4}\\b"]}
                    ]
                },
                "stage": "output",
                "text": "my ssn is 123-45-6789"
            }),
        )
        .await
        .assert_success();
    let body = resp.json_value();
    assert_eq!(body["blocked"], true);
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["check_id"], "no_ssn");
    assert_eq!(hits[0]["action"], "block");
    assert_eq!(hits[0]["reason_code"], "guardrail.regex");
}

#[tokio::test]
async fn test_dry_run_advisory_logs_without_blocking() {
    let server = TestServer::new().await;

    let resp = server
        .post(
            "/v1/capabilities/guardrails/dry-run",
            json!({
                "config": {
                    "mode": "advisory",
                    "checks": [
                        {"stage": "output", "type": "blocklist", "words": ["forbidden"]}
                    ]
                },
                "stage": "output",
                "text": "this is forbidden"
            }),
        )
        .await
        .assert_success();
    let body = resp.json_value();
    assert_eq!(body["blocked"], false);
    let hits = body["hits"].as_array().unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0]["action"], "log");
}

#[tokio::test]
async fn test_dry_run_no_match_is_empty() {
    let server = TestServer::new().await;

    let resp = server
        .post(
            "/v1/capabilities/guardrails/dry-run",
            json!({
                "config": {
                    "checks": [
                        {"stage": "output", "type": "blocklist", "words": ["forbidden"]}
                    ]
                },
                "stage": "output",
                "text": "perfectly fine content"
            }),
        )
        .await
        .assert_success();
    let body = resp.json_value();
    assert_eq!(body["blocked"], false);
    assert!(body["hits"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn test_dry_run_tool_pattern_uses_tool_name() {
    let server = TestServer::new().await;

    let resp = server
        .post(
            "/v1/capabilities/guardrails/dry-run",
            json!({
                "config": {
                    "checks": [
                        {"stage": "tool_use", "type": "tool_pattern", "tools": ["bash*"]}
                    ]
                },
                "stage": "tool_use",
                "text": "{}",
                "tool_name": "bashkit_exec"
            }),
        )
        .await
        .assert_success();
    let body = resp.json_value();
    assert_eq!(body["blocked"], true);
    assert_eq!(body["hits"][0]["rule_type"], "tool_pattern");
}

#[tokio::test]
async fn test_dry_run_rejects_invalid_regex() {
    let server = TestServer::new().await;

    let resp = server
        .post(
            "/v1/capabilities/guardrails/dry-run",
            json!({
                "config": {
                    "checks": [
                        {"stage": "output", "type": "regex", "patterns": ["("]}
                    ]
                },
                "stage": "output",
                "text": "anything"
            }),
        )
        .await;
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
}
