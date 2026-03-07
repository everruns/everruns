//! Real API smoke tests for Brave Search.
//!
//! Gated behind `integration` feature — only compiled when run with:
//!   cargo test -p everruns-integrations-brave-search --features integration
//!
//! CI workflow passes BRAVE_SEARCH_API_KEY via Doppler and enables the feature
//! automatically. Tests panic if the key is missing to prevent silent failures.

#![cfg(feature = "integration")]

use everruns_integrations_brave_search::client::BraveSearchClient;

macro_rules! require_api_key {
    () => {
        match std::env::var("BRAVE_SEARCH_API_KEY") {
            Ok(k) if !k.is_empty() => k,
            _ => panic!("BRAVE_SEARCH_API_KEY not set — cannot run integration tests"),
        }
    };
}

#[tokio::test]
async fn smoke_basic_search() {
    let key = require_api_key!();
    let client = BraveSearchClient::new(key);

    let resp = client
        .web_search("Rust programming language", Some(3), None, None)
        .await
        .expect("search should succeed");

    let results = resp.web.expect("web field should exist").results;
    assert!(!results.is_empty(), "should return results");
    assert!(!results[0].title.is_empty());
    assert!(!results[0].url.is_empty());
}

#[tokio::test]
async fn smoke_freshness_filter() {
    let key = require_api_key!();
    let client = BraveSearchClient::new(key);

    let resp = client
        .web_search("AI news", Some(2), None, Some("pw"))
        .await
        .expect("search with freshness should succeed");

    // May or may not have results, but should not error
    let _results = resp.web.map(|w| w.results).unwrap_or_default();
}

#[tokio::test]
async fn smoke_pagination() {
    let key = require_api_key!();
    let client = BraveSearchClient::new(key);

    let resp = client
        .web_search("Rust async await", Some(2), Some(5), None)
        .await
        .expect("search with offset should succeed");

    let _results = resp.web.map(|w| w.results).unwrap_or_default();
}
