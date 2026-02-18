//! Real API smoke tests for Brave Search.
//!
//! Gated behind `integration` feature — only compiled when run with:
//!   cargo test -p everruns-integrations-brave-search --features integration
//!
//! CI workflow passes BRAVE_SEARCH_API_KEY and enables the feature automatically.

#![cfg(feature = "integration")]

use everruns_integrations_brave_search::client::BraveSearchClient;

fn api_key() -> String {
    std::env::var("BRAVE_SEARCH_API_KEY").expect("BRAVE_SEARCH_API_KEY must be set")
}

#[tokio::test]
async fn smoke_basic_search() {
    let client = BraveSearchClient::new(api_key());

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
    let client = BraveSearchClient::new(api_key());

    let resp = client
        .web_search("AI news", Some(2), None, Some("pw"))
        .await
        .expect("search with freshness should succeed");

    // May or may not have results, but should not error
    let _results = resp.web.map(|w| w.results).unwrap_or_default();
}

#[tokio::test]
async fn smoke_pagination() {
    let client = BraveSearchClient::new(api_key());

    let resp = client
        .web_search("Rust async await", Some(2), Some(5), None)
        .await
        .expect("search with offset should succeed");

    let _results = resp.web.map(|w| w.results).unwrap_or_default();
}
