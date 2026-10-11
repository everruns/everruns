use std::sync::Arc;

use super::*;
use crate::web_search::http::DirectEgress;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const DDG_HTML: &str = include_str!("fixtures/duckduckgo_html.html");
const DDG_NO_RESULTS: &str = include_str!("fixtures/duckduckgo_html_no_results.html");
const DDG_BOT_CHECK: &str = include_str!("fixtures/duckduckgo_bot_check.html");
const DDG_LITE: &str = include_str!("fixtures/duckduckgo_lite.html");
const MOJEEK: &str = include_str!("fixtures/mojeek.html");
const MOJEEK_BLOCKED: &str = include_str!("fixtures/mojeek_blocked.html");

fn hits(outcome: PageOutcome) -> Vec<SearchHit> {
    match outcome {
        PageOutcome::Hits(hits) => hits,
        PageOutcome::NoResults => panic!("expected hits, got no results"),
        PageOutcome::Blocked(why) => panic!("expected hits, got blocked: {why}"),
    }
}

#[test]
fn duckduckgo_html_reads_each_result_from_its_own_block() {
    let hits = hits(parse_duckduckgo_html(200, DDG_HTML));
    // The ad is skipped.
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].title, "Tokio - An asynchronous Rust runtime");
    assert_eq!(hits[0].url, "https://tokio.rs/");
    assert_eq!(hits[0].site, "tokio.rs");
    assert_eq!(hits[0].published.as_deref(), Some("2026-10-02"));
    assert!(hits[0].snippet.as_deref().unwrap().starts_with("Tokio is an event-driven"));
    // The GitHub result has no snippet: it must not take the docs.rs one.
    assert_eq!(hits[1].url, "https://github.com/tokio-rs/tokio");
    assert!(hits[1].snippet.is_none());
    assert!(hits[2].snippet.as_deref().unwrap().contains("event-driven & non-blocking"));
}

#[test]
fn duckduckgo_bot_check_is_a_failure_not_zero_results() {
    // Recorded from a datacenter address: HTTP 202 with the anomaly challenge.
    assert!(matches!(
        parse_duckduckgo_html(202, DDG_BOT_CHECK),
        PageOutcome::Blocked(_)
    ));
    // The markers alone are enough, whatever the status.
    assert!(matches!(
        parse_duckduckgo_html(200, DDG_BOT_CHECK),
        PageOutcome::Blocked(_)
    ));
    assert!(matches!(
        parse_duckduckgo_lite(200, DDG_BOT_CHECK),
        PageOutcome::Blocked(_)
    ));
}

#[test]
fn an_unrecognised_page_is_a_failure_and_a_real_empty_page_is_not() {
    assert!(matches!(
        parse_duckduckgo_html(200, "<html><body>maintenance</body></html>"),
        PageOutcome::Blocked(_)
    ));
    assert!(matches!(
        parse_duckduckgo_html(200, DDG_NO_RESULTS),
        PageOutcome::NoResults
    ));
}

#[test]
fn duckduckgo_lite_pairs_snippets_with_their_link() {
    let hits = hits(parse_duckduckgo_lite(200, DDG_LITE));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].title, "Announcing Tokio 1.50");
    assert_eq!(
        hits[0].url,
        "https://tokio.rs/blog/2026-10-tokio-1-50"
    );
    assert_eq!(
        hits[0].snippet.as_deref(),
        Some("Today we are releasing Tokio 1.50 — with a new scheduler.")
    );
    assert_eq!(hits[1].url, "https://crates.io/crates/tokio");
    assert!(hits[1].snippet.is_none());
}

#[test]
fn mojeek_results_and_block_page() {
    let hits = hits(parse_mojeek(200, MOJEEK));
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].title, "Tutorial | Tokio");
    assert!(hits[0].snippet.as_deref().unwrap().contains("Rust programming"));
    assert!(hits[1].snippet.is_none());
    // Recorded: Mojeek's 403 for automated traffic.
    assert!(matches!(parse_mojeek(403, MOJEEK_BLOCKED), PageOutcome::Blocked(_)));
}

#[test]
fn wikipedia_api_results() {
    let body = serde_json::json!({"query": {"search": [
        {"title": "Tokio (software)", "snippet": "<span class=\"searchmatch\">Tokio</span> is a runtime", "timestamp": "2026-09-01T10:00:00Z"}
    ]}})
    .to_string();
    let hits = hits(parse_wikipedia(200, &body));
    assert_eq!(hits[0].url, "https://en.wikipedia.org/wiki/Tokio_%28software%29");
    assert_eq!(hits[0].snippet.as_deref(), Some("Tokio is a runtime"));
    assert_eq!(hits[0].published.as_deref(), Some("2026-09-01"));
}

#[test]
fn result_urls_unwrap_redirects_and_drop_ads() {
    assert_eq!(
        result_url("//duckduckgo.com/l/?uddg=https%3A%2F%2Fa.com%2Fx%3Fy%3D1&rut=z").as_deref(),
        Some("https://a.com/x?y=1")
    );
    assert!(result_url("https://duckduckgo.com/y.js?ad=1").is_none());
    assert!(result_url("javascript:void(0)").is_none());
}

#[test]
fn interleave_takes_one_from_each_list_in_turn() {
    let list = |prefix: &str, n: usize| -> Vec<SearchHit> {
        (0..n)
            .map(|i| SearchHit::new(format!("{prefix}{i}"), format!("https://{prefix}.com/{i}")))
            .collect()
    };
    let titles: Vec<_> = interleave(vec![list("w", 3), list("g", 1), list("n", 2)])
        .into_iter()
        .map(|hit| hit.title)
        .collect();
    assert_eq!(titles, ["w0", "g0", "n0", "w1", "n1", "w2"]);
}

