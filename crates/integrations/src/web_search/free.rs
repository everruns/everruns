//! Free search: keyless, best-effort, and honest about failures.
//!
//! Ported from yolop's `free_search` and fixed on the way:
//! - web results come from DuckDuckGo HTML, then DuckDuckGo Lite, then
//!   Mojeek, then Wikipedia's search API as a last resort, stopping at the
//!   first source that answers;
//! - a bot check or an unreadable page is a failure that moves to the next
//!   source, never "0 results";
//! - each result's fields are read from that result's own block;
//! - `topic: dev` asks GitHub, npm and crates.io in parallel and interleaves
//!   them with the web results instead of appending them after the limit;
//! - DuckDuckGo's Instant Answer becomes a top `answer` when there is one;
//! - a short in-process cache and a per-host pace keep a burst of agent
//!   searches from getting the worker's address blocked.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use everruns_contracts::runtime::egress::EgressResponse;
use everruns_contracts::runtime::web_search::{
    SearchAnswer, SearchContext, SearchDriver, SearchDriverCapabilities, SearchExchange,
    SearchFailure, SearchFailureKind, SearchFreshness, SearchHit, SearchRequest, SearchResponse,
    SearchTopic,
};
use serde_json::Value;

use super::html;
use super::http::{self, BROWSER_USER_AGENT};

/// API user agent: registries ask clients to identify themselves.
const API_USER_AGENT: &str = "everruns-web-search (+https://everruns.com)";
const CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const CACHE_CAP: usize = 256;
/// Minimum spacing between requests to one search page host.
const PAGE_HOST_INTERVAL: Duration = Duration::from_millis(700);
/// A search waits at most this long for its turn at a busy host.
const MAX_PACE_WAIT: Duration = Duration::from_secs(5);
const DEV_RESULTS_PER_SOURCE: u32 = 5;

/// Where free search sends its requests; tests point these at a mock server.
#[derive(Debug, Clone)]
pub struct FreeSearchEndpoints {
    pub duckduckgo_html: String,
    pub duckduckgo_lite: String,
    pub mojeek: String,
    pub wikipedia: String,
    pub instant_answer: String,
    pub github: String,
    pub npm: String,
    pub crates: String,
}

impl Default for FreeSearchEndpoints {
    fn default() -> Self {
        Self {
            duckduckgo_html: "https://html.duckduckgo.com/html/".into(),
            duckduckgo_lite: "https://lite.duckduckgo.com/lite/".into(),
            mojeek: "https://www.mojeek.com/search".into(),
            wikipedia: "https://en.wikipedia.org/w/api.php".into(),
            instant_answer: "https://api.duckduckgo.com/".into(),
            github: "https://api.github.com/search/repositories".into(),
            npm: "https://registry.npmjs.org/-/v1/search".into(),
            crates: "https://crates.io/api/v1/crates".into(),
        }
    }
}

/// The keyless search driver.
#[derive(Debug, Clone, Default)]
pub struct FreeSearchDriver {
    endpoints: FreeSearchEndpoints,
    /// Off in tests so they neither wait nor share answers.
    pace_and_cache: PaceAndCache,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaceAndCache {
    On,
    Off,
}

impl Default for PaceAndCache {
    fn default() -> Self {
        Self::On
    }
}

impl FreeSearchDriver {
    /// A driver against custom endpoints, without pacing or caching.
    pub fn with_endpoints(endpoints: FreeSearchEndpoints) -> Self {
        Self {
            endpoints,
            pace_and_cache: PaceAndCache::Off,
        }
    }
}

#[async_trait]
impl SearchDriver for FreeSearchDriver {
    fn id(&self) -> &str {
        "free"
    }

    fn label(&self) -> &str {
        "Free search"
    }

    fn capabilities(&self) -> SearchDriverCapabilities {
        SearchDriverCapabilities {
            freshness: true,
            domain_filters: false,
            excerpts: false,
        }
    }

    fn egress_hosts(&self) -> &'static [&'static str] {
        &[
            "html.duckduckgo.com",
            "lite.duckduckgo.com",
            "api.duckduckgo.com",
            "www.mojeek.com",
            "en.wikipedia.org",
            "api.github.com",
            "registry.npmjs.org",
            "crates.io",
        ]
    }

