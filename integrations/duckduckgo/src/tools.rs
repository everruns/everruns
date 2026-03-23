//! Tool implementations for DuckDuckGo operations.

use everruns_core::ToolHints;
use everruns_core::tools::{Tool, ToolExecutionResult};
use everruns_core::traits::ToolContext;

use async_trait::async_trait;
use serde_json::{Value, json};

use crate::client::DuckDuckGoClient;

// ============================================================================
// DuckDuckGoSearchTool
// ============================================================================

pub struct DuckDuckGoSearchTool;

#[async_trait]
impl Tool for DuckDuckGoSearchTool {
    fn name(&self) -> &str {
        "duckduckgo_search"
    }

    fn description(&self) -> &str {
        "Get instant answers from DuckDuckGo. Returns abstracts (from Wikipedia and other sources), \
         definitions, direct answers, and related topics. \
         No API key required."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query string"
                },
                "no_html": {
                    "type": "boolean",
                    "description": "Strip HTML from result text (default: true)",
                    "default": true
                }
            },
            "required": ["query"],
            "additionalProperties": false
        })
    }

    fn hints(&self) -> ToolHints {
        ToolHints::default()
            .with_readonly(true)
            .with_idempotent(true)
            .with_open_world(true)
    }

    async fn execute(&self, arguments: Value) -> ToolExecutionResult {
        let query = match arguments.get("query").and_then(|v| v.as_str()) {
            Some(q) if !q.is_empty() => q,
            _ => {
                return ToolExecutionResult::tool_error("Missing required parameter: query");
            }
        };

        let no_html = arguments
            .get("no_html")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let client = DuckDuckGoClient::new();

        match client.instant_answer(query, no_html).await {
            Ok(response) => format_response(query, &response),
            Err(e) => ToolExecutionResult::tool_error(e),
        }
    }

    async fn execute_with_context(
        &self,
        arguments: Value,
        _context: &ToolContext,
    ) -> ToolExecutionResult {
        // No context needed — DuckDuckGo API is free, no API key required.
        self.execute(arguments).await
    }

    fn requires_context(&self) -> bool {
        false
    }
}

