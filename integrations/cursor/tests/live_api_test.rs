//! Live API smoke tests for Cursor Cloud Agents.
//!
//! Run with:
//!   doppler run -- cargo test -p everruns-integrations-cursor --features cursor-live-tests --test live_api_test -- --test-threads=1
//!
//! Tests fail closed if CURSOR_API_KEY is missing.

#![cfg(feature = "cursor-live-tests")]

use everruns_integrations_cursor::client::CursorClient;

fn require_api_key() -> String {
    match std::env::var("CURSOR_API_KEY") {
        Ok(value) if !value.trim().is_empty() => value,
        _ => panic!("CURSOR_API_KEY not set — cannot run Cursor live tests"),
    }
}

#[tokio::test]
async fn smoke_api_key_info() {
    let client = CursorClient::new(require_api_key());
    let info = client
        .api_key_info()
        .await
        .expect("Cursor API key info should succeed");
    assert!(!info.api_key_name.is_empty());
    assert!(!info.created_at.is_empty());
}

#[tokio::test]
async fn smoke_list_models() {
    let client = CursorClient::new(require_api_key());
    let models = client
        .list_models()
        .await
        .expect("Cursor list models should succeed");
    assert!(!models.models.is_empty());
}

#[tokio::test]
async fn smoke_list_agents_no_create() {
    let client = CursorClient::new(require_api_key());
    match client.list_agents(Some(1), None).await {
        Ok(agents) => assert!(agents.agents.len() <= 1),
        Err(err)
            if err
                .to_string()
                .contains("Cloud agent is not supported in Privacy Mode (Legacy)") =>
        {
            eprintln!(
                "Cursor cloud agents are disabled for this workspace privacy mode; skipping agents list smoke"
            );
        }
        Err(err) => panic!("Cursor list agents should succeed: {err}"),
    }
}