    async fn search(
        &self,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> Result<SearchResponse, SearchFailure> {
        let key = serde_json::to_string(request).unwrap_or_default();
        if self.pace_and_cache == PaceAndCache::On
            && let Some(mut cached) = cache_get(&key)
        {
            cached
                .notes
                .push("Answered from a free search made in the last 10 minutes.".into());
            return Ok(cached);
        }

        let query = query_with_sites(request);
        let dev = request.topic == SearchTopic::Dev;
        let (web, answer, dev_lists) = futures::join!(
            self.web_results(&query, request, context),
            self.instant_answer(&request.query, context),
            async {
                if dev {
                    self.dev_results(&request.query, context).await
                } else {
                    Vec::new()
                }
            },
        );

        let mut exchanges = web.exchanges;
        exchanges.extend(answer.1);
        let mut notes = web.notes;
        let mut lists = vec![web.hits];
        for (hits, exchange) in dev_lists {
            exchanges.push(exchange);
            lists.push(hits);
        }
        let mut hits = interleave(lists);
        hits.retain(|hit| request.url_passes_domain_filters(&hit.url));
        dedupe(&mut hits);
        hits.truncate(request.limit as usize);

        if hits.is_empty() && answer.0.is_none() && !web.confirmed_empty {
            return Err(SearchFailure::new(
                web.failure_kind.unwrap_or(SearchFailureKind::Unavailable),
                format!("Free search could not reach a search engine: {}", notes.join(" ")),
            )
            .with_exchanges(exchanges));
        }
        if request.freshness.is_some() && !web.honored_freshness {
            notes.push("The source that answered cannot filter by date.".into());
        }

        let response = SearchResponse {
            hits,
            answer: answer.0,
            notes,
            exchanges,
        };
        if self.pace_and_cache == PaceAndCache::On {
            cache_put(key, &response);
        }
        Ok(response)
    }
}

struct WebResults {
    hits: Vec<SearchHit>,
    exchanges: Vec<SearchExchange>,
    notes: Vec<String>,
    /// A source answered "no results" (as opposed to failing).
    confirmed_empty: bool,
    honored_freshness: bool,
    failure_kind: Option<SearchFailureKind>,
}

#[derive(Clone, Copy)]
enum WebSource {
    DuckDuckGoHtml,
    DuckDuckGoLite,
    Mojeek,
    Wikipedia,
}

impl WebSource {
    fn label(self) -> &'static str {
        match self {
            Self::DuckDuckGoHtml => "DuckDuckGo",
            Self::DuckDuckGoLite => "DuckDuckGo Lite",
            Self::Mojeek => "Mojeek",
            Self::Wikipedia => "Wikipedia",
        }
    }

    fn id(self) -> &'static str {
        match self {
            Self::DuckDuckGoHtml => "duckduckgo",
            Self::DuckDuckGoLite => "duckduckgo_lite",
            Self::Mojeek => "mojeek",
            Self::Wikipedia => "wikipedia",
        }
    }
}

