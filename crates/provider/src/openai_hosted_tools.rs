//! OpenAI hosted (provider-executed) tools for the Responses API.
//!
//! Hosted tools run inside OpenAI: the model calls them and OpenAI feeds the
//! result back within the same response, so the agent loop never dispatches
//! them. A capability contributes the selected tools as request intent under
//! [`OPENAI_HOSTED_TOOLS_OPTION`] in `LlmCallConfig::driver_options`, and the
//! Responses driver appends them to the request `tools` array.
//!
//! Only endpoints that implement OpenAI's hosted tools render them (the OpenAI
//! and Azure OpenAI drivers). Every other driver must reject the option rather
//! than drop it, so an agent never silently loses a tool it was configured
//! with. `web_search` ships first; code interpreter, file search and remote
//! MCP are follow-ups (EVE-1115).
//!
//! Wire shape: <https://platform.openai.com/docs/guides/tools-web-search>.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// `driver_options` key carrying [`OpenAiHostedTools`].
pub const OPENAI_HOSTED_TOOLS_OPTION: &str = "openai/hosted_tools";

/// Driver ids whose Responses endpoint executes OpenAI hosted tools.
pub const HOSTED_TOOLS_DRIVER_IDS: [&str; 2] = ["openai", "azure_openai"];

/// Hosted tools selected for one call.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct OpenAiHostedTools {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub web_search: Option<WebSearchTool>,
}

impl OpenAiHostedTools {
    pub fn is_empty(&self) -> bool {
        self.web_search.is_none()
    }

    /// Read the option from a call's `driver_options`. A malformed payload is
    /// an error, not "no tools": the caller asked for hosted tools.
    pub fn from_driver_options(
        options: &HashMap<String, Value>,
    ) -> Result<Option<Self>, serde_json::Error> {
        options
            .get(OPENAI_HOSTED_TOOLS_OPTION)
            .map(|raw| serde_json::from_value::<Self>(raw.clone()))
            .transpose()
            .map(|tools| tools.filter(|tools| !tools.is_empty()))
    }

    /// The `(key, value)` pair a capability contributes, or `None` when empty.
    pub fn to_driver_option(&self) -> Option<(String, Value)> {
        if self.is_empty() {
            return None;
        }
        let value = serde_json::to_value(self).ok()?;
        Some((OPENAI_HOSTED_TOOLS_OPTION.to_string(), value))
    }

    /// Responses `tools` entries, in a stable order.
    pub fn wire_tools(&self) -> Vec<Value> {
        self.web_search
            .iter()
            .map(WebSearchTool::wire)
            .collect::<Vec<_>>()
    }
}

/// How much retrieved context the model may use per search.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SearchContextSize {
    Low,
    Medium,
    High,
}

impl SearchContextSize {
    pub const ALL: [Self; 3] = [Self::Low, Self::Medium, Self::High];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }
}

/// Approximate location used to localize search results.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchUserLocation {
    /// Two-letter ISO country code, e.g. `US`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub region: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// IANA time zone, e.g. `America/Chicago`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
}

impl WebSearchUserLocation {
    fn is_empty(&self) -> bool {
        self.country.is_none()
            && self.region.is_none()
            && self.city.is_none()
            && self.timezone.is_none()
    }
}

/// OpenAI's hosted `web_search` tool.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WebSearchTool {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search_context_size: Option<SearchContextSize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<WebSearchUserLocation>,
    /// Restrict results to these domains (no scheme, e.g. `openai.com`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
}

impl WebSearchTool {
    /// The Responses `tools` entry for this tool.
    pub fn wire(&self) -> Value {
        let mut tool = serde_json::json!({ "type": "web_search" });
        if let Some(size) = self.search_context_size {
            tool["search_context_size"] = Value::from(size.as_str());
        }
        if let Some(location) = self.user_location.as_ref().filter(|l| !l.is_empty()) {
            let mut wire = serde_json::to_value(location).unwrap_or_default();
            wire["type"] = Value::from("approximate");
            tool["user_location"] = wire;
        }
        if !self.allowed_domains.is_empty() {
            tool["filters"] = serde_json::json!({ "allowed_domains": self.allowed_domains });
        }
        tool
    }
}

