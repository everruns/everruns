//! Provider-neutral web search contract.
//!
//! One model-facing tool, `web_search`, answers from a *search plan*: an
//! ordered list of providers tried until one succeeds. Function-executed
//! providers (free search, Brave, Parallel) implement [`SearchDriver`]; native
//! model provider search is a driver option on the LLM call instead, because
//! the model provider runs it inside the generation.

use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::runtime::egress::EgressService;
use crate::runtime::network_access::NetworkAccessList;

/// Canonical capability id and tool name.
pub const WEB_SEARCH_CAPABILITY_ID: &str = "web_search";
/// The single model-facing tool every provider answers through.
pub const WEB_SEARCH_TOOL_NAME: &str = "web_search";
/// Largest number of results one call returns.
pub const WEB_SEARCH_MAX_RESULTS: u32 = 20;
/// Results per call when neither the model nor the agent sets a limit.
pub const WEB_SEARCH_DEFAULT_RESULTS: u32 = 8;
/// Cap on each raw provider response kept for the details view.
pub const WEB_SEARCH_RAW_CAP_BYTES: usize = 256 * 1024;

/// How recent results must be.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchFreshness {
    Day,
    Week,
    Month,
    Year,
}

/// What a query is about. Only free search acts on it: `Dev` also asks
/// GitHub, npm and crates.io.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchTopic {
    #[default]
    General,
    Dev,
}

/// One search, already merged with the agent's settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchRequest {
    pub query: String,
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<SearchFreshness>,
    #[serde(default)]
    pub topic: SearchTopic,
    /// Only results on these sites (and their subdomains).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    /// Never results on these sites (and their subdomains).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_domains: Vec<String>,
}

impl SearchRequest {
    pub fn new(query: impl Into<String>) -> Self {
        Self {
            query: query.into(),
            limit: WEB_SEARCH_DEFAULT_RESULTS,
            freshness: None,
            topic: SearchTopic::General,
            allowed_domains: Vec::new(),
            blocked_domains: Vec::new(),
        }
    }

    /// Whether a URL passes the request's domain filters.
    pub fn url_passes_domain_filters(&self, url: &str) -> bool {
        let Some(host) = url_host(url) else {
            return false;
        };
        if self
            .blocked_domains
            .iter()
            .any(|domain| host_matches(&host, domain))
        {
            return false;
        }
        self.allowed_domains.is_empty()
            || self
                .allowed_domains
                .iter()
                .any(|domain| host_matches(&host, domain))
    }
}

/// One result, in the shape the model cites from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    /// Registrable-looking host without `www.`, for "on tokio.rs".
    pub site: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
    /// Longer quotable text, when the provider returns one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub excerpt: Option<String>,
    /// Which source inside the provider found it (free search mixes several).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

impl SearchHit {
    pub fn new(title: impl Into<String>, url: impl Into<String>) -> Self {
        let url = url.into();
        Self {
            title: title.into(),
            site: site_of(&url),
            url,
            published: None,
            snippet: None,
            excerpt: None,
            source: None,
        }
    }
}

/// A direct answer shown above the result list (an encyclopedia abstract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchAnswer {
    pub text: String,
    pub source: String,
    pub url: String,
}

/// One HTTP exchange a driver made, kept verbatim for the raw view.
///
/// Drivers put credentials in headers, never in `url`, and the exchange keeps
/// no headers, so nothing secret is stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchExchange {
    pub method: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Set when `body` was cut at [`WEB_SEARCH_RAW_CAP_BYTES`].
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SearchExchange {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            method: "GET".into(),
            url: url.into(),
            status: None,
            body: None,
            truncated: false,
            error: None,
        }
    }

    /// Record the response body, capped for storage.
    pub fn with_body(mut self, status: u16, body: &[u8]) -> Self {
        self.status = Some(status);
        let cut = body.len().min(WEB_SEARCH_RAW_CAP_BYTES);
        self.truncated = cut < body.len();
        self.body = Some(String::from_utf8_lossy(&body[..cut]).into_owned());
        self
    }

    pub fn with_error(mut self, error: impl Into<String>) -> Self {
        self.error = Some(error.into());
        self
    }
}

