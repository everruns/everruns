//! Cloudflare Web Search API driver.
//!
//! Decision: one Cloudflare account gives three search indexes (Ceramic, Exa,
//! Linkup) behind one endpoint, billed to the account's AI Gateway credits or
//! to a provider key stored on the gateway (`byokAlias`). The index is a
//! setting of the search account, not of the agent: an organization that
//! wants two indexes connects two accounts, so Automatic and the agent picker
//! keep treating each account as one provider.
//!
//! Decision: the account id, gateway, index and key alias are not secrets.
//! They live in the connection's `provider_metadata` and reach the worker as
//! the search account's `options`; only the API token is encrypted.
//!
//! The API has no date or domain filters and returns at most 10 results, so
//! blocked and allowed domains are applied to the results, and a freshness
//! request is reported in the notes rather than silently dropped. Ceramic
//! returns descriptions of up to 8,000 characters; they are cut to an excerpt.

use async_trait::async_trait;
use everruns_contracts::runtime::web_search::{
    SearchContext, SearchDriver, SearchDriverCapabilities, SearchFailure, SearchFailureKind,
    SearchHit, SearchRequest, SearchResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::http;

pub(super) const CLOUDFLARE_API_BASE: &str = "https://api.cloudflare.com";

/// The API's own caps: query length in characters and results per request.
const MAX_QUERY_CHARS: usize = 1024;
const MAX_RESULTS: u32 = 10;
/// Characters of a description shown as the snippet and kept as the excerpt.
const SNIPPET_CHARS: usize = 300;
const EXCERPT_CHARS: usize = 1500;

/// Which search index answers through Cloudflare.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CloudflareSearchIndex {
    /// Ceramic.ai, $0.25 per 1,000 searches, zero data retention.
    #[default]
    Ceramic,
    /// Exa, $7 per 1,000 searches, no zero data retention.
    Exa,
    /// Linkup, $5 per 1,000 searches, zero data retention.
    Linkup,
}

impl CloudflareSearchIndex {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ceramic => "ceramic",
            Self::Exa => "exa",
            Self::Linkup => "linkup",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Ceramic => "Cloudflare (Ceramic)",
            Self::Exa => "Cloudflare (Exa)",
            Self::Linkup => "Cloudflare (Linkup)",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value.trim().to_ascii_lowercase().as_str() {
            "" | "ceramic" => Ok(Self::Ceramic),
            "exa" => Ok(Self::Exa),
            "linkup" => Ok(Self::Linkup),
            other => Err(format!(
                "Unknown search index {other:?}; use ceramic, exa or linkup"
            )),
        }
    }
}

/// The non-secret settings of a Cloudflare search account.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudflareSearchOptions {
    /// Cloudflare account id (32 hex characters).
    pub account_id: String,
    /// AI Gateway the searches run through; `default` when not set.
    #[serde(default = "default_gateway")]
    pub gateway_id: String,
    #[serde(default)]
    pub index: CloudflareSearchIndex,
    /// Alias of a provider key stored on the gateway. Without it the gateway
    /// uses its `default` key for the index, or else AI Gateway credits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byok_alias: Option<String>,
}

fn default_gateway() -> String {
    "default".to_string()
}

impl CloudflareSearchOptions {
    /// Options for an account and index on the `default` gateway.
    pub fn new(account_id: impl Into<String>, index: CloudflareSearchIndex) -> Self {
        Self {
            account_id: account_id.into(),
            gateway_id: default_gateway(),
            index,
            byok_alias: None,
        }
    }

    /// Build from the connect form's fields, applying defaults to empty ones.
    pub fn from_fields(
        account_id: &str,
        gateway_id: Option<&str>,
        index: Option<&str>,
        byok_alias: Option<&str>,
    ) -> Result<Self, String> {
        let trimmed = |value: Option<&str>| {
            value
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Self {
            account_id: account_id.trim().to_ascii_lowercase(),
            gateway_id: trimmed(gateway_id).unwrap_or_else(default_gateway),
            index: CloudflareSearchIndex::parse(index.unwrap_or_default())?,
            byok_alias: trimmed(byok_alias),
        }
        .validated()
    }

    /// Parse and check stored options (a search account's `options`).
    pub fn from_value(value: &Value) -> Result<Self, String> {
        serde_json::from_value::<Self>(value.clone())
            .map_err(|error| format!("Invalid Cloudflare search settings: {error}"))?
            .validated()
    }

    /// The account id and gateway go into the request path, so both are held
    /// to the characters Cloudflare issues before any request is built.
    fn validated(self) -> Result<Self, String> {
        if self.account_id.len() != 32 || !self.account_id.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(
                "The Cloudflare account id is 32 hexadecimal characters (dashboard URL or Overview page)"
                    .to_string(),
            );
        }
        if !is_alias(&self.gateway_id) {
            return Err(
                "The AI Gateway id uses letters, digits, '-' and '_' (up to 64)".to_string(),
            );
        }
        if self
            .byok_alias
            .as_deref()
            .is_some_and(|alias| !is_alias(alias))
        {
            return Err("The key alias uses letters, digits, '-' and '_' (up to 64)".to_string());
        }
        Ok(self)
    }
}

