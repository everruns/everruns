//! Browserless API client.
//!
//! Decision: REST-only approach. Each API call spins up a fresh browser, performs one
//! operation, and tears down. No persistent browser sessions to leak.
//! Decision: /function endpoint for interactions (click, type) since Browserless
//! REST APIs don't expose standalone click/type endpoints.

use serde_json::{Value, json};
use tracing::debug;

// ============================================================================
// BrowserlessClient - HTTP client for Browserless REST APIs
// ============================================================================

pub struct BrowserlessClient {
    http: reqwest::Client,
    api_token: String,
    api_base: String,
}

impl BrowserlessClient {
    pub fn new(api_token: String) -> Self {
        Self::with_base_url(api_token, crate::browserless_api_base())
    }

    pub fn with_base_url(api_token: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_token,
            api_base,
        }
    }

    /// Build URL with token query parameter.
    fn endpoint(&self, path: &str) -> String {
        format!("{}{}?token={}", self.api_base, path, self.api_token)
    }

    // --- Screenshot API ---

    /// Take a screenshot of a URL. Returns PNG bytes.
    pub async fn screenshot(
        &self,
        url: &str,
        full_page: bool,
        selector: Option<&str>,
        wait_for_selector: Option<&str>,
        wait_for_timeout: Option<u64>,
        cookies: &[Value],
    ) -> Result<Vec<u8>, String> {
        let mut body = json!({
            "url": url,
            "options": {
                "fullPage": full_page,
                "type": "png"
            }
        });

        if let Some(sel) = selector {
            body["selector"] = json!(sel);
        }
        if let Some(wfs) = wait_for_selector {
            body["waitForSelector"] = json!({"selector": wfs, "timeout": 30000});
        }
        if let Some(timeout) = wait_for_timeout {
            body["waitForTimeout"] = json!(timeout);
        }
        if !cookies.is_empty() {
            body["cookies"] = json!(cookies);
        }

        debug!("Browserless screenshot: {url}");

        let resp = self
            .http
            .post(self.endpoint("/screenshot"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Browserless API: {e}"))?;

        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!("Browserless API error ({status}): {body_text}"));
        }

        resp.bytes()
            .await
            .map(|b| b.to_vec())
            .map_err(|e| format!("Failed to read screenshot bytes: {e}"))
    }

    // --- Content API ---

    /// Get fully rendered HTML content of a URL.
    pub async fn content(
        &self,
        url: &str,
        wait_for_selector: Option<&str>,
        wait_for_timeout: Option<u64>,
        best_attempt: bool,
        cookies: &[Value],
    ) -> Result<String, String> {
        let mut body = json!({ "url": url });

        if let Some(wfs) = wait_for_selector {
            body["waitForSelector"] = json!({"selector": wfs, "timeout": 30000});
        }
        if let Some(timeout) = wait_for_timeout {
            body["waitForTimeout"] = json!(timeout);
        }
        if best_attempt {
            body["bestAttempt"] = json!(true);
        }
        if !cookies.is_empty() {
            body["cookies"] = json!(cookies);
        }

        debug!("Browserless content: {url}");

        let resp = self
            .http
            .post(self.endpoint("/content"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Browserless API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Browserless API error ({status}): {body_text}"));
        }

        Ok(body_text)
    }

    // --- Scrape API ---

    /// Scrape structured data from a URL using CSS selectors.
    pub async fn scrape(
        &self,
        url: &str,
        elements: &[Value],
        wait_for_selector: Option<&str>,
        wait_for_timeout: Option<u64>,
        cookies: &[Value],
    ) -> Result<Value, String> {
        let mut body = json!({
            "url": url,
            "elements": elements
        });

        if let Some(wfs) = wait_for_selector {
            body["waitForSelector"] = json!({"selector": wfs, "timeout": 30000});
        }
        if let Some(timeout) = wait_for_timeout {
            body["waitForTimeout"] = json!(timeout);
        }
        if !cookies.is_empty() {
            body["cookies"] = json!(cookies);
        }

        debug!("Browserless scrape: {url}");

        let resp = self
            .http
            .post(self.endpoint("/scrape"))
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Browserless API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Browserless API error ({status}): {body_text}"));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from Browserless: {e}"))
    }

    // --- Function API ---

    /// Execute custom Puppeteer code. Used for multi-step interactions
    /// (click, type, keyboard, mouse, touch, then screenshot/read DOM).
    pub async fn function(&self, code: &str, context: Option<&Value>) -> Result<Value, String> {
        let mut body = json!({ "code": code });
        if let Some(ctx) = context {
            body["context"] = ctx.clone();
        }

        debug!("Browserless function call");

        let resp = self
            .http
            .post(self.endpoint("/function"))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Browserless API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Browserless API error ({status}): {body_text}"));
        }

        if body_text.is_empty() {
            return Ok(json!({}));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from Browserless: {e}"))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_client_screenshot() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/screenshot"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PNG_BYTES".to_vec()))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .screenshot("https://example.com", true, None, None, None, &[])
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), b"PNG_BYTES");
    }

    #[tokio::test]
    async fn test_client_content() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/content"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("<html><body>Hello</body></html>"),
            )
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .content("https://example.com", None, None, false, &[])
            .await;
        assert!(result.is_ok());
        assert!(result.unwrap().contains("Hello"));
    }

    #[tokio::test]
    async fn test_client_scrape() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/scrape"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"results": [{"text": "found"}]}]
            })))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let elements = vec![json!({"selector": "h1"})];
        let result = client
            .scrape("https://example.com", &elements, None, None, &[])
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_function() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/function"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": "clicked",
                "type": "application/json"
            })))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .function(
                "export default async ({ page }) => { return { data: 'clicked', type: 'application/json' }; }",
                None,
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_screenshot_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/screenshot"))
            .respond_with(ResponseTemplate::new(400).set_body_string("Bad Request"))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .screenshot("https://example.com", false, None, None, None, &[])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("400"));
    }

    #[tokio::test]
    async fn test_client_content_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/content"))
            .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("bad_token".to_string(), mock_server.uri());
        let result = client
            .content("https://example.com", None, None, false, &[])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("401"));
    }

    #[tokio::test]
    async fn test_client_scrape_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/scrape"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Error"))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .scrape("https://example.com", &[], None, None, &[])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("500"));
    }

    #[tokio::test]
    async fn test_client_function_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/function"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Script error"))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client.function("bad code", None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("500"));
    }

    #[tokio::test]
    async fn test_client_function_empty_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/function"))
            .respond_with(ResponseTemplate::new(200).set_body_string(""))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .function("export default async ({ page }) => {}", None)
            .await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), json!({}));
    }

    #[tokio::test]
    async fn test_client_screenshot_with_options() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/screenshot"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"PNG".to_vec()))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .screenshot(
                "https://example.com",
                false,
                Some("#main"),
                Some("h1"),
                Some(5000),
                &[],
            )
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_content_with_options() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/content"))
            .respond_with(ResponseTemplate::new(200).set_body_string("<html></html>"))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .content("https://example.com", Some("body"), Some(3000), true, &[])
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_client_scrape_malformed_json() {
        let mock_server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/scrape"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not valid json"))
            .mount(&mock_server)
            .await;

        let client = BrowserlessClient::with_base_url("test_token".to_string(), mock_server.uri());
        let result = client
            .scrape("https://example.com", &[], None, None, &[])
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }
}
