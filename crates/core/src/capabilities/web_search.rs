//! Web Search Capability - provides tools to search the web
//!
//! This capability provides web search functionality using configurable providers:
//! - Brave Search API
//! - Tavily API
//! - Exa API
//!
//! Design decisions:
//! - Provider selection is configured per-agent via capability config
//! - API keys are stored as capability secrets (encrypted, write-only)
//! - The tool requires context to access the secret store
//! - Results are returned in a unified format regardless of provider

use super::{Capability, CapabilityStatus};
use crate::tools::{Tool, ToolExecutionResult};
use crate::traits::ToolContext;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::{debug, error};

/// Web search provider types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WebSearchProvider {
    /// Brave Search API - https://brave.com/search/api/
    #[default]
    Brave,
    /// Tavily API - https://tavily.com/
    Tavily,
    /// Exa API - https://exa.ai/
    Exa,
}

impl std::fmt::Display for WebSearchProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WebSearchProvider::Brave => write!(f, "brave"),
            WebSearchProvider::Tavily => write!(f, "tavily"),
            WebSearchProvider::Exa => write!(f, "exa"),
        }
    }
}

impl std::str::FromStr for WebSearchProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "brave" => Ok(WebSearchProvider::Brave),
            "tavily" => Ok(WebSearchProvider::Tavily),
            "exa" => Ok(WebSearchProvider::Exa),
            _ => Err(format!("Unknown web search provider: {}", s)),
        }
    }
}

/// Configuration schema for the Web Search capability
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WebSearchConfig {
    /// The search provider to use
    #[serde(default)]
    pub provider: WebSearchProvider,
}

/// Secret name used for web search API key
pub const WEB_SEARCH_API_KEY_SECRET: &str = "api_key";

/// Web Search capability - provides tools to search the web
pub struct WebSearchCapability;

impl Capability for WebSearchCapability {
    fn id(&self) -> &str {
        "web_search"
    }

    fn name(&self) -> &str {
        "Web Search"
    }

    fn description(&self) -> &str {
        r#"Search the web using configurable providers (Brave Search, Tavily, or Exa).

Requires configuration:
1. Select a search provider in capability settings
2. Set the API key for the selected provider (stored securely)

Supported providers:
- **Brave Search** - Privacy-focused search with comprehensive results
- **Tavily** - AI-optimized search designed for LLM applications
- **Exa** - Neural search engine with semantic understanding"#
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn icon(&self) -> Option<&str> {
        Some("search")
    }

    fn category(&self) -> Option<&str> {
        Some("Network")
    }

    fn system_prompt_addition(&self) -> Option<&str> {
        Some(
            r#"## Web Search

You have access to web search via the `web_search` tool. Use it to find current information from the internet.

**Usage:**
- Call `web_search` with a search query to get relevant results
- Results include titles, URLs, and snippets from web pages
- Use specific, well-formed queries for best results

**Best practices:**
- Use web search for current events, recent information, or facts you're uncertain about
- Combine multiple searches to get comprehensive information
- Always cite sources when using search results in your responses"#,
        )
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(WebSearchTool)]
    }
}

// ============================================================================
// Tool: web_search
// ============================================================================

/// Tool that searches the web using the configured provider
pub struct WebSearchTool;

#[async_trait]
impl Tool for WebSearchTool {
    fn name(&self) -> &str {
        "web_search"
    }

    fn description(&self) -> &str {
        "Search the web for information. Returns a list of relevant results with titles, URLs, and snippets."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "The search query"
                },
                "count": {
                    "type": "integer",
                    "description": "Maximum number of results to return (default: 10, max: 20)",
                    "minimum": 1,
                    "maximum": 20,
                    "default": 10
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, _arguments: Value) -> ToolExecutionResult {
        ToolExecutionResult::tool_error(
            "web_search requires context. This tool must be executed with session context to access the API key.",
        )
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        context: &ToolContext,
    ) -> ToolExecutionResult {
        let query = match arguments.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.trim().is_empty() => q.trim().to_string(),
            _ => {
                return ToolExecutionResult::tool_error(
                    "Missing or empty required parameter: query",
                );
            }
        };

