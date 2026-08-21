//! End-to-end check of the `/metrics` endpoint.
//!
//! Runs in its own test binary because the recorder installs globally, once per
//! process. This is the smoke test for the in-tree Prometheus recorder: it
//! drives the real middleware, serves the real route over TCP, and asserts on
//! the scraped body — the same path a Prometheus server takes.

use axum::Router;
use axum::routing::get;
use everruns_server::api::prometheus::{
    http_metrics_layer, install_prometheus_recorder, names, route,
};

#[tokio::test]
async fn metrics_endpoint_serves_valid_exposition_for_recorded_requests() {
    let handle = install_prometheus_recorder().expect("recorder installs once per process");

    // An instrumented application route, wired exactly as the server wires it.
    let app = Router::new()
        .route("/v1/things/{id}", get(|| async { "ok" }))
        .route_layer(axum::middleware::from_fn(http_metrics_layer));

    let app_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let app_address = app_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(app_listener, app).await });

    let metrics_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let metrics_address = metrics_listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(metrics_listener, route(handle)).await });

    let client = reqwest::Client::new();
    for _ in 0..3 {
        let response = client
            .get(format!("http://{app_address}/v1/things/42"))
            .send()
            .await
            .expect("app request");
        assert!(response.status().is_success());
    }

    let response = client
        .get(format!("http://{metrics_address}/metrics"))
        .send()
        .await
        .expect("scrape");

    assert_eq!(
        response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
        Some("text/plain; version=0.0.4; charset=utf-8"),
    );
    let body = response.text().await.expect("body");

    // The counter carries the matched route template, not the concrete id, so
    // path cardinality stays bounded.
    let counter = format!(
        "{}{{method=\"GET\",path=\"/v1/things/{{id}}\",status=\"200\"}} 3",
        names::HTTP_REQUESTS_TOTAL
    );
    assert!(body.contains(&counter), "missing {counter} in:\n{body}");
    assert!(!body.contains("/v1/things/42"), "id leaked into a label");

    // Durations render as histograms, not summaries.
    let duration = names::HTTP_REQUEST_DURATION;
    assert!(body.contains(&format!("# TYPE {duration} histogram")));
    assert!(body.contains(&format!("{duration}_bucket")));
    assert!(body.contains(&format!("{duration}_count")));
    assert!(body.contains(&format!("{duration}_sum")));
    assert!(
        !body.contains("quantile="),
        "summary quantiles must not appear:\n{body}"
    );

    assert_exposition_is_well_formed(&body);
}

/// Minimal Prometheus text-format validation: every non-comment line is
/// `name[{labels}] value`, every value parses, and every metric that emits
/// samples declared a TYPE first.
fn assert_exposition_is_well_formed(body: &str) {
    let mut declared: Vec<String> = Vec::new();

    for line in body.lines() {
        if line.is_empty() {
            continue;
        }
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let mut parts = rest.split(' ');
            let name = parts.next().expect("type line names a metric");
            let kind = parts.next().expect("type line names a kind");
            assert!(
                matches!(kind, "counter" | "gauge" | "histogram" | "summary"),
                "unknown metric type {kind}"
            );
            declared.push(name.to_string());
            continue;
        }
        if line.starts_with('#') {
            continue;
        }

        let (series, value) = line.rsplit_once(' ').expect("sample has a value");
        value
            .parse::<f64>()
            .map(|_| ())
            .or_else(|_| match value {
                "+Inf" | "-Inf" | "NaN" => Ok(()),
                other => Err(other),
            })
            .unwrap_or_else(|other| panic!("unparseable sample value {other} in {line}"));

        let name = series.split_once('{').map(|(n, _)| n).unwrap_or(series);
        let base = name
            .strip_suffix("_bucket")
            .or_else(|| name.strip_suffix("_count"))
            .or_else(|| name.strip_suffix("_sum"))
            .unwrap_or(name);
        assert!(
            declared.iter().any(|d| d == name || d == base),
            "sample {name} appeared before its # TYPE line"
        );

        if let Some((_, labels)) = series.split_once('{') {
            let labels = labels.strip_suffix('}').expect("label set closes");
            assert!(
                labels.matches('"').count().is_multiple_of(2),
                "unbalanced quotes in {labels}"
            );
        }
    }

    assert!(!declared.is_empty(), "no metrics were declared");
}