/// What a driver found.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<SearchAnswer>,
    /// Plain notes for people and, with `show_provider`, the model.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exchanges: Vec<SearchExchange>,
}

/// Why a driver could not answer. Every kind moves the plan to its next
/// provider unless the agent turned fallback off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchFailureKind {
    /// No key or account for this provider.
    NotConfigured,
    /// The provider rejected the key.
    Unauthorized,
    RateLimited,
    /// The provider answered with a bot check or a page we cannot read.
    Blocked,
    /// The egress policy refused the request.
    NetworkDenied,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchFailure {
    pub kind: SearchFailureKind,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exchanges: Vec<SearchExchange>,
}

impl SearchFailure {
    pub fn new(kind: SearchFailureKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            exchanges: Vec::new(),
        }
    }

    pub fn with_exchanges(mut self, exchanges: Vec<SearchExchange>) -> Self {
        self.exchanges = exchanges;
        self
    }
}

impl std::fmt::Display for SearchFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// What a driver can honor natively; anything else is applied after the fact
/// (domain filters) or ignored with a note (freshness).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchDriverCapabilities {
    pub freshness: bool,
    pub domain_filters: bool,
    pub excerpts: bool,
}

/// Everything a driver needs from the host for one search.
#[derive(Clone)]
pub struct SearchContext {
    /// The host's outbound boundary; search traffic obeys the same egress
    /// policy as `web_fetch`.
    pub egress: Arc<dyn EgressService>,
    /// The agent's own network access list, applied on top of system policy.
    pub network_access: Option<NetworkAccessList>,
}

/// A function-executed search provider.
#[async_trait]
pub trait SearchDriver: Send + Sync {
    /// Stable provider id (`free`, `brave`, `parallel`).
    fn id(&self) -> &str;

    /// Name people see ("Free search", "Brave Search").
    fn label(&self) -> &str;

    fn capabilities(&self) -> SearchDriverCapabilities;

    /// Every host this driver may call. A test checks each against the
    /// system allowlist so search is never blocked by egress policy.
    fn egress_hosts(&self) -> &'static [&'static str];

    async fn search(
        &self,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure>;
}

/// `https://www.tokio.rs/x` → `tokio.rs`.
pub fn site_of(url: &str) -> String {
    url_host(url)
        .map(|host| host.trim_start_matches("www.").to_string())
        .unwrap_or_default()
}

fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;
    let host = host.split(':').next()?.trim().to_ascii_lowercase();
    (!host.is_empty()).then_some(host)
}

fn host_matches(host: &str, domain: &str) -> bool {
    let domain = domain
        .trim()
        .trim_start_matches("*.")
        .trim_start_matches("www.")
        .to_ascii_lowercase();
    if domain.is_empty() {
        return false;
    }
    let host = host.trim_start_matches("www.");
    host == domain || host.ends_with(&format!(".{domain}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn site_strips_scheme_www_port_and_path() {
        assert_eq!(site_of("https://www.tokio.rs:443/blog?x=1"), "tokio.rs");
        assert_eq!(site_of("http://docs.rs/tokio"), "docs.rs");
        assert_eq!(site_of("not a url"), "not a url");
    }

    #[test]
    fn domain_filters_match_subdomains_and_block_first() {
        let mut request = SearchRequest::new("q");
        request.allowed_domains = vec!["rust-lang.org".into()];
        assert!(request.url_passes_domain_filters("https://doc.rust-lang.org/std"));
        assert!(!request.url_passes_domain_filters("https://evil-rust-lang.org/"));
        request.blocked_domains = vec!["doc.rust-lang.org".into()];
        assert!(!request.url_passes_domain_filters("https://doc.rust-lang.org/std"));
        assert!(request.url_passes_domain_filters("https://blog.rust-lang.org/"));
    }

    #[test]
    fn exchange_body_is_capped() {
        let body = vec![b'a'; WEB_SEARCH_RAW_CAP_BYTES + 10];
        let exchange = SearchExchange::get("https://example.com").with_body(200, &body);
        assert!(exchange.truncated);
        assert_eq!(exchange.body.unwrap().len(), WEB_SEARCH_RAW_CAP_BYTES);
    }
}