        let count = arguments
            .get("count")
            .and_then(|v| v.as_u64())
            .map(|c| c.clamp(1, 20) as usize)
            .unwrap_or(10);

        // Get capability config to determine provider
        let config: WebSearchConfig = context
            .capability_config
            .as_ref()
            .and_then(|c| serde_json::from_value(c.clone()).ok())
            .unwrap_or_default();

        // Get API key from capability secrets
        let api_key = match context
            .get_capability_secret(WEB_SEARCH_API_KEY_SECRET)
            .await
        {
            Ok(Some(key)) => key,
            Ok(None) => {
                return ToolExecutionResult::tool_error(
                    "Web search API key not configured. Please set the API key in capability settings.",
                );
            }
            Err(e) => {
                error!("Failed to retrieve web search API key: {}", e);
                return ToolExecutionResult::internal_error_msg(
                    "Failed to retrieve API key configuration",
                );
            }
        };

        debug!(
            "Executing web search with provider {:?}, query: {}",
            config.provider, query
        );

        // Execute search based on provider
        match config.provider {
            WebSearchProvider::Brave => search_brave(&query, count, &api_key).await,
            WebSearchProvider::Tavily => search_tavily(&query, count, &api_key).await,
            WebSearchProvider::Exa => search_exa(&query, count, &api_key).await,
        }
    }

    fn requires_context(&self) -> bool {
        true
    }
}

// ============================================================================
// Search Result Types
// ============================================================================

/// A single search result
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    /// Title of the page
    pub title: String,
    /// URL of the page
    pub url: String,
    /// Snippet or description
    pub snippet: String,
    /// Publication date if available
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_date: Option<String>,
}

/// Response from a web search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResponse {
    /// The search query
    pub query: String,
    /// Search provider used
    pub provider: String,
    /// Search results
    pub results: Vec<SearchResult>,
    /// Total number of results returned
    pub count: usize,
}

// ============================================================================
// Provider Implementations
// ============================================================================

/// Search using Brave Search API
async fn search_brave(query: &str, count: usize, api_key: &str) -> ToolExecutionResult {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to create HTTP client: {}", e);
            return ToolExecutionResult::internal_error_msg("Failed to create HTTP client");
        }
    };

    let url = format!(
        "https://api.search.brave.com/res/v1/web/search?q={}&count={}",
        urlencoding::encode(query),
        count
    );

    let response = match client
        .get(&url)
        .header("Accept", "application/json")
        .header("X-Subscription-Token", api_key)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Brave search request failed: {}", e);
            return ToolExecutionResult::tool_error(format!("Search request failed: {}", e));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        error!("Brave search API error: {} - {}", status, body);
        return ToolExecutionResult::tool_error(format!(
            "Search API error: {} - {}",
            status,
            if status.as_u16() == 401 {
                "Invalid API key"
            } else {
                "Request failed"
            }
        ));
    }

    let body: Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            error!("Failed to parse Brave search response: {}", e);
            return ToolExecutionResult::internal_error_msg("Failed to parse search response");
        }
    };

    // Parse Brave Search API response
    let mut results = Vec::new();
    if let Some(web_results) = body
        .get("web")
        .and_then(|w| w.get("results"))
        .and_then(|r| r.as_array())
    {
        for item in web_results.iter().take(count) {
            let result = SearchResult {
                title: item
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: item
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                snippet: item
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .to_string(),
                published_date: item
                    .get("age")
                    .and_then(|a| a.as_str())
                    .map(|s| s.to_string()),
            };
            if !result.url.is_empty() {
                results.push(result);
            }
        }
    }

    let response = SearchResponse {
        query: query.to_string(),
        provider: "brave".to_string(),
        count: results.len(),
        results,
    };

    ToolExecutionResult::success(
        serde_json::to_value(response).unwrap_or(json!({"error": "Failed to serialize response"})),
    )
}

