//! DuckDuckGo Instant Answer API client.

use serde::{Deserialize, Serialize};
use tracing::debug;

// ============================================================================
// API Response Types
// ============================================================================

/// Top-level response from DuckDuckGo Instant Answer API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstantAnswerResponse {
    /// Topic summary (typically from Wikipedia).
    #[serde(rename = "Abstract", default)]
    pub r#abstract: String,

    /// Source of the abstract (e.g. "Wikipedia").
    #[serde(rename = "AbstractSource", default)]
    pub abstract_source: String,

    /// URL of the abstract source.
    #[serde(rename = "AbstractURL", default)]
    pub abstract_url: String,

    /// Instant answer text.
    #[serde(rename = "Answer", default)]
    pub answer: String,

    /// Type of answer (e.g. "calc", "color", "ip", etc.).
    #[serde(rename = "AnswerType", default)]
    pub answer_type: String,

    /// Definition text.
    #[serde(rename = "Definition", default)]
    pub definition: String,

    /// Source of the definition.
    #[serde(rename = "DefinitionSource", default)]
    pub definition_source: String,

    /// URL for the definition source.
    #[serde(rename = "DefinitionURL", default)]
    pub definition_url: String,

    /// Topic heading.
    #[serde(rename = "Heading", default)]
    pub heading: String,

    /// Response type: A (article), D (disambiguation), C (category),
    /// N (name), E (exclusive), or empty.
    #[serde(rename = "Type", default)]
    pub response_type: String,

    /// Related topics.
    #[serde(rename = "RelatedTopics", default)]
    pub related_topics: Vec<TopicResult>,

    /// Official results (e.g. official website).
    #[serde(rename = "Results", default)]
    pub results: Vec<TopicResult>,
}

/// A topic result entry — can be either a direct result or a group.
///
/// DuckDuckGo API returns two shapes: direct results with `Text`/`FirstURL`,
/// and topic groups with `Name`/`Topics`. We deserialize both into this struct
/// and distinguish by checking which fields are populated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TopicResult {
    /// Display text (may contain HTML).
    #[serde(rename = "Text", default)]
    pub text: String,

    /// URL for this topic.
    #[serde(rename = "FirstURL", default)]
    pub first_url: String,

    /// Group name (for grouped/categorized topics).
    #[serde(rename = "Name", default)]
    pub name: String,

    /// Nested topics (for grouped results).
    #[serde(rename = "Topics", default)]
    pub topics: Vec<TopicResult>,
}

impl TopicResult {
    /// Whether this is a direct result (has text and URL).
    pub fn is_direct(&self) -> bool {
        !self.first_url.is_empty()
    }

    /// Whether this is a topic group (has nested topics).
    pub fn is_group(&self) -> bool {
        !self.topics.is_empty()
    }
}

// ============================================================================
// DuckDuckGoClient
// ============================================================================

pub struct DuckDuckGoClient {
    http: reqwest::Client,
    api_base: String,
}

impl Default for DuckDuckGoClient {
    fn default() -> Self {
        Self::new()
    }
}

impl DuckDuckGoClient {
    pub fn new() -> Self {
        Self::with_base_url(crate::DUCKDUCKGO_API_BASE.to_string())
    }