/// What one source's page said.
enum PageOutcome {
    Hits(Vec<SearchHit>),
    NoResults,
    Blocked(&'static str),
}

impl FreeSearchDriver {
    async fn web_results(
        &self,
        query: &str,
        request: &SearchRequest,
        context: &SearchContext,
    ) -> WebResults {
        let mut out = WebResults {
            hits: Vec::new(),
            exchanges: Vec::new(),
            notes: Vec::new(),
            confirmed_empty: false,
            honored_freshness: false,
            failure_kind: None,
        };
        let limit = request.limit.max(1);
        for source in [
            WebSource::DuckDuckGoHtml,
            WebSource::DuckDuckGoLite,
            WebSource::Mojeek,
            WebSource::Wikipedia,
        ] {
            let (url, honors_freshness) = self.source_url(source, query, request, limit);
            let host_paced = !matches!(source, WebSource::Wikipedia);
            if host_paced && !self.pace(&url).await {
                out.notes
                    .push(format!("{} was busy with other searches.", source.label()));
                out.failure_kind = Some(SearchFailureKind::RateLimited);
                continue;
            }
            let user_agent = if host_paced {
                BROWSER_USER_AGENT
            } else {
                API_USER_AGENT
            };
            let (exchange, response) = http::get(context, &url, &[("User-Agent", user_agent)]).await;
            out.exchanges.push(exchange);
            let response = match response {
                Ok(response) => response,
                Err(failure) => {
                    out.notes
                        .push(format!("{} failed: {}.", source.label(), failure.message));
                    out.failure_kind = Some(failure.kind);
                    continue;
                }
            };
            let body = String::from_utf8_lossy(&response.body);
            let outcome = match source {
                WebSource::DuckDuckGoHtml => parse_duckduckgo_html(response.status, &body),
                WebSource::DuckDuckGoLite => parse_duckduckgo_lite(response.status, &body),
                WebSource::Mojeek => parse_mojeek(response.status, &body),
                WebSource::Wikipedia => parse_wikipedia(response.status, &body),
            };
            match outcome {
                PageOutcome::Hits(mut hits) => {
                    for hit in &mut hits {
                        hit.source = Some(source.id().into());
                    }
                    if matches!(source, WebSource::Wikipedia) {
                        out.notes.push(
                            "Web search engines were unavailable; only Wikipedia answered."
                                .into(),
                        );
                    }
                    out.hits = hits;
                    out.honored_freshness = honors_freshness;
                    return out;
                }
                PageOutcome::NoResults => {
                    out.confirmed_empty = true;
                    out.honored_freshness = honors_freshness;
                    return out;
                }
                PageOutcome::Blocked(why) => {
                    out.notes.push(format!("{} {why}.", source.label()));
                    out.failure_kind = Some(SearchFailureKind::Blocked);
                }
            }
        }
        out
    }

    fn source_url(
        &self,
        source: WebSource,
        query: &str,
        request: &SearchRequest,
        limit: u32,
    ) -> (String, bool) {
        let q = http::encode(query);
        let df = request.freshness.map(|freshness| match freshness {
            SearchFreshness::Day => "d",
            SearchFreshness::Week => "w",
            SearchFreshness::Month => "m",
            SearchFreshness::Year => "y",
        });
        let with_df = |base: &str| match df {
            Some(df) => format!("{base}?q={q}&df={df}"),
            None => format!("{base}?q={q}"),
        };
        match source {
            WebSource::DuckDuckGoHtml => (with_df(&self.endpoints.duckduckgo_html), true),
            WebSource::DuckDuckGoLite => (with_df(&self.endpoints.duckduckgo_lite), true),
            WebSource::Mojeek => (format!("{}?q={q}", self.endpoints.mojeek), false),
            WebSource::Wikipedia => (
                format!(
                    "{}?action=query&list=search&format=json&utf8=1&srlimit={}&srsearch={}",
                    self.endpoints.wikipedia,
                    limit.min(10),
                    http::encode(&request.query)
                ),
                false,
            ),
        }
    }

    async fn instant_answer(
        &self,
        query: &str,
        context: &SearchContext,
    ) -> (Option<SearchAnswer>, Option<SearchExchange>) {
        let url = format!(
            "{}?q={}&format=json&no_html=1&skip_disambig=1",
            self.endpoints.instant_answer,
            http::encode(query)
        );
        let (exchange, response) = http::get(context, &url, &[("User-Agent", API_USER_AGENT)]).await;
        let answer = response
            .ok()
            .filter(|response| response.status == 200)
            .and_then(|response| serde_json::from_slice::<Value>(&response.body).ok())
            .and_then(|json| {
                let text = json.get("AbstractText")?.as_str()?.trim();
                let url = json.get("AbstractURL")?.as_str()?;
                (!text.is_empty() && url.starts_with("http")).then(|| SearchAnswer {
                    text: text.to_string(),
                    source: json
                        .get("AbstractSource")
                        .and_then(Value::as_str)
                        .unwrap_or("DuckDuckGo")
                        .to_string(),
                    url: url.to_string(),
                })
            });
        (answer, Some(exchange))
    }