/// Search using Tavily API
async fn search_tavily(query: &str, count: usize, api_key: &str) -> ToolExecutionResult {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to create HTTP client: {}", e);
            return ToolExecutionResult::internal_error_msg("Failed to create HTTP client");
        }
    };

    let request_body = json!({
        "api_key": api_key,
        "query": query,
        "max_results": count,
        "include_answer": false,
        "include_raw_content": false
    });

    let response = match client
        .post("https://api.tavily.com/search")
        .header("Content-Type", "application/json")
        .json(&request_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Tavily search request failed: {}", e);
            return ToolExecutionResult::tool_error(format!("Search request failed: {}", e));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        error!("Tavily search API error: {} - {}", status, body);
        return ToolExecutionResult::tool_error(format!(
            "Search API error: {} - {}",
            status,
            if status.as_u16() == 401 || status.as_u16() == 403 {
                "Invalid API key"
            } else {
                "Request failed"
            }
        ));
    }

    let body: Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            error!("Failed to parse Tavily search response: {}", e);
            return ToolExecutionResult::internal_error_msg("Failed to parse search response");
        }
    };

    // Parse Tavily API response
    let mut results = Vec::new();
    if let Some(tavily_results) = body.get("results").and_then(|r| r.as_array()) {
        for item in tavily_results.iter().take(count) {
            let result = SearchResult {
                title: item
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: item
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                snippet: item
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("")
                    .to_string(),
                published_date: item
                    .get("published_date")
                    .and_then(|p| p.as_str())
                    .map(|s| s.to_string()),
            };
            if !result.url.is_empty() {
                results.push(result);
            }
        }
    }

    let response = SearchResponse {
        query: query.to_string(),
        provider: "tavily".to_string(),
        count: results.len(),
        results,
    };

    ToolExecutionResult::success(
        serde_json::to_value(response).unwrap_or(json!({"error": "Failed to serialize response"})),
    )
}

