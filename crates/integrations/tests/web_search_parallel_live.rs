#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live Parallel Search API test for the `web_search` capability.
//!
//! Needs `PARALLEL_API_KEY` and returns early without it:
//!   doppler run -- cargo test -p everruns-integrations \
//!     --features web-search,parallel-integration --test web_search_parallel_live

use everruns_contracts::runtime::capabilities::Capability;
use everruns_integrations::web_search::{PARALLEL_API_KEY, WebSearchCapability};
use serde_json::json;

#[tokio::test]
async fn parallel_answers_web_search() {
    let Some(key) = std::env::var(PARALLEL_API_KEY)
        .ok()
        .filter(|key| !key.is_empty())
    else {
        eprintln!("Skipping: {PARALLEL_API_KEY} is not set");
        return;
    };
    let tools = WebSearchCapability::with_parallel_key(key)
        .tools_with_config(&json!({"provider": "parallel", "fallback": "none"}));
    let result = tools[0]
        .execute(json!({"query": "tokio rust async runtime", "limit": 3, "freshness": "year"}))
        .await
        .into_tool_result("call_1", "web_search");
    let details = result.details.expect("details");
    assert_eq!(details["provider"], "parallel", "{details}");
    let model = result.result.expect("result");
    let results = model["results"].as_array().expect("results");
    assert!(!results.is_empty(), "{model}");
    assert!(results.len() <= 3, "{model}");
    assert!(results[0]["url"].as_str().unwrap().starts_with("http"));
}