    async fn dev_results(
        &self,
        query: &str,
        context: &SearchContext,
    ) -> Vec<(Vec<SearchHit>, SearchExchange)> {
        let q = http::encode(query);
        let n = DEV_RESULTS_PER_SOURCE;
        let github = format!("{}?q={q}&per_page={n}", self.endpoints.github);
        let npm = format!("{}?text={q}&size={n}", self.endpoints.npm);
        let crates = format!("{}?q={q}&per_page={n}", self.endpoints.crates);
        let headers = [
            ("User-Agent", API_USER_AGENT),
            ("Accept", "application/json"),
        ];
        let (github, npm, crates) = futures::join!(
            http::get(context, &github, &headers),
            http::get(context, &npm, &headers),
            http::get(context, &crates, &headers),
        );
        vec![
            read_json_hits(github, parse_github),
            read_json_hits(npm, parse_npm),
            read_json_hits(crates, parse_crates),
        ]
    }

    /// Wait for this host's turn. `false` when the wait would be too long.
    async fn pace(&self, url: &str) -> bool {
        if self.pace_and_cache == PaceAndCache::Off {
            return true;
        }
        static NEXT: OnceLock<Mutex<HashMap<String, Instant>>> = OnceLock::new();
        let host = everruns_contracts::runtime::web_search::site_of(url);
        let wait = {
            let mut next = NEXT
                .get_or_init(Default::default)
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            let now = Instant::now();
            let slot = next.get(&host).copied().unwrap_or(now).max(now);
            if slot - now > MAX_PACE_WAIT {
                return false;
            }
            next.insert(host, slot + PAGE_HOST_INTERVAL);
            slot - now
        };
        if !wait.is_zero() {
            tokio::time::sleep(wait).await;
        }
        true
    }
}

fn read_json_hits(
    (exchange, response): (SearchExchange, Result<EgressResponse, SearchFailure>),
    parse: fn(&Value) -> Vec<SearchHit>,
) -> (Vec<SearchHit>, SearchExchange) {
    let hits = response
        .ok()
        .filter(|response| response.status == 200)
        .and_then(|response| serde_json::from_slice::<Value>(&response.body).ok())
        .map(|json| parse(&json))
        .unwrap_or_default();
    (hits, exchange)
}

fn cache() -> &'static Mutex<HashMap<String, (Instant, SearchResponse)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (Instant, SearchResponse)>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

fn cache_get(key: &str) -> Option<SearchResponse> {
    let cache = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    cache
        .get(key)
        .filter(|(at, _)| at.elapsed() < CACHE_TTL)
        .map(|(_, response)| response.clone())
}

fn cache_put(key: String, response: &SearchResponse) {
    let mut cache = cache().lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if cache.len() >= CACHE_CAP {
        cache.retain(|_, (at, _)| at.elapsed() < CACHE_TTL);
        if cache.len() >= CACHE_CAP {
            cache.clear();
        }
    }
    cache.insert(key, (Instant::now(), response.clone()));
}

/// Narrow the engine query with `site:` operators when the agent allows only
/// a few sites; results are filtered again afterwards either way.
fn query_with_sites(request: &SearchRequest) -> String {
    match request.allowed_domains.as_slice() {
        [] => request.query.clone(),
        [one] => format!("{} site:{one}", request.query),
        some if some.len() <= 3 => format!(
            "{} ({})",
            request.query,
            some.iter()
                .map(|domain| format!("site:{domain}"))
                .collect::<Vec<_>>()
                .join(" OR ")
        ),
        _ => request.query.clone(),
    }
}

fn interleave(lists: Vec<Vec<SearchHit>>) -> Vec<SearchHit> {
    let mut iters: Vec<_> = lists.into_iter().map(Vec::into_iter).collect();
    let mut out = Vec::new();
    loop {
        let before = out.len();
        for iter in &mut iters {
            if let Some(hit) = iter.next() {
                out.push(hit);
            }
        }
        if out.len() == before {
            return out;
        }
    }
}

fn dedupe(hits: &mut Vec<SearchHit>) {
    let mut seen = std::collections::HashSet::new();
    hits.retain(|hit| {
        let key = hit
            .url
            .trim_end_matches('/')
            .replacen("http://", "https://", 1)
            .replacen("://www.", "://", 1);
        seen.insert(key)
    });
}

// ---------------------------------------------------------------------------
// Page readers
// ---------------------------------------------------------------------------

fn is_duckduckgo_bot_check(status: u16, body: &str) -> bool {
    // DuckDuckGo answers a suspected bot with 202 and an "anomaly" challenge.
    status == 202
        || body.contains("anomaly-modal")
        || body.contains("challenge-form")
        || body.contains("bots use DuckDuckGo")
}

fn parse_duckduckgo_html(status: u16, body: &str) -> PageOutcome {
    if is_duckduckgo_bot_check(status, body) {
        return PageOutcome::Blocked("showed a bot check");
    }
    if status != 200 {
        return PageOutcome::Blocked("returned an error page");
    }
    let hits: Vec<SearchHit> = html::blocks(body, "div", "result")
        .into_iter()
        .filter(|block| !block[..block.find('>').unwrap_or(0)].contains("result--ad"))
        .filter_map(|block| {
            let link = html::find(block, "a", "result__a")?;
            let url = result_url(&link.attr("href")?)?;
            let mut hit = SearchHit::new(link.text(), url);
            hit.snippet = html::find(block, "a", "result__snippet")
                .map(|snippet| snippet.text())
                .filter(|snippet| !snippet.is_empty());
            hit.published = html::find(block, "div", "result__extras__url")
                .and_then(|extras| iso_date(extras.inner));
            (!hit.title.is_empty()).then_some(hit)
        })
        .collect();
    if hits.is_empty() {
        return if body.contains("no-results") {
            PageOutcome::NoResults
        } else {
            PageOutcome::Blocked("returned a page without results")
        };
    }
    PageOutcome::Hits(hits)
}

fn parse_duckduckgo_lite(status: u16, body: &str) -> PageOutcome {
    if is_duckduckgo_bot_check(status, body) {
        return PageOutcome::Blocked("showed a bot check");
    }
    if status != 200 {
        return PageOutcome::Blocked("returned an error page");
    }
    let hits: Vec<SearchHit> = html::blocks(body, "a", "result-link")
        .into_iter()
        .filter_map(|block| {
            let link = html::find(block, "a", "result-link")?;
            let url = result_url(&link.attr("href")?)?;
            let mut hit = SearchHit::new(link.text(), url);
            hit.snippet = html::find(block, "td", "result-snippet")
                .map(|snippet| snippet.text())
                .filter(|snippet| !snippet.is_empty());
            (!hit.title.is_empty()).then_some(hit)
        })
        .collect();
    if hits.is_empty() {
        return if body.contains("No results.") {
            PageOutcome::NoResults
        } else {
            PageOutcome::Blocked("returned a page without results")
        };
    }
    PageOutcome::Hits(hits)
}

fn parse_mojeek(status: u16, body: &str) -> PageOutcome {
    if status == 403 || status == 429 || body.contains("automated queries") {
        return PageOutcome::Blocked("refused automated queries");
    }
    if status != 200 {
        return PageOutcome::Blocked("returned an error page");
    }
    let results = body
        .find("results-standard")
        .map_or("", |start| &body[start..]);
    let hits: Vec<SearchHit> = html::blocks(results, "li", "")
        .into_iter()
        .filter_map(|block| {
            let link = html::find(block, "a", "title")?;
            let url = link.attr("href").filter(|url| url.starts_with("http"))?;
            let mut hit = SearchHit::new(link.text(), url);
            hit.snippet = html::find(block, "p", "s")
                .map(|snippet| snippet.text())
                .filter(|snippet| !snippet.is_empty());
            (!hit.title.is_empty()).then_some(hit)
        })
        .collect();
    if hits.is_empty() {
        return if body.contains("No pages found") {
            PageOutcome::NoResults
        } else {
            PageOutcome::Blocked("returned a page without results")
        };
    }
    PageOutcome::Hits(hits)
}

fn parse_wikipedia(status: u16, body: &str) -> PageOutcome {
    if status != 200 {
        return PageOutcome::Blocked("returned an error");
    }
    let Ok(json) = serde_json::from_str::<Value>(body) else {
        return PageOutcome::Blocked("returned unreadable JSON");
    };
    let Some(items) = json.pointer("/query/search").and_then(Value::as_array) else {
        return PageOutcome::Blocked("returned an unexpected answer");
    };
    if items.is_empty() {
        return PageOutcome::NoResults;
    }
    PageOutcome::Hits(
        items
            .iter()
            .filter_map(|item| {
                let title = item.get("title")?.as_str()?;
                let url = format!(
                    "https://en.wikipedia.org/wiki/{}",
                    http::encode(&title.replace(' ', "_")).replace('+', "_")
                );
                let mut hit = SearchHit::new(title, url);
                hit.snippet = item
                    .get("snippet")
                    .and_then(Value::as_str)
                    .map(html::text)
                    .filter(|snippet| !snippet.is_empty());
                hit.published = item
                    .get("timestamp")
                    .and_then(Value::as_str)
                    .and_then(|ts| ts.get(..10))
                    .map(str::to_string);
                Some(hit)
            })
            .collect(),
    )
}

fn parse_github(json: &Value) -> Vec<SearchHit> {
    json.get("items")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let mut hit = SearchHit::new(
                item.get("full_name")?.as_str()?,
                item.get("html_url")?.as_str()?,
            );
            hit.snippet = item
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            hit.published = item
                .get("pushed_at")
                .and_then(Value::as_str)
                .and_then(|ts| ts.get(..10))
                .map(str::to_string);
            hit.source = Some("github".into());
            Some(hit)
        })
        .collect()
}