fn is_alias(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The Cloudflare Web Search API with one API token (Workers AI Read and AI
/// Gateway Read). The token travels only in the `Authorization` header, so
/// recorded exchanges never contain it.
pub struct CloudflareSearchDriver {
    api_token: String,
    options: CloudflareSearchOptions,
    api_base: String,
}

impl CloudflareSearchDriver {
    pub fn new(api_token: impl Into<String>, options: CloudflareSearchOptions) -> Self {
        Self::with_base_url(api_token, options, CLOUDFLARE_API_BASE)
    }

    /// A driver against a custom endpoint, for tests.
    pub fn with_base_url(
        api_token: impl Into<String>,
        options: CloudflareSearchOptions,
        api_base: impl Into<String>,
    ) -> Self {
        Self {
            api_token: api_token.into(),
            options,
            api_base: api_base.into(),
        }
    }

    fn body(&self, request: &SearchRequest) -> Value {
        let query: String = request.query.chars().take(MAX_QUERY_CHARS).collect();
        let mut body = json!({
            "query": query,
            "provider": self.options.index.as_str(),
            "limit": request.limit.clamp(1, MAX_RESULTS),
            "options": {"gateway": {"id": self.options.gateway_id}},
        });
        if let Some(alias) = &self.options.byok_alias {
            body["byokAlias"] = json!(alias);
        }
        body
    }
}

#[async_trait]
impl SearchDriver for CloudflareSearchDriver {
    fn id(&self) -> &str {
        "cloudflare"
    }

    fn label(&self) -> &str {
        self.options.index.label()
    }

    fn capabilities(&self) -> SearchDriverCapabilities {
        SearchDriverCapabilities {
            freshness: false,
            domain_filters: false,
            excerpts: true,
        }
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        &["api.cloudflare.com"]
    }

    async fn search(
        &self,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure> {
        let url = format!(
            "{}/client/v4/accounts/{}/ai/websearch/",
            self.api_base, self.options.account_id
        );
        let authorization = format!("Bearer {}", self.api_token);
        let (exchange, response) = http::post_json(
            context,
            &url,
            &[
                ("Authorization", authorization.as_str()),
                ("Accept", "application/json"),
            ],
            &self.body(request),
        )
        .await;
        let response =
            response.map_err(|failure| failure.with_exchanges(vec![exchange.clone()]))?;
        if let Some(mut failure) = http::status_failure(self.label(), response.status) {
            if let Some(detail) = error_message(&response.body) {
                failure.message = format!("{}: {detail}", failure.message);
            }
            return Err(failure.with_exchanges(vec![exchange]));
        }
        let json: Value = serde_json::from_slice(&response.body).map_err(|error| {
            SearchFailure::new(
                SearchFailureKind::Unavailable,
                format!("Cloudflare returned unreadable JSON: {error}"),
            )
            .with_exchanges(vec![exchange.clone()])
        })?;
        // The REST API answers with the result itself; tolerate the usual
        // `{"success", "result"}` envelope too.
        let result = json
            .get("result")
            .filter(|result| result.is_object())
            .unwrap_or(&json);
        let mut hits: Vec<SearchHit> = result
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(parse_item)
            .collect();
        hits.retain(|hit| request.url_passes_domain_filters(&hit.url));
        hits.truncate(request.limit as usize);
        let mut notes = Vec::new();
        if request.freshness.is_some() {
            notes.push(
                "Cloudflare Web Search has no date filter; results are not limited to recent pages"
                    .to_string(),
            );
        }
        Ok(SearchResponse {
            hits,
            answer: None,
            notes,
            exchanges: vec![exchange],
        })
    }
}

fn parse_item(item: &Value) -> Option<SearchHit> {
    let url = item.get("url")?.as_str()?;
    let title = item
        .get("title")
        .and_then(Value::as_str)
        .filter(|title| !title.trim().is_empty())
        .unwrap_or(url);
    let mut hit = SearchHit::new(title.trim(), url);
    hit.published = ["publishedDate", "published_date", "date"]
        .iter()
        .find_map(|key| item.get(*key).and_then(Value::as_str))
        .and_then(|date| date.get(..10))
        .map(str::to_string);
    let description = item
        .get("description")
        .and_then(Value::as_str)
        .map(|text| text.split_whitespace().collect::<Vec<_>>().join(" "))
        .filter(|text| !text.is_empty());
    if let Some(description) = description {
        hit.snippet = Some(cut(&description, SNIPPET_CHARS));
        if description.chars().count() > SNIPPET_CHARS {
            hit.excerpt = Some(cut(&description, EXCERPT_CHARS));
        }
    }
    Some(hit)
}

fn cut(text: &str, chars: usize) -> String {
    match text.char_indices().nth(chars) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

/// The message of a Cloudflare error: the Web Search API's
/// `{"ok": false, "error": {"code", "message"}}`, or the v4 API envelope
/// `{"success": false, "errors": [{"code", "message"}]}`.
fn error_message(body: &[u8]) -> Option<String> {
    let json: Value = serde_json::from_slice(body).ok()?;
    let message = json
        .get("error")
        .and_then(|error| error.get("message"))
        .and_then(Value::as_str)
        .or_else(|| {
            json.get("errors")?
                .as_array()?
                .iter()
                .find_map(|error| error.get("message")?.as_str())
        })?;
    Some(cut(message.trim(), 200)).filter(|message| !message.is_empty())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::web_search::http::DirectEgress;
    use everruns_contracts::runtime::web_search::SearchFreshness;
    use wiremock::matchers::{body_json, header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ACCOUNT: &str = "0123456789abcdef0123456789abcdef";

    fn context() -> SearchContext {
        SearchContext {
            egress: Arc::new(DirectEgress::new()),
            network_access: None,
        }
    }

    fn driver(server: &MockServer, options: CloudflareSearchOptions) -> CloudflareSearchDriver {
        CloudflareSearchDriver::with_base_url("secret-token", options, server.uri())
    }

    #[tokio::test]
    async fn maps_items_and_keeps_the_token_out_of_the_record() {
        let server = MockServer::builder().start().await;
        let long = "Tokio 1.50 is out. ".repeat(100);
        Mock::given(method("POST"))
            .and(path(format!("/client/v4/accounts/{ACCOUNT}/ai/websearch/")))
            .and(header("authorization", "Bearer secret-token"))
            .and(body_json(json!({
                "query": "tokio 1.50",
                "provider": "exa",
                "limit": 5,
                "options": {"gateway": {"id": "research"}},
                "byokAlias": "team-exa"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "items": [
                    {
                        "url": "https://tokio.rs/blog/2026-10-tokio-1-50",
                        "title": "Announcing Tokio 1.50",
                        "description": long,
                        "publishedDate": "2026-10-02T00:00:00Z"
                    },
                    {"url": "https://www.pinterest.com/pin/1", "title": "Pin"},
                    {"title": "no url"}
                ],
                "metadata": {"query": "tokio 1.50", "requestId": "r1", "latencyMs": 612}
            })))
            .mount(&server)
            .await;
        let options = CloudflareSearchOptions::from_fields(
            ACCOUNT,
            Some("research"),
            Some("Exa"),
            Some("team-exa"),
        )
        .unwrap();
        let driver = driver(&server, options);
        assert_eq!(driver.label(), "Cloudflare (Exa)");
        let mut request = SearchRequest::new("tokio 1.50");
        request.limit = 5;
        request.blocked_domains = vec!["pinterest.com".into()];

        let response = driver.search(&request, &context()).await.unwrap();
        assert_eq!(
            response.hits.len(),
            1,
            "blocked domains and url-less items are dropped"
        );
        let hit = &response.hits[0];
        assert_eq!(hit.title, "Announcing Tokio 1.50");
        assert_eq!(hit.site, "tokio.rs");
        assert_eq!(hit.published.as_deref(), Some("2026-10-02"));
        assert!(
            hit.snippet
                .as_deref()
                .unwrap()
                .starts_with("Tokio 1.50 is out.")
        );
        assert_eq!(
            hit.snippet.as_deref().unwrap().chars().count(),
            SNIPPET_CHARS + 1
        );
        assert_eq!(
            hit.excerpt.as_deref().unwrap().chars().count(),
            EXCERPT_CHARS + 1
        );
        assert!(response.notes.is_empty());
        let recorded = serde_json::to_string(&response.exchanges).unwrap();
        assert!(!recorded.contains("secret-token"));
    }

    #[test]
    fn body_caps_limit_and_query_and_omits_an_unset_alias() {
        let driver = CloudflareSearchDriver::new(
            "t",
            CloudflareSearchOptions::new(ACCOUNT, CloudflareSearchIndex::Ceramic),
        );
        let mut request = SearchRequest::new("x".repeat(2000));
        request.limit = 50;
        let body = driver.body(&request);
        assert_eq!(body["limit"], 10);
        assert_eq!(body["provider"], "ceramic");
        assert_eq!(body["options"]["gateway"]["id"], "default");
        assert_eq!(body["query"].as_str().unwrap().len(), MAX_QUERY_CHARS);
        assert!(body.get("byokAlias").is_none());
    }

    #[tokio::test]
    async fn enveloped_results_and_freshness_note() {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "success": true,
                "result": {"items": [{"url": "https://docs.rs/tokio", "title": "tokio", "description": "Docs"}]}
            })))
            .mount(&server)
            .await;
        let mut request = SearchRequest::new("tokio");
        request.freshness = Some(SearchFreshness::Week);
        let response = driver(
            &server,
            CloudflareSearchOptions::new(ACCOUNT, Default::default()),
        )
        .search(&request, &context())
        .await
        .unwrap();
        assert_eq!(response.hits[0].snippet.as_deref(), Some("Docs"));
        assert!(
            response.hits[0].excerpt.is_none(),
            "short descriptions are only a snippet"
        );
        assert!(response.notes[0].contains("no date filter"));
    }

    #[tokio::test]
    async fn errors_carry_cloudflare_message_and_typed_kind() {
        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(400).set_body_json(json!({
                "ok": false,
                "error": {
                    "category": "credential",
                    "code": "web_search_byok_not_configured",
                    "status": 400,
                    "message": "No provider key is configured on this gateway for the requested provider."
                }
            })))
            .mount(&server)
            .await;
        let failure = driver(
            &server,
            CloudflareSearchOptions::new(ACCOUNT, Default::default()),
        )
        .search(&SearchRequest::new("q"), &context())
        .await
        .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::Unavailable);
        assert_eq!(
            failure.message,
            "Cloudflare (Ceramic) returned an error (400): No provider key is configured on this gateway for the requested provider."
        );
        assert_eq!(
            error_message(br#"{"success": false, "errors": [{"code": 10000, "message": "Authentication error"}]}"#)
                .as_deref(),
            Some("Authentication error")
        );

        let server = MockServer::builder().start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(403).set_body_string("forbidden"))
            .mount(&server)
            .await;
        let failure = driver(
            &server,
            CloudflareSearchOptions::new(ACCOUNT, Default::default()),
        )
        .search(&SearchRequest::new("q"), &context())
        .await
        .unwrap_err();
        assert_eq!(failure.kind, SearchFailureKind::Unauthorized);
        assert_eq!(
            failure.message,
            "Cloudflare (Ceramic) rejected the key (403)"
        );
    }

    #[test]
    fn options_reject_values_that_would_change_the_request_path() {
        assert!(CloudflareSearchOptions::from_fields("not-an-account", None, None, None).is_err());
        assert!(
            CloudflareSearchOptions::from_fields(ACCOUNT, Some("../zones"), None, None).is_err()
        );
        assert!(CloudflareSearchOptions::from_fields(ACCOUNT, None, Some("bing"), None).is_err());
        assert!(CloudflareSearchOptions::from_fields(ACCOUNT, None, None, Some("a b")).is_err());
        let stored = json!({"account_id": ACCOUNT, "index": "bing"});
        assert!(
            CloudflareSearchOptions::from_value(&stored).is_err(),
            "stored options are checked as stored"
        );
        let options = CloudflareSearchOptions::from_fields(
            &ACCOUNT.to_uppercase(),
            Some(" "),
            Some(""),
            None,
        )
        .unwrap();
        assert_eq!(
            options,
            CloudflareSearchOptions::new(ACCOUNT, CloudflareSearchIndex::Ceramic)
        );
        let round_trip =
            CloudflareSearchOptions::from_value(&serde_json::to_value(&options).unwrap()).unwrap();
        assert_eq!(round_trip, options);
    }
}
