//! Smoke tests against real Brave Search API.
//! Run with: doppler run -- cargo test -p everruns-integrations-brave-search --test smoke_real_api -- --ignored

use everruns_integrations_brave_search::client::BraveSearchClient;

fn api_key() -> Option<String> {
    std::env::var("BRAVE_SEARCH_API_KEY")
        .ok()
        .filter(|k| !k.is_empty())
}

#[tokio::test]
#[ignore] // requires BRAVE_SEARCH_API_KEY
async fn smoke_basic_search() {
    let key = api_key().expect("BRAVE_SEARCH_API_KEY required");
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
#[ignore]
async fn smoke_freshness_filter() {
    let key = api_key().expect("BRAVE_SEARCH_API_KEY required");
    let client = BraveSearchClient::new(key);

    let resp = client
        .web_search("AI news", Some(2), None, Some("pw"))
        .await
        .expect("search with freshness should succeed");

    // May or may not have results, but should not error
    let _results = resp.web.map(|w| w.results).unwrap_or_default();
}

#[tokio::test]
#[ignore]
async fn smoke_pagination() {
    let key = api_key().expect("BRAVE_SEARCH_API_KEY required");
    let client = BraveSearchClient::new(key);

    let resp = client
        .web_search("Rust async await", Some(2), Some(5), None)
        .await
        .expect("search with offset should succeed");

    let _results = resp.web.map(|w| w.results).unwrap_or_default();
}