/// Search using Exa API
async fn search_exa(query: &str, count: usize, api_key: &str) -> ToolExecutionResult {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            error!("Failed to create HTTP client: {}", e);
            return ToolExecutionResult::internal_error_msg("Failed to create HTTP client");
        }
    };

    let request_body = json!({
        "query": query,
        "num_results": count,
        "type": "auto",
        "use_autoprompt": true,
        "contents": {
            "text": {
                "max_characters": 500
            }
        }
    });

    let response = match client
        .post("https://api.exa.ai/search")
        .header("Content-Type", "application/json")
        .header("x-api-key", api_key)
        .json(&request_body)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            error!("Exa search request failed: {}", e);
            return ToolExecutionResult::tool_error(format!("Search request failed: {}", e));
        }
    };

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        error!("Exa search API error: {} - {}", status, body);
        return ToolExecutionResult::tool_error(format!(
            "Search API error: {} - {}",
            status,
            if status.as_u16() == 401 || status.as_u16() == 403 {
                "Invalid API key"
            } else {
                "Request failed"
            }
        ));
    }

    let body: Value = match response.json().await {
        Ok(j) => j,
        Err(e) => {
            error!("Failed to parse Exa search response: {}", e);
            return ToolExecutionResult::internal_error_msg("Failed to parse search response");
        }
    };

    // Parse Exa API response
    let mut results = Vec::new();
    if let Some(exa_results) = body.get("results").and_then(|r| r.as_array()) {
        for item in exa_results.iter().take(count) {
            let snippet = item
                .get("text")
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();

            let result = SearchResult {
                title: item
                    .get("title")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string(),
                url: item
                    .get("url")
                    .and_then(|u| u.as_str())
                    .unwrap_or("")
                    .to_string(),
                snippet,
                published_date: item
                    .get("publishedDate")
                    .and_then(|p| p.as_str())
                    .map(|s| s.to_string()),
            };
            if !result.url.is_empty() {
                results.push(result);
            }
        }
    }

    let response = SearchResponse {
        query: query.to_string(),
        provider: "exa".to_string(),
        count: results.len(),
        results,
    };

    ToolExecutionResult::success(
        serde_json::to_value(response).unwrap_or(json!({"error": "Failed to serialize response"})),
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capability_metadata() {
        let cap = WebSearchCapability;
        assert_eq!(cap.id(), "web_search");
        assert_eq!(cap.name(), "Web Search");
        assert_eq!(cap.status(), CapabilityStatus::Available);
        assert_eq!(cap.icon(), Some("search"));
        assert_eq!(cap.category(), Some("Network"));
    }

    #[test]
    fn test_capability_has_tools() {
        let cap = WebSearchCapability;
        let tools = cap.tools();

        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "web_search");
    }

    #[test]
    fn test_capability_has_system_prompt() {
        let cap = WebSearchCapability;
        let prompt = cap.system_prompt_addition().unwrap();
        assert!(prompt.contains("web_search"));
        assert!(prompt.contains("Web Search"));
    }

    #[test]
    fn test_tool_requires_context() {
        assert!(WebSearchTool.requires_context());
    }

    #[test]
    fn test_config_default() {
        let config = WebSearchConfig::default();
        assert_eq!(config.provider, WebSearchProvider::Brave);
    }

    #[test]
    fn test_config_parse() {
        let json = json!({
            "provider": "tavily"
        });
        let config: WebSearchConfig = serde_json::from_value(json).unwrap();
        assert_eq!(config.provider, WebSearchProvider::Tavily);
    }

    #[test]
    fn test_provider_parse() {
        assert_eq!(
            "brave".parse::<WebSearchProvider>().unwrap(),
            WebSearchProvider::Brave
        );
        assert_eq!(
            "tavily".parse::<WebSearchProvider>().unwrap(),
            WebSearchProvider::Tavily
        );
        assert_eq!(
            "exa".parse::<WebSearchProvider>().unwrap(),
            WebSearchProvider::Exa
        );
        assert_eq!(
            "BRAVE".parse::<WebSearchProvider>().unwrap(),
            WebSearchProvider::Brave
        );
        assert!("unknown".parse::<WebSearchProvider>().is_err());
    }

    #[test]
    fn test_provider_display() {
        assert_eq!(WebSearchProvider::Brave.to_string(), "brave");
        assert_eq!(WebSearchProvider::Tavily.to_string(), "tavily");
        assert_eq!(WebSearchProvider::Exa.to_string(), "exa");
    }

    #[tokio::test]
    async fn test_web_search_without_context() {
        let tool = WebSearchTool;
        let result = tool.execute(json!({"query": "test query"})).await;

        if let ToolExecutionResult::ToolError(msg) = result {
            assert!(msg.contains("requires context"));
        } else {
            panic!("Expected tool error");
        }
    }

    #[test]
    fn test_search_result_serialization() {
        let result = SearchResult {
            title: "Test Title".to_string(),
            url: "https://example.com".to_string(),
            snippet: "Test snippet".to_string(),
            published_date: Some("2024-01-15".to_string()),
        };

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(json["title"], "Test Title");
        assert_eq!(json["url"], "https://example.com");
        assert_eq!(json["snippet"], "Test snippet");
        assert_eq!(json["published_date"], "2024-01-15");
    }

    #[test]
    fn test_search_response_serialization() {
        let response = SearchResponse {
            query: "test".to_string(),
            provider: "brave".to_string(),
            count: 1,
            results: vec![SearchResult {
                title: "Test".to_string(),
                url: "https://example.com".to_string(),
                snippet: "Snippet".to_string(),
                published_date: None,
            }],
        };

        let json = serde_json::to_value(&response).unwrap();
        assert_eq!(json["query"], "test");
        assert_eq!(json["provider"], "brave");
        assert_eq!(json["count"], 1);
        assert!(json["results"].is_array());
    }
}
