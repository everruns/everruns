//! Live API tests against the real Browserless service.
//!
//! These tests require a valid `BROWSERLESS_TOKEN` environment variable (set via Doppler).
//! Run via: doppler run -- cargo test -p everruns-integrations-browserless --features browserless-live-tests
//!
//! All tests are gated behind the `browserless-live-tests` feature flag.
//! They are NOT run in regular CI — only for manual verification and Doppler environments.

#![cfg(feature = "browserless-live-tests")]

use everruns_integrations_browserless::cdp::CdpSession;
use everruns_integrations_browserless::client::BrowserlessClient;

fn api_token() -> String {
    std::env::var("BROWSERLESS_TOKEN")
        .expect("BROWSERLESS_TOKEN must be set for live API tests (available via Doppler)")
}

// ============================================================================
// REST API live tests
// ============================================================================

#[tokio::test]
async fn live_screenshot() {
    let client = BrowserlessClient::new(api_token());
    let bytes = client
        .screenshot("https://example.com", true, None, None, None, &[])
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
        .content("https://example.com", None, None, false, &[])
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
            // example.com dropped its <h1> in its 2026 redesign; <title> is the
            // one element the other live tests already pin ("Example Domain").
            &[serde_json::json!({"selector": "title"})],
            None,
            None,
            &[],
        )
        .await
        .expect("Scrape should succeed");

    assert_eq!(
        result["data"][0]["results"][0]["text"], "Example Domain",
        "Should scrape the page title, got: {result}"
    );
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
async fn live_cdp_session_connect_attaches_page_target() {
    let token = api_token();
    let ws_url = format!("wss://production-sfo.browserless.io/chromium?token={token}");

    let session = CdpSession::connect(&ws_url)
        .await
        .expect("CDP connect should succeed");

    assert!(
        !session.page_target_id().is_empty(),
        "connect should attach to a page target"
    );
    assert!(
        !session.page_session_id().is_empty(),
        "connect should expose the attached page sessionId"
    );

    session.disconnect().await;
}

#[tokio::test]
async fn live_cdp_session_navigate_and_screenshot() {
    let token = api_token();
    let ws_url = format!("wss://production-sfo.browserless.io/chromium?token={token}");

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
    let ws_url = format!("wss://production-sfo.browserless.io/chromium?token={token}");

    // Open session, navigate, reconnect, disconnect
    let mut session = CdpSession::connect(&ws_url)
        .await
        .expect("CDP connect should succeed");

    session
        .navigate("https://example.com")
        .await
        .expect("Navigate should succeed");

    let new_endpoint = session
        // Free-tier Browserless tokens cap reconnect TTL at 10s.
        .reconnect(5000)
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
    let ws_url = format!("wss://production-sfo.browserless.io/chromium?token={token}");

    let mut session = CdpSession::connect(&ws_url)
        .await
        .expect("CDP connect should succeed");

    session
        .navigate("https://example.com")
        .await
        .expect("Navigate should succeed");

    // Click the page's only link. It sits below the fold of Browserless' default
    // 800x600 viewport, so this also covers click_selector scrolling it into view.
    session
        .click_selector("a")
        .await
        .expect("Click should succeed");

    // Poll for navigation: the link target is a remote site and may take a moment.
    let mut url = String::new();
    for _ in 0..30 {
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
        // Evaluation can fail while the old document is torn down; retry.
        let Ok(current) = session.get_url().await else {
            continue;
        };
        url = current;
        if url != "https://example.com/" && url != "https://example.com" {
            break;
        }
    }
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
        .content("https://example.com", None, None, false, &[])
        .await;
    let r2 = client
        .content("https://example.com", None, None, false, &[])
        .await;

    assert!(r1.is_ok());
    assert!(r2.is_ok());
}

