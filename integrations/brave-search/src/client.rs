//! Brave Search API client.

use serde::{Deserialize, Serialize};
use tracing::debug;

// ----------------------------------------------------------------------------
// API Response Types
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSearchResponse {
    #[serde(default)]
    pub web: Option<WebResults>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebResults {
    #[serde(default)]
    pub results: Vec<WebResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebResult {
    pub title: String,
    pub url: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub age: Option<String>,
    #[serde(default)]
    pub language: Option<String>,
}

// ----------------------------------------------------------------------------
// BraveSearchClient
// ----------------------------------------------------------------------------

pub struct BraveSearchClient {
    http: reqwest::Client,
    api_key: String,
    api_base: String,
}

impl BraveSearchClient {
    pub fn new(api_key: String) -> Self {
        Self::with_base_url(api_key, crate::BRAVE_SEARCH_API_BASE.to_string())
    }

    pub fn with_base_url(api_key: String, api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_key,
            api_base,
        }
    }

    /// Perform a web search query.
    pub async fn web_search(
        &self,
        query: &str,
        count: Option<u32>,
        offset: Option<u32>,
        freshness: Option<&str>,
    ) -> Result<WebSearchResponse, String> {
        // Build URL with query parameters manually (reqwest .query() requires default features)
        let mut url = format!(
            "{}/web/search?q={}",
            self.api_base,
            urlencoding::encode(query)
        );
        if let Some(c) = count {
            url.push_str(&format!("&count={c}"));
        }
        if let Some(o) = offset {
            url.push_str(&format!("&offset={o}"));
        }
        if let Some(f) = freshness {
            url.push_str(&format!("&freshness={}", urlencoding::encode(f)));
        }

        debug!("Brave Search query: {query}");

        let resp = self
            .http
            .get(&url)
            .header("X-Subscription-Token", &self.api_key)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("Failed to connect to Brave Search API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("Brave Search API error ({status}): {body_text}"));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from Brave Search: {e}"))
    }
}

pub(crate) mod urlencoding {
    /// Percent-encode a string for use in query parameters.
    pub fn encode(input: &str) -> String {
        let mut result = String::with_capacity(input.len() * 2);
        for byte in input.bytes() {
            match byte {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    result.push(byte as char);
                }
                _ => {
                    result.push('%');
                    result.push_str(&format!("{byte:02X}"));
                }
            }
        }
        result
    }
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn test_web_search_success() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": {
                    "results": [
                        {
                            "title": "Rust Programming Language",
                            "url": "https://www.rust-lang.org/",
                            "description": "A language empowering everyone to build reliable software.",
                            "age": "2 hours ago"
                        },
                        {
                            "title": "Rust (programming language) - Wikipedia",
                            "url": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
                            "description": "Rust is a multi-paradigm programming language."
                        }
                    ]
                }
            })))
            .mount(&mock_server)
            .await;

        let client = BraveSearchClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client
            .web_search("rust programming", None, None, None)
            .await;
        assert!(result.is_ok());
        let response = result.unwrap();
        let results = response.web.unwrap().results;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "Rust Programming Language");
        assert_eq!(results[0].url, "https://www.rust-lang.org/");
    }

    #[tokio::test]
    async fn test_web_search_empty_results() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "web": { "results": [] }
            })))
            .mount(&mock_server)
            .await;

        let client = BraveSearchClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client
            .web_search("nonexistent query", None, None, None)
            .await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.web.unwrap().results.is_empty());
    }

    #[tokio::test]
    async fn test_web_search_no_web_field() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .mount(&mock_server)
            .await;

        let client = BraveSearchClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.web_search("test", None, None, None).await;
        assert!(result.is_ok());
        assert!(result.unwrap().web.is_none());
    }

    #[tokio::test]
    async fn test_web_search_401_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/web/search"))
            .respond_with(
                ResponseTemplate::new(401).set_body_string("{\"error\":\"Unauthorized\"}"),
            )
            .mount(&mock_server)
            .await;

        let client = BraveSearchClient::with_base_url("bad_key".to_string(), mock_server.uri());
        let result = client.web_search("test", None, None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("401"));
    }

    #[tokio::test]
    async fn test_web_search_429_rate_limit() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/web/search"))
            .respond_with(
                ResponseTemplate::new(429).set_body_string("{\"error\":\"Rate limit exceeded\"}"),
            )
            .mount(&mock_server)
            .await;

        let client = BraveSearchClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.web_search("test", None, None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("429"));
    }

    #[tokio::test]
    async fn test_web_search_malformed_json() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/web/search"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock_server)
            .await;

        let client = BraveSearchClient::with_base_url("test_key".to_string(), mock_server.uri());
        let result = client.web_search("test", None, None, None).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[test]
    fn test_web_result_deserialization() {
        let json = r#"{
            "title": "Test",
            "url": "https://example.com",
            "description": "A test result",
            "age": "1 day ago",
            "language": "en"
        }"#;
        let result: WebResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.title, "Test");
        assert_eq!(result.url, "https://example.com");
        assert_eq!(result.description, "A test result");
        assert_eq!(result.age, Some("1 day ago".to_string()));
        assert_eq!(result.language, Some("en".to_string()));
    }

    #[test]
    fn test_web_result_minimal_deserialization() {
        let json = r#"{"title": "Test", "url": "https://example.com"}"#;
        let result: WebResult = serde_json::from_str(json).unwrap();
        assert_eq!(result.title, "Test");
        assert_eq!(result.description, "");
        assert_eq!(result.age, None);
    }

    #[test]
    fn test_web_search_response_deserialization() {
        let json = r#"{
            "web": {
                "results": [
                    {"title": "A", "url": "https://a.com", "description": "desc a"},
                    {"title": "B", "url": "https://b.com", "description": "desc b"}
                ]
            }
        }"#;
        let response: WebSearchResponse = serde_json::from_str(json).unwrap();
        let results = response.web.unwrap().results;
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].title, "A");
        assert_eq!(results[1].url, "https://b.com");
    }
}
