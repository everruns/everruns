//! Real API smoke tests for DuckDuckGo.
//!
//! Gated behind `integration` feature — only compiled when run with:
//!   cargo test -p everruns-integrations-duckduckgo --features integration
//!
//! DuckDuckGo Instant Answer API is free and requires no API key,
//! but we gate these tests to avoid hitting the API in every CI run.

#![cfg(feature = "integration")]

use everruns_integrations_duckduckgo::client::DuckDuckGoClient;

#[tokio::test]
async fn smoke_basic_search() {
    let client = DuckDuckGoClient::new();

    let resp = client
        .instant_answer("Rust programming language", false)
        .await
        .expect("search should succeed");

    // Should return an article-type response for a well-known topic
    assert!(
        !resp.heading.is_empty() || !resp.r#abstract.is_empty(),
        "should return some content for a well-known query"
    );
}

#[tokio::test]
async fn smoke_calculation_answer() {
    let client = DuckDuckGoClient::new();

    let resp = client
        .instant_answer("2+2", false)
        .await
        .expect("calculation query should succeed");

    // DuckDuckGo may or may not return an answer for simple calculations,
    // but the API call should succeed without error.
    let _ = resp;
}

#[tokio::test]
async fn smoke_no_html_mode() {
    let client = DuckDuckGoClient::new();

    let resp = client
        .instant_answer("HTML", true)
        .await
        .expect("no_html search should succeed");

    // With no_html=true, abstract text should not contain HTML tags
    if !resp.r#abstract.is_empty() {
        assert!(
            !resp.r#abstract.contains("<a href="),
            "no_html mode should strip HTML tags"
        );
    }
}

#[tokio::test]
async fn smoke_disambiguation() {
    let client = DuckDuckGoClient::new();

    let resp = client
        .instant_answer("Mercury", false)
        .await
        .expect("disambiguation query should succeed");

    // Mercury is ambiguous (planet, element, god) — should return related topics
    // or a disambiguation type response
    let has_content =
        !resp.heading.is_empty() || !resp.related_topics.is_empty() || !resp.r#abstract.is_empty();
    assert!(
        has_content,
        "should return some content for ambiguous query"
    );
}
