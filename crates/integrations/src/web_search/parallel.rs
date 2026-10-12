//! Parallel Search API driver.
//!
//! Decision: `fast` mode. It is Parallel's recommended mode for tool-calling
//! agents (about 700 ms, $1 per 1,000 requests); `advanced` takes seconds per
//! call, which a model that searches several times per turn feels. The query
//! goes in as both the objective and the one search query, so Parallel ranks
//! for what the model asked rather than for a rewritten keyword string.

use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use everruns_contracts::runtime::web_search::{
    SearchContext, SearchDriver, SearchDriverCapabilities, SearchFailure, SearchFailureKind,
    SearchFreshness, SearchHit, SearchRequest, SearchResponse,
};
use serde_json::{Value, json};

use super::http;

pub(super) const PARALLEL_API_BASE: &str = "https://api.parallel.ai";

/// Characters of excerpt text kept per result: enough to quote, small enough
/// that 20 results stay a modest tool result.
const MAX_CHARS_PER_RESULT: u32 = 1500;

/// Parallel's Search API with one API key. The key travels only in the
/// `x-api-key` header, so recorded exchanges never contain it.
pub struct ParallelSearchDriver {
    api_key: String,
    api_base: String,
}

impl ParallelSearchDriver {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, PARALLEL_API_BASE)
    }

    /// A driver against a custom endpoint, for tests.
    pub fn with_base_url(api_key: impl Into<String>, api_base: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_base: api_base.into(),
        }
    }

    fn body(request: &SearchRequest) -> Value {
        let mut source_policy = serde_json::Map::new();
        // Parallel ignores exclusions when inclusions are set; blocked
        // domains are still applied to the results below.
        if !request.allowed_domains.is_empty() {
            source_policy.insert("include_domains".into(), json!(request.allowed_domains));
        } else if !request.blocked_domains.is_empty() {
            source_policy.insert("exclude_domains".into(), json!(request.blocked_domains));
        }
        if let Some(freshness) = request.freshness {
            source_policy.insert(
                "after_date".into(),
                json!(after_date(freshness, now_secs())),
            );
        }
        let mut advanced = json!({
            "max_results": request.limit.clamp(1, 20),
            "excerpt_settings": {"max_chars_per_result": MAX_CHARS_PER_RESULT},
        });
        if !source_policy.is_empty() {
            advanced["source_policy"] = Value::Object(source_policy);
        }
        json!({
            "mode": "fast",
            "objective": request.query,
            "search_queries": [request.query],
            "advanced_settings": advanced,
        })
    }
}

#[async_trait]
impl SearchDriver for ParallelSearchDriver {
    fn id(&self) -> &str {
        "parallel"
    }

    fn label(&self) -> &str {
        "Parallel"
    }

    fn capabilities(&self) -> SearchDriverCapabilities {
        SearchDriverCapabilities {
            freshness: true,
            domain_filters: true,
            excerpts: true,
        }
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        &["api.parallel.ai"]
    }

