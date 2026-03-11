//! Live API tests against the real Browserless service.
//!
//! These tests require a valid `BROWSERLESS_API_KEY` environment variable.
//! Run via: doppler run -- cargo test -p everruns-integrations-browserless --features browserless-live-tests
//!
//! All tests are gated behind the `browserless-live-tests` feature flag.
//! They are NOT run in regular CI — only for manual verification and Doppler environments.

#![cfg(feature = "browserless-live-tests")]

use everruns_integrations_browserless::cdp::CdpSession;
use everruns_integrations_browserless::client::BrowserlessClient;

fn api_token() -> String {
    std::env::var("BROWSERLESS_API_KEY")
        .expect("BROWSERLESS_API_KEY must be set for live API tests")
}

// ============================================================================
// REST API live tests
// ============================================================================

#[tokio::test]
async fn live_screenshot() {
    let client = BrowserlessClient::new(api_token());
    let bytes = client
        .screenshot("https://example.com", true, None, None, None)
        .await
        .expect("Screenshot should succeed");

    assert!(!bytes.is_empty(), "Screenshot bytes should not be empty");
    // PNG magic bytes
    assert_eq!(&bytes[..4], b"\x89PNG", "Should be valid PNG");
}

#[tokio::test]
async fn live_content() {
    let client = BrowserlessClient::new(api_token());
    let html = client
        .content("https://example.com", None, None, false)
        .await
        .expect("Content should succeed");

    assert!(html.contains("Example Domain"), "Should contain page text");
    assert!(html.contains("<html"), "Should be HTML");
}

#[tokio::test]
async fn live_scrape() {
    let client = BrowserlessClient::new(api_token());
    let result = client
        .scrape(
            "https://example.com",
            &[serde_json::json!({"selector": "h1"})],
            None,
            None,
        )
        .await
        .expect("Scrape should succeed");

    assert!(result.get("data").is_some(), "Should have data field");
}

#[tokio::test]
async fn live_function() {
    let client = BrowserlessClient::new(api_token());
    let result = client
        .function(
            r#"export default async ({ page }) => {
                await page.goto('https://example.com', { waitUntil: 'networkidle2' });
                const title = await page.title();
                return { data: JSON.stringify({ title }), type: 'application/json' };
            }"#,
            None,
        )
        .await
        .expect("Function should succeed");

    let data_str = result
        .get("data")
        .and_then(|v| v.as_str())
        .expect("Should have data");
    let data: serde_json::Value = serde_json::from_str(data_str).expect("Should be valid JSON");
    assert_eq!(data["title"], "Example Domain");
}

// ============================================================================
// CDP session live tests
// ============================================================================

#[tokio::test]
async fn live_cdp_session_navigate_and_screenshot() {
    let token = api_token();
    let ws_url = format!("wss://production-sfo.browserless.io?token={token}");

    let mut session = CdpSession::connect(&ws_url)
        .await
        .expect("CDP connect should succeed");

    // Navigate
    session
        .navigate("https://example.com")
        .await
        .expect("Navigate should succeed");

    // Get title
    let title = session.get_title().await.expect("get_title should succeed");
    assert_eq!(title, "Example Domain");

    // Get URL
    let url = session.get_url().await.expect("get_url should succeed");
    assert!(url.contains("example.com"));

    // Screenshot
    let b64 = session
        .screenshot(false)
        .await
        .expect("Screenshot should succeed");
    assert!(!b64.is_empty(), "Screenshot should not be empty");

    // Content
    let content = session
        .get_content()
        .await
        .expect("get_content should succeed");
    assert!(content.contains("Example Domain"));

    // Page info
    let info = session
        .get_page_info()
        .await
        .expect("get_page_info should succeed");
    assert_eq!(info["title"], "Example Domain");

    // Don't call reconnect — browser will be destroyed (cleanup)
    session.disconnect().await;
}

#[tokio::test]
async fn live_cdp_session_reconnect() {
    let token = api_token();
    let ws_url = format!("wss://production-sfo.browserless.io?token={token}");

    // Open session, navigate, reconnect, disconnect
    let mut session = CdpSession::connect(&ws_url)
        .await
        .expect("CDP connect should succeed");

    session
        .navigate("https://example.com")
        .await
        .expect("Navigate should succeed");

    let new_endpoint = session
        .reconnect(30000)
        .await
        .expect("Reconnect should succeed");

    session.disconnect().await;

    // Reconnect using the returned endpoint
    let reconnect_url = if new_endpoint.contains('?') {
        format!("{new_endpoint}&token={token}")
    } else {
        format!("{new_endpoint}?token={token}")
    };

    let mut session2 = CdpSession::connect(&reconnect_url)
        .await
        .expect("Reconnect should succeed");

    // The page should still be at example.com
    let title = session2
        .get_title()
        .await
        .expect("get_title should succeed");
    assert_eq!(
        title, "Example Domain",
        "Page state should persist across reconnect"
    );

    // Clean up (no reconnect = browser destroyed)
    session2.disconnect().await;
}

#[tokio::test]
async fn live_cdp_session_interact() {
    let token = api_token();
    let ws_url = format!("wss://production-sfo.browserless.io?token={token}");

    let mut session = CdpSession::connect(&ws_url)
        .await
        .expect("CDP connect should succeed");

    session
        .navigate("https://example.com")
        .await
        .expect("Navigate should succeed");

    // Click the "More information..." link
    session
        .click_selector("a")
        .await
        .expect("Click should succeed");

    // Wait for navigation
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    // Check we navigated away
    let url = session.get_url().await.expect("get_url should succeed");
    assert!(
        url != "https://example.com/" && url != "https://example.com",
        "Should have navigated away from example.com, got: {url}"
    );

    // Clean up
    session.disconnect().await;
}

// ============================================================================
// Resource cleanup verification
// ============================================================================

#[tokio::test]
async fn live_no_resources_leaked_rest() {
    // REST calls are stateless. Verify multiple independent calls work fine.
    let client = BrowserlessClient::new(api_token());

    let r1 = client
        .content("https://example.com", None, None, false)
        .await;
    let r2 = client
        .content("https://example.com", None, None, false)
        .await;

    assert!(r1.is_ok());
    assert!(r2.is_ok());
}

#[tokio::test]
async fn live_no_resources_leaked_cdp() {
    // Open a CDP session, then let it expire without explicitly closing.
    // Verify no resources are leaked (browser destroyed after timeout).
    let token = api_token();
    let ws_url = format!("wss://production-sfo.browserless.io?token={token}");

    let mut session = CdpSession::connect(&ws_url)
        .await
        .expect("CDP connect should succeed");

    session
        .navigate("https://example.com")
        .await
        .expect("Navigate should succeed");

    // Reconnect with a very short timeout (5s)
    let new_endpoint = session
        .reconnect(5000)
        .await
        .expect("Reconnect should work");
    session.disconnect().await;

    // Wait for the timeout to expire
    tokio::time::sleep(tokio::time::Duration::from_secs(7)).await;

    // Try to reconnect — should fail because the browser was destroyed
    let reconnect_url = format!("{new_endpoint}?token={token}");
    let result = CdpSession::connect(&reconnect_url).await;
    assert!(
        result.is_err(),
        "Should fail to reconnect after timeout — browser was cleaned up"
    );
}