fn parse_npm(json: &Value) -> Vec<SearchHit> {
    json.get("objects")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let package = item.get("package")?;
            let name = package.get("name")?.as_str()?;
            let url = package
                .pointer("/links/npm")
                .and_then(Value::as_str)
                .map_or_else(|| format!("https://www.npmjs.com/package/{name}"), str::to_string);
            let mut hit = SearchHit::new(format!("{name} (npm)"), url);
            hit.snippet = package
                .get("description")
                .and_then(Value::as_str)
                .map(str::to_string);
            hit.published = package
                .get("date")
                .and_then(Value::as_str)
                .and_then(|ts| ts.get(..10))
                .map(str::to_string);
            hit.source = Some("npm".into());
            Some(hit)
        })
        .collect()
}

fn parse_crates(json: &Value) -> Vec<SearchHit> {
    json.get("crates")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?;
            let mut hit = SearchHit::new(
                format!("{name} (crate)"),
                format!("https://crates.io/crates/{name}"),
            );
            hit.snippet = item
                .get("description")
                .and_then(Value::as_str)
                .map(|text| text.trim().to_string());
            hit.published = item
                .get("updated_at")
                .and_then(Value::as_str)
                .and_then(|ts| ts.get(..10))
                .map(str::to_string);
            hit.source = Some("crates_io".into());
            Some(hit)
        })
        .collect()
}

/// Real target of a DuckDuckGo result link (`//duckduckgo.com/l/?uddg=…`).
/// Ads and DuckDuckGo's own pages return `None`.
fn result_url(href: &str) -> Option<String> {
    let url = if let Some(query) = href.split_once("uddg=").map(|(_, rest)| rest) {
        http::decode(query.split('&').next().unwrap_or_default())
    } else if href.starts_with("//") {
        format!("https:{href}")
    } else {
        href.to_string()
    };
    let site = everruns_contracts::runtime::web_search::site_of(&url);
    (url.starts_with("http") && !site.ends_with("duckduckgo.com")).then_some(url)
}

/// First `YYYY-MM-DD` in an HTML fragment.
fn iso_date(fragment: &str) -> Option<String> {
    let bytes = fragment.as_bytes();
    (0..bytes.len().saturating_sub(9)).find_map(|i| {
        let window = &bytes[i..i + 10];
        let shape = window.iter().enumerate().all(|(j, b)| match j {
            4 | 7 => *b == b'-',
            _ => b.is_ascii_digit(),
        });
        shape.then(|| String::from_utf8_lossy(window).into_owned())
    })
}

#[cfg(test)]
#[path = "free_tests.rs"]
mod tests;
