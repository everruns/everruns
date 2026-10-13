//! The providers under test, each called the way production calls it.
//!
//! Free, Brave, Parallel and the three Cloudflare indexes run through the
//! shipped `SearchDriver`s with direct egress, so a run measures the working
//! tree's request building and result mapping, not a reimplementation.
//!
//! Native search has no driver: it is a model-side tool. It is measured as
//! OpenAI Responses `web_search` on `gpt-6-luna`, the hosted default model
//! family, and its hits are the pages the model cited, then the other sources
//! its searches returned. That gives native an advantage worth stating: a
//! model picks and re-queries before citing, while every other provider is
//! graded on one raw result list.

use std::sync::Arc;
use std::time::Instant;

use everruns_contracts::runtime::web_search::{
    SearchContext, SearchDriver, SearchFailureKind, SearchHit, SearchRequest,
};
use everruns_integrations::web_search::{
    BraveSearchDriver, CloudflareSearchDriver, CloudflareSearchIndex, CloudflareSearchOptions,
    FreeSearchDriver, ParallelSearchDriver,
};
use serde_json::{Value, json};

/// Results requested from every provider: the hosted default.
pub const LIMIT: u32 = 5;

pub const ALL: &[&str] = &[
    "free", "brave", "parallel", "ceramic", "exa", "linkup", "native",
];

/// List price per search in USD, for the cost column. Native adds tokens on
/// top, measured per call.
pub fn price_per_search(provider: &str) -> f64 {
    match provider {
        "brave" => 0.005,
        "parallel" => 0.005,
        "ceramic" => 0.00025,
        "exa" => 0.007,
        "linkup" => 0.005,
        "native" => 0.010,
        _ => 0.0,
    }
}

/// One provider's answer to one query.
pub struct Outcome {
    pub hits: Vec<SearchHit>,
    pub latency_ms: u64,
    /// Extra cost beyond the list price (native's tokens).
    pub extra_cost_usd: f64,
    /// 429s retried before this answer.
    pub rate_limited: u32,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

/// Why a provider cannot run here, or `None` when it can.
pub fn missing_credential(provider: &str) -> Option<&'static str> {
    let needs: &[&'static str] = match provider {
        "free" => &[],
        "brave" => &["BRAVE_SEARCH_API_KEY"],
        "parallel" => &["PARALLEL_API_KEY"],
        "ceramic" | "exa" | "linkup" => &["CLOUDFLARE_ACCOUNT_ID", "CLOUDFLARE_API_TOKEN"],
        "native" => &["OPENAI_API_KEY"],
        _ => return Some("unknown provider"),
    };
    needs.iter().copied().find(|name| env(name).is_none())
}

fn context() -> SearchContext {
    SearchContext {
        egress: Arc::new(everruns_core::host::DirectEgressService::for_runtime_traffic_from_env()),
        network_access: None,
    }
}

fn driver(provider: &str) -> Result<Box<dyn SearchDriver>, String> {
    let cloudflare = |index| {
        let account = env("CLOUDFLARE_ACCOUNT_ID").unwrap_or_default();
        let token = env("CLOUDFLARE_API_TOKEN").unwrap_or_default();
        Box::new(CloudflareSearchDriver::new(
            token,
            CloudflareSearchOptions::new(account, index),
        )) as Box<dyn SearchDriver>
    };
    Ok(match provider {
        "free" => Box::new(FreeSearchDriver::default()),
        "brave" => Box::new(BraveSearchDriver::new(
            env("BRAVE_SEARCH_API_KEY").unwrap_or_default(),
        )),
        "parallel" => Box::new(ParallelSearchDriver::new(
            env("PARALLEL_API_KEY").unwrap_or_default(),
        )),
        "ceramic" => cloudflare(CloudflareSearchIndex::Ceramic),
        "exa" => cloudflare(CloudflareSearchIndex::Exa),
        "linkup" => cloudflare(CloudflareSearchIndex::Linkup),
        other => return Err(format!("no driver for '{other}'")),
    })
}