#[test]
fn site_operators_narrow_the_query() {
    let mut request = SearchRequest::new("tokio");
    request.allowed_domains = vec!["docs.rs".into()];
    assert_eq!(query_with_sites(&request), "tokio site:docs.rs");
    request.allowed_domains.push("tokio.rs".into());
    assert_eq!(query_with_sites(&request), "tokio (site:docs.rs OR site:tokio.rs)");
}

fn endpoints(server: &MockServer) -> FreeSearchEndpoints {
    let base = server.uri();
    FreeSearchEndpoints {
        duckduckgo_html: format!("{base}/html/"),
        duckduckgo_lite: format!("{base}/lite/"),
        mojeek: format!("{base}/mojeek"),
        wikipedia: format!("{base}/w/api.php"),
        instant_answer: format!("{base}/ia"),
        github: format!("{base}/github"),
        npm: format!("{base}/npm"),
        crates: format!("{base}/crates"),
    }
}

fn context() -> SearchContext {
    SearchContext {
        egress: Arc::new(DirectEgress::new()),
        network_access: None,
    }
}

async fn mount(server: &MockServer, at: &str, status: u16, body: &str) {
    Mock::given(method("GET"))
        .and(path(at))
        .respond_with(ResponseTemplate::new(status).set_body_string(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn falls_through_a_bot_check_to_the_next_source() {
    let server = MockServer::start().await;
    mount(&server, "/html/", 202, DDG_BOT_CHECK).await;
    mount(&server, "/lite/", 200, DDG_LITE).await;
    mount(&server, "/ia", 200, "{}").await;
    let driver = FreeSearchDriver::with_endpoints(endpoints(&server));

    let response = driver
        .search(&SearchRequest::new("tokio 1.50"), &context())
        .await
        .unwrap();
    assert_eq!(response.hits.len(), 2);
    assert_eq!(response.hits[0].source.as_deref(), Some("duckduckgo_lite"));
    assert_eq!(response.notes, ["DuckDuckGo showed a bot check."]);
    // Every request is kept for the raw view: html, lite and instant answer.
    assert_eq!(response.exchanges.len(), 3);
    assert_eq!(response.exchanges[0].status, Some(202));
}

#[tokio::test]
async fn every_source_blocked_is_an_error_with_the_raw_exchanges() {
    let server = MockServer::start().await;
    mount(&server, "/html/", 202, DDG_BOT_CHECK).await;
    mount(&server, "/lite/", 202, DDG_BOT_CHECK).await;
    mount(&server, "/mojeek", 403, MOJEEK_BLOCKED).await;
    mount(&server, "/w/api.php", 503, "down").await;
    mount(&server, "/ia", 200, "{}").await;
    let driver = FreeSearchDriver::with_endpoints(endpoints(&server));

    let failure = driver
        .search(&SearchRequest::new("tokio"), &context())
        .await
        .unwrap_err();
    assert_eq!(failure.kind, SearchFailureKind::Blocked);
    assert!(failure.message.contains("DuckDuckGo showed a bot check"));
    assert!(failure.message.contains("Mojeek refused automated queries"));
    assert_eq!(failure.exchanges.len(), 5);
}

#[tokio::test]
async fn dev_topic_interleaves_registries_and_adds_the_instant_answer() {
    let server = MockServer::start().await;
    mount(&server, "/html/", 200, DDG_HTML).await;
    mount(
        &server,
        "/ia",
        200,
        r#"{"AbstractText":"Tokio is a runtime.","AbstractSource":"Wikipedia","AbstractURL":"https://en.wikipedia.org/wiki/Tokio"}"#,
    )
    .await;
    mount(
        &server,
        "/github",
        200,
        r#"{"items":[{"full_name":"tokio-rs/axum","html_url":"https://github.com/tokio-rs/axum","description":"web framework"}]}"#,
    )
    .await;
    mount(
        &server,
        "/npm",
        200,
        r#"{"objects":[{"package":{"name":"tokio-js","links":{"npm":"https://www.npmjs.com/package/tokio-js"}}}]}"#,
    )
    .await;
    mount(
        &server,
        "/crates",
        200,
        r#"{"crates":[{"name":"tokio","description":" runtime ","updated_at":"2026-10-02T00:00:00Z"}]}"#,
    )
    .await;
    let driver = FreeSearchDriver::with_endpoints(endpoints(&server));
    let mut request = SearchRequest::new("tokio");
    request.topic = SearchTopic::Dev;
    request.limit = 5;

    let response = driver.search(&request, &context()).await.unwrap();
    let urls: Vec<_> = response.hits.iter().map(|hit| hit.url.as_str()).collect();
    assert_eq!(
        urls,
        [
            "https://tokio.rs/",
            "https://github.com/tokio-rs/axum",
            "https://www.npmjs.com/package/tokio-js",
            "https://crates.io/crates/tokio",
            "https://github.com/tokio-rs/tokio",
        ]
    );
    assert_eq!(response.answer.unwrap().source, "Wikipedia");
}

#[tokio::test]
async fn blocked_domains_are_removed() {
    let server = MockServer::start().await;
    mount(&server, "/html/", 200, DDG_HTML).await;
    mount(&server, "/ia", 200, "{}").await;
    let driver = FreeSearchDriver::with_endpoints(endpoints(&server));
    let mut request = SearchRequest::new("tokio");
    request.blocked_domains = vec!["github.com".into()];

    let response = driver.search(&request, &context()).await.unwrap();
    assert!(response.hits.iter().all(|hit| hit.site != "github.com"));
    assert_eq!(response.hits.len(), 2);
}
