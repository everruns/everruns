//! Brave Search API driver.

use async_trait::async_trait;
use everruns_contracts::runtime::web_search::{
    SearchContext, SearchDriver, SearchDriverCapabilities, SearchFailure, SearchFailureKind,
    SearchFreshness, SearchHit, SearchRequest, SearchResponse,
};
use serde_json::Value;

use super::http;

const BRAVE_API_BASE: &str = "https://api.search.brave.com/res/v1";

/// Brave Search with one API key. The key travels only in the
/// `X-Subscription-Token` header, so recorded exchanges never contain it.
pub struct BraveSearchDriver {
    api_key: String,
    api_base: String,
}

impl BraveSearchDriver {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_base_url(api_key, BRAVE_API_BASE)
    }

    /// A driver against a custom endpoint, for tests.
    pub fn with_base_url(api_key: impl Into<String>, api_base: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            api_base: api_base.into(),
        }
    }
}

#[async_trait]
impl SearchDriver for BraveSearchDriver {
    fn id(&self) -> &str {
        "brave"
    }

    fn label(&self) -> &str {
        "Brave Search"
    }

    fn capabilities(&self) -> SearchDriverCapabilities {
        SearchDriverCapabilities {
            freshness: true,
            domain_filters: false,
            excerpts: true,
        }
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        &["api.search.brave.com"]
    }

    async fn search(
        &self,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure> {
        let mut query = request.query.clone();
        if let [domain] = request.allowed_domains.as_slice() {
            query.push_str(&format!(" site:{domain}"));
        }
        let mut url = format!(
            "{}/web/search?q={}&count={}&extra_snippets=true",
            self.api_base,
            http::encode(&query),
            request.limit.clamp(1, 20)
        );
        if let Some(freshness) = request.freshness {
            url.push_str(match freshness {
                SearchFreshness::Day => "&freshness=pd",
                SearchFreshness::Week => "&freshness=pw",
                SearchFreshness::Month => "&freshness=pm",
                SearchFreshness::Year => "&freshness=py",
            });
        }
        let (exchange, response) = http::get(
            context,
            &url,
            &[
                ("X-Subscription-Token", self.api_key.as_str()),
                ("Accept", "application/json"),
            ],
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
                format!("Brave Search returned unreadable JSON: {error}"),
            )
            .with_exchanges(vec![exchange.clone()])
        })?;
        let mut hits: Vec<SearchHit> = json
            .pointer("/web/results")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_result)
            .collect();
        hits.retain(|hit| request.url_passes_domain_filters(&hit.url));
        hits.truncate(request.limit as usize);
        Ok(SearchResponse {
            hits,
            answer: None,
            notes: Vec::new(),
            exchanges: vec![exchange],
        })
    }
}

fn parse_result(item: &Value) -> Option<SearchHit> {
    let mut hit = SearchHit::new(
        super::html::text(item.get("title")?.as_str()?),
        item.get("url")?.as_str()?,
    );
    hit.snippet = item
        .get("description")
        .and_then(Value::as_str)
        .map(super::html::text)
        .filter(|text| !text.is_empty());
    hit.published = item
        .get("page_age")
        .and_then(Value::as_str)
        .and_then(|age| age.get(..10))
        .map(str::to_string)
        .or_else(|| item.get("age").and_then(Value::as_str).map(str::to_string));
    let extra: Vec<String> = item
        .get("extra_snippets")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(super::html::text)
        .collect();
    hit.excerpt = (!extra.is_empty()).then(|| extra.join(" … "));
    Some(hit)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::web_search::http::DirectEgress;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn context() -> SearchContext {
        SearchContext {
            egress: Arc::new(DirectEgress::new()),
            network_access: None,
        }
    }

    #[tokio::test]
    async fn maps_results_with_excerpts_and_keeps_the_key_out_of_the_record() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/web/search"))
            .and(header("X-Subscription-Token", "secret-key"))
            .and(query_param("freshness", "pw"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": {"results": [{
                    "title": "Announcing <strong>Tokio</strong> 1.50",
                    "url": "https://tokio.rs/blog/2026-10-tokio-1-50",
                    "description": "Tokio 1.50 is out.",
                    "page_age": "2026-10-02T08:00:00",
                    "extra_snippets": ["New scheduler.", "Faster timers."]
                }]}
            })))
            .mount(&server)
            .await;
        let driver = BraveSearchDriver::with_base_url("secret-key", server.uri());
        let mut request = SearchRequest::new("tokio 1.50");
        request.freshness = Some(SearchFreshness::Week);

        let response = driver.search(&request, &context()).await.unwrap();
        let hit = &response.hits[0];
        assert_eq!(hit.title, "Announcing Tokio 1.50");
        assert_eq!(hit.site, "tokio.rs");
        assert_eq!(hit.published.as_deref(), Some("2026-10-02"));
        assert_eq!(
            hit.excerpt.as_deref(),
            Some("New scheduler. … Faster timers.")
        );
        let recorded = serde_json::to_string(&response.exchanges).unwrap();
        assert!(!recorded.contains("secret-key"));
    }

    #[tokio::test]
    async fn rate_limit_and_bad_key_are_typed_failures() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(429).set_body_string("slow down"))
            .mount(&server)
            .await;
        let failure = BraveSearchDriver::with_base_url("k", server.uri())
            .search(&SearchRequest::new("q"), &context())
            .await
            .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::RateLimited);
        assert_eq!(failure.exchanges[0].body.as_deref(), Some("slow down"));

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let failure = BraveSearchDriver::with_base_url("k", server.uri())
            .search(&SearchRequest::new("q"), &context())
            .await
            .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::Unauthorized);
        assert_eq!(failure.message, "Brave Search rejected the key (401)");
    }
}