pub async fn search(provider: &str, query: &str) -> Result<Outcome, String> {
    let started = Instant::now();
    if provider == "native" {
        let (hits, extra_cost_usd) = native(query).await?;
        return Ok(Outcome {
            hits,
            latency_ms: started.elapsed().as_millis() as u64,
            extra_cost_usd,
            rate_limited: 0,
        });
    }
    let driver = driver(provider)?;
    let mut request = SearchRequest::new(query);
    request.limit = LIMIT;
    // A rate limit says how hard the study pushed, not how good the index
    // is: back off and retry, and report the count so it is not hidden.
    let mut rate_limited = 0;
    loop {
        let attempt = Instant::now();
        match driver.search(&request, &context()).await {
            Ok(response) => {
                let mut hits = response.hits;
                hits.truncate(LIMIT as usize);
                return Ok(Outcome {
                    hits,
                    latency_ms: attempt.elapsed().as_millis() as u64,
                    extra_cost_usd: 0.0,
                    rate_limited,
                });
            }
            Err(failure) if failure.kind == SearchFailureKind::RateLimited && rate_limited < 4 => {
                rate_limited += 1;
                tokio::time::sleep(std::time::Duration::from_secs(2 << rate_limited)).await;
            }
            Err(failure) => return Err(format!("{:?}: {}", failure.kind, failure.message)),
        }
    }
}

/// gpt-6-luna list prices per million tokens (model profile `gpt6.rs`).
const LUNA_INPUT_PER_M: f64 = 0.10;
const LUNA_OUTPUT_PER_M: f64 = 0.50;

async fn native(query: &str) -> Result<(Vec<SearchHit>, f64), String> {
    let key = env("OPENAI_API_KEY").ok_or("OPENAI_API_KEY is not set")?;
    let body = json!({
        "model": "gpt-6-luna",
        "tools": [{ "type": "web_search" }],
        "tool_choice": "required",
        "include": ["web_search_call.action.sources"],
        "input": format!(
            "Search the web for the query below and cite the pages that answer it.\n\nQuery: {query}"
        ),
    });
    let response: Value = reqwest::Client::new()
        .post("https://api.openai.com/v1/responses")
        .bearer_auth(key)
        .json(&body)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    if let Some(error) = response.get("error").filter(|e| !e.is_null()) {
        return Err(error.to_string());
    }
    let mut hits: Vec<SearchHit> = Vec::new();
    let mut push = |url: &str, title: &str| {
        if !url.starts_with("http") || hits.iter().any(|h| h.url == url) {
            return;
        }
        let title = if title.is_empty() { url } else { title };
        hits.push(SearchHit::new(title, url));
    };
    let output = response["output"].as_array().cloned().unwrap_or_default();
    // Cited pages first: they are what the model chose.
    for item in &output {
        for part in item["content"].as_array().into_iter().flatten() {
            for note in part["annotations"].as_array().into_iter().flatten() {
                if note["type"] == "url_citation" {
                    push(
                        note["url"].as_str().unwrap_or_default(),
                        note["title"].as_str().unwrap_or_default(),
                    );
                }
            }
        }
    }
    let mut searches = 0u32;
    for item in &output {
        if item["type"] == "web_search_call" && item["action"]["type"] == "search" {
            searches += 1;
            for source in item["action"]["sources"].as_array().into_iter().flatten() {
                push(source["url"].as_str().unwrap_or_default(), "");
            }
        }
    }
    hits.truncate(LIMIT as usize);
    let usage = &response["usage"];
    let tokens = usage["input_tokens"].as_f64().unwrap_or(0.0) * LUNA_INPUT_PER_M
        + usage["output_tokens"].as_f64().unwrap_or(0.0) * LUNA_OUTPUT_PER_M;
    // List price covers one search call; extra calls cost the same each.
    let extra = tokens / 1e6 + f64::from(searches.saturating_sub(1)) * price_per_search("native");
    Ok((hits, extra))
}