#[tokio::test]
async fn live_no_resources_leaked_cdp() {
    // Open a CDP session, then let it expire without explicitly closing.
    // Verify no resources are leaked (browser destroyed after timeout).
    let token = api_token();
    let ws_url = format!("wss://production-sfo.browserless.io/chromium?token={token}");

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

// ============================================================================
// Computer use (EVE-1119): the `computer` tool on a real Browserless browser
// ============================================================================

mod computer_use {
    use super::api_token;
    use async_trait::async_trait;
    use everruns_core::capabilities::Capability;
    use everruns_core::connection_services::UserConnectionResolver;
    use everruns_core::network_access::NetworkAccessList;
    use everruns_core::session_services::{KeyInfo, SecretInfo, SessionStorageStore};
    use everruns_core::tool_context::ToolContext;
    use everruns_core::tools::{Tool, ToolExecutionResult};
    use everruns_integrations_browserless::computer::BrowserlessComputerUseCapability;
    use everruns_integrations_browserless::session_tools::BrowserlessCloseBrowserTool;
    use everruns_provider::error::Result;
    use everruns_provider::typed_id::SessionId;
    use serde_json::{Value, json};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    struct Token(String);

    #[async_trait]
    impl UserConnectionResolver for Token {
        async fn get_connection_token(&self, _: SessionId, _: &str) -> Result<Option<String>> {
            Ok(Some(self.0.clone()))
        }
    }

    #[derive(Default)]
    struct Memory(Mutex<HashMap<String, String>>);

    #[async_trait]
    impl SessionStorageStore for Memory {
        async fn set_value(&self, _: SessionId, key: &str, value: &str) -> Result<()> {
            self.0
                .lock()
                .unwrap()
                .insert(key.to_string(), value.to_string());
            Ok(())
        }
        async fn get_value(&self, _: SessionId, key: &str) -> Result<Option<String>> {
            Ok(self.0.lock().unwrap().get(key).cloned())
        }
        async fn delete_value(&self, _: SessionId, key: &str) -> Result<bool> {
            Ok(self.0.lock().unwrap().remove(key).is_some())
        }
        async fn list_keys(&self, _: SessionId) -> Result<Vec<KeyInfo>> {
            Ok(vec![])
        }
        async fn set_secret(&self, _: SessionId, _: &str, _: &str) -> Result<()> {
            Ok(())
        }
        async fn get_secret(&self, _: SessionId, _: &str) -> Result<Option<String>> {
            Ok(None)
        }
        async fn delete_secret(&self, _: SessionId, _: &str) -> Result<bool> {
            Ok(false)
        }
        async fn list_secrets(&self, _: SessionId) -> Result<Vec<SecretInfo>> {
            Ok(vec![])
        }
    }

    fn context() -> ToolContext {
        ToolContext::new(SessionId::new())
            .with_storage_store_arc(Arc::new(Memory::default()))
            .with_connection_resolver(Arc::new(Token(api_token())))
    }

    fn computer() -> Box<dyn Tool> {
        BrowserlessComputerUseCapability
            .tools_with_config(&json!({"display_width": 1024, "display_height": 768}))
            .remove(0)
    }

    async fn run(tool: &dyn Tool, context: &ToolContext, args: Value) -> ToolExecutionResult {
        tool.execute_with_context(args, context).await
    }

    fn png_size(result: &ToolExecutionResult) -> (u32, u32) {
        use base64::Engine;
        let ToolExecutionResult::SuccessWithImages { images, .. } = result else {
            panic!("expected a screenshot, got {result:?}");
        };
        let png = base64::engine::general_purpose::STANDARD
            .decode(&images[0].base64)
            .unwrap();
        (
            u32::from_be_bytes(png[16..20].try_into().unwrap()),
            u32::from_be_bytes(png[20..24].try_into().unwrap()),
        )
    }

    async fn close(context: &ToolContext) {
        BrowserlessCloseBrowserTool
            .execute_with_context(json!({}), context)
            .await;
    }

    #[tokio::test]
    async fn live_computer_navigates_types_keys_and_keeps_the_browser() {
        let tool = computer();
        let ctx = context();

        let result = run(
            tool.as_ref(),
            &ctx,
            json!({"action": "navigate", "url": "https://example.com"}),
        )
        .await;
        assert_eq!(png_size(&result), (1024, 768));

        // The second call reconnects to the same persistent browser: Tab
        // focuses the page's only link, Enter follows it.
        let result = run(tool.as_ref(), &ctx, json!({"action": "key", "text": "Tab"})).await;
        assert!(result.is_success(), "{result:?}");
        let result = run(
            tool.as_ref(),
            &ctx,
            json!({"action": "key", "text": "Return"}),
        )
        .await;
        assert!(result.is_success(), "{result:?}");

        let mut session =
            everruns_integrations_browserless::session_tools::try_get_cdp_session(&ctx)
                .await
                .expect("the computer tool keeps the browser alive");
        let url = session.get_url().await.unwrap();
        session.disconnect().await;
        assert!(url.contains("iana.org"), "followed the link: {url}");

        close(&ctx).await;
    }

    #[tokio::test]
    async fn live_computer_resets_a_page_that_leaves_the_allowed_sites() {
        let tool = computer();
        let mut ctx = context();
        ctx.network_access = Some(NetworkAccessList::allow_only(["example.com"]));

        let result = run(
            tool.as_ref(),
            &ctx,
            json!({"action": "navigate", "url": "https://example.com"}),
        )
        .await;
        assert!(result.is_success(), "{result:?}");

        run(tool.as_ref(), &ctx, json!({"action": "key", "text": "Tab"})).await;
        let result = run(
            tool.as_ref(),
            &ctx,
            json!({"action": "key", "text": "Return"}),
        )
        .await;
        match result {
            ToolExecutionResult::ToolError(msg) => assert!(msg.contains("blocked"), "{msg}"),
            other => panic!("expected the guard to fire, got {other:?}"),
        }

        let result = run(
            tool.as_ref(),
            &ctx,
            json!({"action": "navigate", "url": "https://www.iana.org/"}),
        )
        .await;
        assert!(
            matches!(result, ToolExecutionResult::ToolError(_)),
            "{result:?}"
        );

        close(&ctx).await;
    }
}