    pub fn with_base_url(api_base: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            api_base,
        }
    }

    /// Query the DuckDuckGo Instant Answer API.
    pub async fn instant_answer(
        &self,
        query: &str,
        no_html: bool,
    ) -> Result<InstantAnswerResponse, String> {
        let mut url = format!(
            "{}/?q={}&format=json&no_redirect=1",
            self.api_base,
            urlencoding::encode(query)
        );
        if no_html {
            url.push_str("&no_html=1");
        }

        debug!("DuckDuckGo query: {query}");

        let resp = self
            .http
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| format!("Failed to connect to DuckDuckGo API: {e}"))?;

        let status = resp.status();
        let body_text = resp
            .text()
            .await
            .map_err(|e| format!("Failed to read response: {e}"))?;

        if !status.is_success() {
            return Err(format!("DuckDuckGo API error ({status}): {body_text}"));
        }

        serde_json::from_str(&body_text).map_err(|e| format!("Invalid JSON from DuckDuckGo: {e}"))
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

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn sample_response() -> serde_json::Value {
        serde_json::json!({
            "Abstract": "Rust is a multi-paradigm programming language.",
            "AbstractSource": "Wikipedia",
            "AbstractURL": "https://en.wikipedia.org/wiki/Rust_(programming_language)",
            "Answer": "",
            "AnswerType": "",
            "Definition": "",
            "DefinitionSource": "",
            "DefinitionURL": "",
            "Heading": "Rust (programming language)",
            "Type": "A",
            "RelatedTopics": [
                {
                    "Text": "Cargo is the Rust package manager.",
                    "FirstURL": "https://doc.rust-lang.org/cargo/",
                    "Name": "",
                    "Topics": []
                }
            ],
            "Results": [
                {
                    "Text": "Official Rust website",
                    "FirstURL": "https://www.rust-lang.org/",
                    "Name": "",
                    "Topics": []
                }
            ]
        })
    }

    #[tokio::test]
    async fn test_instant_answer_success() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock_server)
            .await;

        let client = DuckDuckGoClient::with_base_url(mock_server.uri());
        let result = client.instant_answer("rust programming", false).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert_eq!(response.heading, "Rust (programming language)");
        assert_eq!(response.response_type, "A");
        assert!(response.r#abstract.contains("Rust"));
        assert_eq!(response.abstract_source, "Wikipedia");
        assert_eq!(response.related_topics.len(), 1);
        assert_eq!(response.results.len(), 1);
    }

    #[tokio::test]
    async fn test_instant_answer_empty_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "Abstract": "",
                "AbstractSource": "",
                "AbstractURL": "",
                "Answer": "",
                "AnswerType": "",
                "Definition": "",
                "DefinitionSource": "",
                "DefinitionURL": "",
                "Heading": "",
                "Type": "",
                "RelatedTopics": [],
                "Results": []
            })))
            .mount(&mock_server)
            .await;

        let client = DuckDuckGoClient::with_base_url(mock_server.uri());
        let result = client.instant_answer("asdfghjkl", false).await;
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.r#abstract.is_empty());
        assert!(response.heading.is_empty());
        assert!(response.related_topics.is_empty());
    }

    #[tokio::test]
    async fn test_instant_answer_with_no_html() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_json(sample_response()))
            .mount(&mock_server)
            .await;

        let client = DuckDuckGoClient::with_base_url(mock_server.uri());
        let result = client.instant_answer("rust", true).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_instant_answer_server_error() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(500).set_body_string("Internal Server Error"))
            .mount(&mock_server)
            .await;

        let client = DuckDuckGoClient::with_base_url(mock_server.uri());
        let result = client.instant_answer("test", false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("500"));
    }

    #[tokio::test]
    async fn test_instant_answer_malformed_json() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock_server)
            .await;

        let client = DuckDuckGoClient::with_base_url(mock_server.uri());
        let result = client.instant_answer("test", false).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid JSON"));
    }

    #[test]
    fn test_topic_result_is_direct() {
        let direct = TopicResult {
            text: "Some text".to_string(),
            first_url: "https://example.com".to_string(),
            name: String::new(),
            topics: vec![],
        };
        assert!(direct.is_direct());
        assert!(!direct.is_group());
    }

    #[test]
    fn test_topic_result_is_group() {
        let group = TopicResult {
            text: String::new(),
            first_url: String::new(),
            name: "Category".to_string(),
            topics: vec![TopicResult {
                text: "Nested".to_string(),
                first_url: "https://example.com".to_string(),
                name: String::new(),
                topics: vec![],
            }],
        };
        assert!(group.is_group());
        assert!(!group.is_direct());
    }

    #[test]
    fn test_response_deserialization() {
        let json = r#"{
            "Abstract": "Test abstract",
            "AbstractSource": "Wikipedia",
            "AbstractURL": "https://en.wikipedia.org/wiki/Test",
            "Answer": "42",
            "AnswerType": "calc",
            "Definition": "A test",
            "DefinitionSource": "Wiktionary",
            "DefinitionURL": "https://en.wiktionary.org/wiki/test",
            "Heading": "Test",
            "Type": "A",
            "RelatedTopics": [],
            "Results": []
        }"#;
        let response: InstantAnswerResponse = serde_json::from_str(json).unwrap();
        assert_eq!(response.r#abstract, "Test abstract");
        assert_eq!(response.answer, "42");
        assert_eq!(response.answer_type, "calc");
        assert_eq!(response.heading, "Test");
        assert_eq!(response.response_type, "A");
    }

    #[test]
    fn test_response_deserialization_minimal() {
        // DuckDuckGo API always returns all fields, but with defaults
        let json = r#"{}"#;
        let response: InstantAnswerResponse = serde_json::from_str(json).unwrap();
        assert!(response.r#abstract.is_empty());
        assert!(response.heading.is_empty());
        assert!(response.related_topics.is_empty());
        assert!(response.results.is_empty());
    }
}
