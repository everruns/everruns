#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]
//! Live Cloudflare Web Search API test for the search account driver and its
//! connector.
//!
//! Needs `CLOUDFLARE_ACCOUNT_ID` and `CLOUDFLARE_API_TOKEN` (Workers AI Read
//! and AI Gateway Read) and returns early without them:
//!   doppler run -- cargo test -p everruns-integrations \
//!     --features web-search --test web_search_cloudflare_live

use std::collections::HashMap;

use everruns_contracts::connector::Connector;
use everruns_contracts::runtime::web_search::{
    SearchContext, SearchDriver, SearchFailureKind, SearchRequest,
};
use everruns_integrations::web_search::{
    CloudflareSearchDriver, CloudflareSearchIndex, CloudflareSearchOptions,
    CloudflareWebSearchConnector,
};

fn credentials() -> Option<(String, String)> {
    let get = |name: &str| std::env::var(name).ok().filter(|value| !value.is_empty());
    match (get("CLOUDFLARE_ACCOUNT_ID"), get("CLOUDFLARE_API_TOKEN")) {
        (Some(account), Some(token)) => Some((account, token)),
        _ => {
            eprintln!("Skipping: CLOUDFLARE_ACCOUNT_ID or CLOUDFLARE_API_TOKEN is not set");
            None
        }
    }
}

#[tokio::test]
async fn ceramic_answers_through_cloudflare() {
    let Some((account, token)) = credentials() else {
        return;
    };
    let driver = CloudflareSearchDriver::new(
        token,
        CloudflareSearchOptions::new(account, CloudflareSearchIndex::Ceramic),
    );
    let mut request = SearchRequest::new("tokio rust async runtime");
    request.limit = 3;
    let context = SearchContext {
        egress: std::sync::Arc::new(
            everruns_core::host::DirectEgressService::for_runtime_traffic_from_env(),
        ),
        network_access: None,
    };
    let response = match driver.search(&request, &context).await {
        Ok(response) => response,
        // A token without the search permissions is account state, not a
        // driver bug.
        Err(failure) if failure.kind == SearchFailureKind::Unauthorized => {
            eprintln!("Skipping: {}", failure.message);
            return;
        }
        Err(failure) => panic!("{failure:?}"),
    };
    assert!(!response.hits.is_empty());
    assert!(response.hits.len() <= 3);
    assert!(response.hits[0].url.starts_with("http"));
    eprintln!("{:#?}", response.hits);
}

#[tokio::test]
async fn connecting_runs_a_test_search_and_stores_settings() {
    let Some((account, token)) = credentials() else {
        return;
    };
    let fields = HashMap::from([
        ("account_id".to_string(), account.clone()),
        ("api_key".to_string(), token),
    ]);
    match CloudflareWebSearchConnector.validate_fields(&fields).await {
        Ok(validation) => {
            let metadata = validation.provider_metadata.expect("settings");
            assert_eq!(metadata["account_id"], account.to_ascii_lowercase());
            assert_eq!(metadata["index"], "ceramic");
            assert_eq!(metadata["gateway_id"], "default");
        }
        Err(error) => eprintln!("Skipping: {error}"),
    }
}