/// Format the DuckDuckGo API response into the tool result.
fn format_response(
    query: &str,
    response: &crate::client::InstantAnswerResponse,
) -> ToolExecutionResult {
    let mut result = json!({
        "query": query,
        "type": match response.response_type.as_str() {
            "A" => "article",
            "D" => "disambiguation",
            "C" => "category",
            "N" => "name",
            "E" => "exclusive",
            "" => "nothing",
            other => other,
        },
    });

    // Abstract (Wikipedia-style summary)
    if !response.r#abstract.is_empty() {
        result["abstract"] = json!({
            "text": response.r#abstract,
            "source": response.abstract_source,
            "url": response.abstract_url,
        });
    }

    // Heading
    if !response.heading.is_empty() {
        result["heading"] = json!(response.heading);
    }

    // Direct answer (calculations, IP, etc.)
    if !response.answer.is_empty() {
        result["answer"] = json!({
            "text": response.answer,
            "type": response.answer_type,
        });
    }

    // Definition
    if !response.definition.is_empty() {
        result["definition"] = json!({
            "text": response.definition,
            "source": response.definition_source,
            "url": response.definition_url,
        });
    }

    // Related topics (flatten groups into a single list, limit to keep output reasonable)
    let mut topics: Vec<Value> = Vec::new();
    for topic in &response.related_topics {
        if topic.is_direct() {
            topics.push(json!({
                "text": topic.text,
                "url": topic.first_url,
            }));
        } else if topic.is_group() {
            for nested in &topic.topics {
                if nested.is_direct() {
                    topics.push(json!({
                        "text": nested.text,
                        "url": nested.first_url,
                    }));
                }
            }
        }
        if topics.len() >= 10 {
            break;
        }
    }
    if !topics.is_empty() {
        result["related_topics"] = json!(topics);
    }

    // Official results
    let results: Vec<Value> = response
        .results
        .iter()
        .filter(|r| r.is_direct())
        .map(|r| {
            json!({
                "text": r.text,
                "url": r.first_url,
            })
        })
        .collect();
    if !results.is_empty() {
        result["results"] = json!(results);
    }

    ToolExecutionResult::success(result)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_tool_name() {
        let tool = DuckDuckGoSearchTool;
        assert_eq!(tool.name(), "duckduckgo_search");
    }

    #[test]
    fn test_tool_has_description() {
        let tool = DuckDuckGoSearchTool;
        assert!(!tool.description().is_empty());
        assert!(tool.description().contains("DuckDuckGo"));
    }

    #[test]
    fn test_tool_does_not_require_context() {
        let tool = DuckDuckGoSearchTool;
        assert!(!tool.requires_context());
    }

    #[test]
    fn test_search_schema_has_required_query() {
        let tool = DuckDuckGoSearchTool;
        let schema = tool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        let required_strs: Vec<&str> = required.iter().map(|v| v.as_str().unwrap()).collect();
        assert!(required_strs.contains(&"query"));
        assert_eq!(required_strs.len(), 1);
    }

    #[test]
    fn test_search_schema_has_no_html_field() {
        let tool = DuckDuckGoSearchTool;
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["no_html"].is_object());
    }

    #[test]
    fn test_search_schema_disallows_additional_properties() {
        let tool = DuckDuckGoSearchTool;
        let schema = tool.parameters_schema();
        assert_eq!(schema["additionalProperties"], json!(false));
    }

    #[tokio::test]
    async fn test_missing_query() {
        let tool = DuckDuckGoSearchTool;
        let result = tool.execute(json!({})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("Missing required parameter"),
                    "Expected missing param error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for missing query"),
        }
    }

    #[tokio::test]
    async fn test_empty_query() {
        let tool = DuckDuckGoSearchTool;
        let result = tool.execute(json!({"query": ""})).await;
        match result {
            ToolExecutionResult::ToolError(msg) => {
                assert!(
                    msg.contains("Missing required parameter"),
                    "Expected missing param error, got: {msg}"
                );
            }
            _ => panic!("Expected ToolError for empty query"),
        }
    }

    #[test]
    fn test_format_response_article() {
        let response = crate::client::InstantAnswerResponse {
            r#abstract: "Rust is a programming language.".to_string(),
            abstract_source: "Wikipedia".to_string(),
            abstract_url: "https://en.wikipedia.org/wiki/Rust".to_string(),
            answer: String::new(),
            answer_type: String::new(),
            definition: String::new(),
            definition_source: String::new(),
            definition_url: String::new(),
            heading: "Rust (programming language)".to_string(),
            response_type: "A".to_string(),
            related_topics: vec![crate::client::TopicResult {
                text: "Cargo".to_string(),
                first_url: "https://doc.rust-lang.org/cargo/".to_string(),
                name: String::new(),
                topics: vec![],
            }],
            results: vec![crate::client::TopicResult {
                text: "Official site".to_string(),
                first_url: "https://www.rust-lang.org/".to_string(),
                name: String::new(),
                topics: vec![],
            }],
        };

        let result = format_response("rust", &response);
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["query"], "rust");
                assert_eq!(val["type"], "article");
                assert_eq!(val["heading"], "Rust (programming language)");
                assert_eq!(val["abstract"]["text"], "Rust is a programming language.");
                assert_eq!(val["abstract"]["source"], "Wikipedia");
                assert_eq!(val["related_topics"].as_array().unwrap().len(), 1);
                assert_eq!(val["results"].as_array().unwrap().len(), 1);
            }
            _ => panic!("Expected Success result"),
        }
    }

    #[test]
    fn test_format_response_empty() {
        let response = crate::client::InstantAnswerResponse {
            r#abstract: String::new(),
            abstract_source: String::new(),
            abstract_url: String::new(),
            answer: String::new(),
            answer_type: String::new(),
            definition: String::new(),
            definition_source: String::new(),
            definition_url: String::new(),
            heading: String::new(),
            response_type: String::new(),
            related_topics: vec![],
            results: vec![],
        };

        let result = format_response("nonexistent", &response);
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["query"], "nonexistent");
                assert_eq!(val["type"], "nothing");
                assert!(val.get("abstract").is_none());
                assert!(val.get("heading").is_none());
                assert!(val.get("answer").is_none());
                assert!(val.get("related_topics").is_none());
                assert!(val.get("results").is_none());
            }
            _ => panic!("Expected Success result"),
        }
    }

    #[test]
    fn test_format_response_with_answer() {
        let response = crate::client::InstantAnswerResponse {
            r#abstract: String::new(),
            abstract_source: String::new(),
            abstract_url: String::new(),
            answer: "42".to_string(),
            answer_type: "calc".to_string(),
            definition: String::new(),
            definition_source: String::new(),
            definition_url: String::new(),
            heading: String::new(),
            response_type: String::new(),
            related_topics: vec![],
            results: vec![],
        };

        let result = format_response("6*7", &response);
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["answer"]["text"], "42");
                assert_eq!(val["answer"]["type"], "calc");
            }
            _ => panic!("Expected Success result"),
        }
    }

    #[test]
    fn test_format_response_with_definition() {
        let response = crate::client::InstantAnswerResponse {
            r#abstract: String::new(),
            abstract_source: String::new(),
            abstract_url: String::new(),
            answer: String::new(),
            answer_type: String::new(),
            definition: "A test is an assessment.".to_string(),
            definition_source: "Wiktionary".to_string(),
            definition_url: "https://en.wiktionary.org/wiki/test".to_string(),
            heading: "Test".to_string(),
            response_type: String::new(),
            related_topics: vec![],
            results: vec![],
        };

        let result = format_response("test", &response);
        match result {
            ToolExecutionResult::Success(val) => {
                assert_eq!(val["definition"]["text"], "A test is an assessment.");
                assert_eq!(val["definition"]["source"], "Wiktionary");
            }
            _ => panic!("Expected Success result"),
        }
    }

    #[test]
    fn test_format_response_flattens_topic_groups() {
        let response = crate::client::InstantAnswerResponse {
            r#abstract: String::new(),
            abstract_source: String::new(),
            abstract_url: String::new(),
            answer: String::new(),
            answer_type: String::new(),
            definition: String::new(),
            definition_source: String::new(),
            definition_url: String::new(),
            heading: String::new(),
            response_type: "D".to_string(),
            related_topics: vec![
                crate::client::TopicResult {
                    text: "Direct topic".to_string(),
                    first_url: "https://example.com/1".to_string(),
                    name: String::new(),
                    topics: vec![],
                },
                crate::client::TopicResult {
                    text: String::new(),
                    first_url: String::new(),
                    name: "Category".to_string(),
                    topics: vec![crate::client::TopicResult {
                        text: "Nested topic".to_string(),
                        first_url: "https://example.com/2".to_string(),
                        name: String::new(),
                        topics: vec![],
                    }],
                },
            ],
            results: vec![],
        };

        let result = format_response("test", &response);
        match result {
            ToolExecutionResult::Success(val) => {
                let topics = val["related_topics"].as_array().unwrap();
                assert_eq!(topics.len(), 2);
                assert_eq!(topics[0]["text"], "Direct topic");
                assert_eq!(topics[1]["text"], "Nested topic");
            }
            _ => panic!("Expected Success result"),
        }
    }
}