    async fn search(
        &self,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure> {
        let url = format!("{}/v1/search", self.api_base);
        let (exchange, response) = http::post_json(
            context,
            &url,
            &[
                ("x-api-key", self.api_key.as_str()),
                ("Accept", "application/json"),
            ],
            &Self::body(request),
        )
        .await;
        let response =
            response.map_err(|failure| failure.with_exchanges(vec![exchange.clone()]))?;
        if let Some(failure) = http::status_failure(self.label(), response.status) {
            return Err(failure.with_exchanges(vec![exchange]));
        }
        let json: Value = serde_json::from_slice(&response.body).map_err(|error| {
            SearchFailure::new(
                SearchFailureKind::Unavailable,
                format!("Parallel returned unreadable JSON: {error}"),
            )
            .with_exchanges(vec![exchange.clone()])
        })?;
        let mut hits: Vec<SearchHit> = json
            .get("results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_result)
            .collect();
        hits.retain(|hit| request.url_passes_domain_filters(&hit.url));
        hits.truncate(request.limit as usize);
        let notes = json
            .get("warnings")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|warning| {
                warning
                    .get("message")
                    .and_then(Value::as_str)
                    .or_else(|| warning.as_str())
            })
            .map(|message| format!("Parallel: {message}"))
            .collect();
        Ok(SearchResponse {
            hits,
            answer: None,
            notes,
            exchanges: vec![exchange],
        })
    }
}

fn parse_result(item: &Value) -> Option<SearchHit> {
    let url = item.get("url")?.as_str()?;
    let title = item
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(url);
    let mut hit = SearchHit::new(title.trim(), url);
    hit.published = item
        .get("publish_date")
        .and_then(Value::as_str)
        .and_then(|date| date.get(..10))
        .map(str::to_string);
    let excerpts: Vec<&str> = item
        .get("excerpts")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .collect();
    hit.snippet = excerpts.first().map(|text| first_line(text));
    hit.excerpt = (!excerpts.is_empty()).then(|| excerpts.join("\n…\n"));
    Some(hit)
}

/// The first non-heading line of an excerpt, cut to snippet length.
/// Parallel prefixes excerpts with "Section Title: …" lines.
fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with("Section Title:") && *line != "Content:")
        .unwrap_or(text);
    match line.char_indices().nth(300) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_string(),
    }
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// The earliest publication date (`YYYY-MM-DD`, UTC) for a freshness window.
fn after_date(freshness: SearchFreshness, now_secs: i64) -> String {
    let days = match freshness {
        SearchFreshness::Day => 1,
        SearchFreshness::Week => 7,
        SearchFreshness::Month => 31,
        SearchFreshness::Year => 365,
    };
    let (year, month, day) = civil_from_days(now_secs.div_euclid(86_400) - days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::web_search::http::DirectEgress;
    use wiremock::matchers::{body_partial_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn context() -> SearchContext {
        SearchContext {
            egress: Arc::new(DirectEgress::new()),
            network_access: None,
        }
    }

    #[tokio::test]
    async fn maps_results_with_excerpts_and_keeps_the_key_out_of_the_record() {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .and(path("/v1/search"))
            .and(header("x-api-key", "secret-key"))
            .and(body_partial_json(json!({
                "mode": "fast",
                "objective": "tokio 1.50",
                "search_queries": ["tokio 1.50"],
                "advanced_settings": {
                    "max_results": 5,
                    "source_policy": {"exclude_domains": ["pinterest.com"]}
                }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "search_id": "search_1",
                "results": [
                    {
                        "url": "https://tokio.rs/blog/2026-10-tokio-1-50",
                        "title": "Announcing Tokio 1.50",
                        "publish_date": "2026-10-02",
                        "excerpts": [
                            "Section Title: Announcing Tokio 1.50\nContent:\nTokio 1.50 is out.",
                            "Faster timers."
                        ]
                    },
                    {"url": "https://www.pinterest.com/pin/1", "title": "Pin", "excerpts": []}
                ],
                "warnings": [{"type": "input_validation", "message": "query was shortened"}]
            })))
            .mount(&server)
            .await;
        let driver = ParallelSearchDriver::with_base_url("secret-key", server.uri());
        let mut request = SearchRequest::new("tokio 1.50");
        request.limit = 5;
        request.blocked_domains = vec!["pinterest.com".into()];

        let response = driver.search(&request, &context()).await.unwrap();
        assert_eq!(response.hits.len(), 1, "blocked domains are filtered");
        let hit = &response.hits[0];
        assert_eq!(hit.title, "Announcing Tokio 1.50");
        assert_eq!(hit.site, "tokio.rs");
        assert_eq!(hit.published.as_deref(), Some("2026-10-02"));
        assert_eq!(hit.snippet.as_deref(), Some("Tokio 1.50 is out."));
        assert!(hit.excerpt.as_deref().unwrap().ends_with("Faster timers."));
        assert_eq!(response.notes, ["Parallel: query was shortened"]);
        assert_eq!(response.exchanges[0].method, "POST");
        let recorded = serde_json::to_string(&response.exchanges).unwrap();
        assert!(!recorded.contains("secret-key"));
    }

    #[test]
    fn allowed_domains_win_and_freshness_becomes_a_start_date() {
        let mut request = SearchRequest::new("q");
        request.limit = 50;
        request.allowed_domains = vec!["docs.rs".into()];
        request.blocked_domains = vec!["pinterest.com".into()];
        request.freshness = Some(SearchFreshness::Week);
        let body = ParallelSearchDriver::body(&request);
        let policy = &body["advanced_settings"]["source_policy"];
        assert_eq!(policy["include_domains"], json!(["docs.rs"]));
        assert!(policy.get("exclude_domains").is_none());
        assert_eq!(policy["after_date"].as_str().unwrap().len(), 10);
        assert_eq!(body["advanced_settings"]["max_results"], 20);
    }

    #[test]
    fn freshness_dates() {
        // 2026-10-12T15:00:00Z
        let now = 1_791_817_200;
        assert_eq!(after_date(SearchFreshness::Day, now), "2026-10-11");
        assert_eq!(after_date(SearchFreshness::Week, now), "2026-10-05");
        assert_eq!(after_date(SearchFreshness::Year, now), "2025-10-12");
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }

    #[tokio::test]
    async fn rate_limit_and_bad_key_are_typed_failures() {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .mount(&server)
            .await;
        let failure = ParallelSearchDriver::with_base_url("k", server.uri())
            .search(&SearchRequest::new("q"), &context())
            .await
            .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::RateLimited);
        assert_eq!(failure.exchanges[0].body.as_deref(), Some("slow down"));

        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let failure = ParallelSearchDriver::with_base_url("k", server.uri())
            .search(&SearchRequest::new("q"), &context())
            .await
            .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::Unauthorized);
        assert_eq!(failure.message, "Parallel rejected the key (401)");
    }
}
