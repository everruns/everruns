//! Live ARD registry tests against the public reference registry.
//!
//! Gated behind:
//!   cargo test -p everruns-ard --features ard-live-tests
//!
//! Fail-closed contract (specs/integrations.md): when the feature flag is on but
//! the required env var is missing or empty, these tests MUST `panic!` — CI live
//! jobs must never silently pass. The registry base URL is read from
//! `ARD_LIVE_REGISTRY_URL`; an optional bearer token from `ARD_REGISTRY_TOKEN`.

#![cfg(feature = "ard-live-tests")]

use everruns_ard::client::{ArdRegistryClient, Federation, SearchQuery, SearchRequest};

/// Read a required env var or fail closed.
fn required_env(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => panic!(
            "{name} must be set (and non-empty) when running with --features ard-live-tests; \
             refusing to silently pass"
        ),
    }
}

fn live_client() -> ArdRegistryClient {
    let base = required_env("ARD_LIVE_REGISTRY_URL");
    let token = std::env::var("ARD_REGISTRY_TOKEN")
        .ok()
        .filter(|t| !t.is_empty());
    ArdRegistryClient::new(base, token)
}

#[tokio::test]
async fn live_search_returns_results() {
    let client = live_client();
    let request = SearchRequest {
        query: SearchQuery {
            text: "documentation search".to_string(),
            filter: None,
        },
        federation: Some(Federation::None),
        page_size: Some(5),
    };

    let response = client
        .search(&request)
        .await
        .expect("live ARD search should succeed");

    // Validate the envelope of every returned entry — exactly one of url/data,
    // a parseable media type, and a non-empty identifier.
    for entry in &response.results {
        assert!(!entry.identifier.is_empty(), "entry missing identifier");
        entry
            .validate_envelope()
            .unwrap_or_else(|e| panic!("entry envelope invalid: {e}"));
        assert!(
            entry.media_type.contains('/'),
            "entry {} has a non-media-type 'type': {}",
            entry.identifier,
            entry.media_type
        );
    }
}

#[tokio::test]
async fn live_search_filtered_by_type() {
    let client = live_client();
    let request = SearchRequest {
        query: SearchQuery {
            text: "agent".to_string(),
            filter: Some(serde_json::json!({
                "type": ["application/a2a-agent-card+json"]
            })),
        },
        federation: Some(Federation::None),
        page_size: Some(5),
    };

    // We only assert the request/response round-trips and parses; the public
    // registry's contents are not under our control.
    let _ = client
        .search(&request)
        .await
        .expect("live filtered ARD search should succeed");
}
