//! Shared tool protocol and operation used by both execution contexts.

use crate::client::BraveSearchClient;
use everruns_capability::definition::schemars::{self, JsonSchema};
use serde::Deserialize;
use serde_json::{Value, json};

pub(crate) const TOOL_NAME: &str = "brave_web_search";
pub(crate) const TOOL_DESCRIPTION: &str = "Search the web using Brave Search. Returns relevant web results including titles, URLs, and descriptions.";

/// Input to the Brave web-search tool.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchInput {
    pub query: String,
    pub count: Option<u64>,
    pub offset: Option<u32>,
    pub freshness: Option<String>,
}

impl JsonSchema for SearchInput {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "BraveWebSearchInput".into()
    }
    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schema().try_into().expect("search schema is an object")
    }
}

pub(crate) fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "query": {
                "type": "string",
                "description": "Search query string"
            },
            "count": {
                "type": "integer",
                "description": "Number of results to return (1-20, default: 10)",
                "minimum": 1,
                "maximum": 20
            },
            "offset": {
                "type": "integer",
                "description": "Pagination offset (default: 0)",
                "minimum": 0
            },
            "freshness": {
                "type": "string",
                "enum": ["pd", "pw", "pm", "py"],
                "description": "Time filter: pd (past day), pw (past week), pm (past month), py (past year)"
            }
        },
        "required": ["query"],
        "additionalProperties": false
    })
}

pub(crate) async fn search(
    client: &BraveSearchClient,
    input: SearchInput,
) -> Result<Value, String> {
    if input.query.is_empty() {
        return Err("Missing required parameter: query".into());
    }
    if input
        .freshness
        .as_deref()
        .is_some_and(|value| !["pd", "pw", "pm", "py"].contains(&value))
    {
        return Err("Invalid freshness: expected pd, pw, pm, or py".into());
    }
    let response = client
        .web_search(
            &input.query,
            input.count.map(|count| count.clamp(1, 20) as u32),
            input.offset,
            input.freshness.as_deref(),
        )
        .await?;
    let results: Vec<Value> = response
        .web
        .map(|web| web.results)
        .unwrap_or_default()
        .into_iter()
        .map(|result| {
            let mut item = json!({
                "title": result.title,
                "url": result.url,
                "description": result.description,
            });
            if let Some(age) = result.age {
                item["age"] = json!(age);
            }
            item
        })
        .collect();
    Ok(json!({"query": input.query, "count": results.len(), "results": results}))
}