/// Count hosted tool calls in a terminal Responses `output` array, keyed by
/// item type (`web_search_call`, ...). Hosted calls bill per call on top of
/// tokens, so this is what cost accounting prices.
pub fn count_hosted_tool_calls(output: &[Value]) -> HashMap<String, u32> {
    let mut counts = HashMap::new();
    for item in output {
        if let Some(kind) = item.get("type").and_then(Value::as_str)
            && HOSTED_CALL_ITEM_TYPES.contains(&kind)
        {
            *counts.entry(kind.to_string()).or_insert(0) += 1;
        }
    }
    counts
}

/// Output item types OpenAI emits for hosted tool calls.
const HOSTED_CALL_ITEM_TYPES: [&str; 1] = ["web_search_call"];

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bare_web_search_renders_the_minimal_wire_tool() {
        let tools = OpenAiHostedTools {
            web_search: Some(WebSearchTool::default()),
        };
        assert_eq!(tools.wire_tools(), vec![json!({ "type": "web_search" })]);
    }

    #[test]
    fn web_search_options_render_in_openai_shape() {
        let tool = WebSearchTool {
            search_context_size: Some(SearchContextSize::High),
            user_location: Some(WebSearchUserLocation {
                country: Some("UA".into()),
                city: Some("Kyiv".into()),
                ..Default::default()
            }),
            allowed_domains: vec!["openai.com".into()],
        };
        assert_eq!(
            tool.wire(),
            json!({
                "type": "web_search",
                "search_context_size": "high",
                "user_location": { "type": "approximate", "country": "UA", "city": "Kyiv" },
                "filters": { "allowed_domains": ["openai.com"] },
            })
        );
    }

    #[test]
    fn empty_location_is_omitted() {
        let tool = WebSearchTool {
            user_location: Some(WebSearchUserLocation::default()),
            ..Default::default()
        };
        assert_eq!(tool.wire(), json!({ "type": "web_search" }));
    }

    #[test]
    fn driver_option_round_trips_and_empty_contributes_nothing() {
        assert!(OpenAiHostedTools::default().to_driver_option().is_none());

        let tools = OpenAiHostedTools {
            web_search: Some(WebSearchTool {
                search_context_size: Some(SearchContextSize::Low),
                ..Default::default()
            }),
        };
        let (key, value) = tools.to_driver_option().expect("non-empty");
        assert_eq!(key, OPENAI_HOSTED_TOOLS_OPTION);
        let options = HashMap::from([(key, value)]);
        assert_eq!(
            OpenAiHostedTools::from_driver_options(&options).unwrap(),
            Some(tools)
        );
        assert_eq!(
            OpenAiHostedTools::from_driver_options(&HashMap::new()).unwrap(),
            None
        );
    }

    #[test]
    fn malformed_option_is_an_error_not_silence() {
        let options = HashMap::from([(
            OPENAI_HOSTED_TOOLS_OPTION.to_string(),
            json!({ "web_search": { "search_context_size": "huge" } }),
        )]);
        assert!(OpenAiHostedTools::from_driver_options(&options).is_err());
    }

    #[test]
    fn counts_only_hosted_call_items() {
        let output = vec![
            json!({ "type": "web_search_call", "id": "ws_1", "status": "completed" }),
            json!({ "type": "reasoning", "id": "rs_1" }),
            json!({ "type": "web_search_call", "id": "ws_2", "status": "completed" }),
            json!({ "type": "message", "id": "msg_1" }),
        ];
        let counts = count_hosted_tool_calls(&output);
        assert_eq!(counts.len(), 1);
        assert_eq!(counts["web_search_call"], 2);
    }
}
