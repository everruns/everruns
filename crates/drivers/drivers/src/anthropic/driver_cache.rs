//! Prompt-cache controls for the Messages API request.
//!
//! Breakpoints are placed while the request is built (the tool and system
//! conversions, and `driver_layout.rs` for messages). The requested retention is
//! applied afterwards in one pass, so every breakpoint carries the same TTL.
//! Anthropic requires longer-TTL breakpoints to come before shorter ones; a
//! uniform TTL satisfies that by construction.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AnthropicRequest, AnthropicSystem, AnthropicSystemBlock, AnthropicToolEntry};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct AnthropicCacheControl {
    pub(super) r#type: String,
    /// `"1h"` for the extended TTL; absent means the 5 minute default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ttl: Option<String>,
}

impl AnthropicCacheControl {
    pub(super) fn ephemeral() -> Self {
        Self {
            r#type: "ephemeral".to_string(),
            ttl: None,
        }
    }
}

/// `usage.cache_creation`: cache writes split by TTL.
#[derive(Debug, Default, Deserialize)]
pub(super) struct AnthropicCacheCreation {
    #[serde(default)]
    pub(super) ephemeral_1h_input_tokens: Option<u32>,
}

/// Set `ttl` on every cache breakpoint in the request. `None` keeps the
/// provider default and leaves the request untouched.
pub(super) fn apply_ttl(request: &mut AnthropicRequest, ttl: Option<&str>) {
    let Some(ttl) = ttl else {
        return;
    };
    let set = |control: &mut Option<AnthropicCacheControl>| {
        if let Some(control) = control {
            control.ttl = Some(ttl.to_string());
        }
    };
    set(&mut request.cache_control);
    if let Some(AnthropicSystem::Blocks(blocks)) = &mut request.system {
        for AnthropicSystemBlock::Text { cache_control, .. } in blocks {
            set(cache_control);
        }
    }
    for entry in request.tools.iter_mut().flatten() {
        match entry {
            AnthropicToolEntry::Function(tool) => set(&mut tool.cache_control),
            AnthropicToolEntry::Raw(raw) => set_raw_ttl(raw, ttl),
            AnthropicToolEntry::Search(_) => {}
        }
    }
    // Messages are already wire JSON. Only content blocks carry breakpoints;
    // anything deeper (tool inputs, tool results) is caller data.
    for message in &mut request.messages {
        if let Some(blocks) = message.get_mut("content").and_then(Value::as_array_mut) {
            blocks.iter_mut().for_each(|block| set_raw_ttl(block, ttl));
        }
    }
}

fn set_raw_ttl(entry: &mut Value, ttl: &str) {
    if let Some(control) = entry
        .get_mut("cache_control")
        .and_then(Value::as_object_mut)
    {
        control.insert("ttl".to_string(), Value::String(ttl.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn empty_request() -> AnthropicRequest {
        AnthropicRequest {
            model: "claude-sonnet-5-5".into(),
            messages: Vec::new(),
            max_tokens: 16,
            temperature: None,
            system: None,
            stream: true,
            tools: None,
            tool_choice: None,
            thinking: None,
            output_config: None,
            diagnostics: None,
            context_management: None,
            cache_control: None,
        }
    }

    #[test]
    fn ttl_reaches_every_breakpoint_and_nothing_else() {
        let mut request: AnthropicRequest = empty_request();
        request.cache_control = Some(AnthropicCacheControl::ephemeral());
        request.system = Some(AnthropicSystem::Blocks(vec![AnthropicSystemBlock::Text {
            text: "stable".into(),
            cache_control: Some(AnthropicCacheControl::ephemeral()),
        }]));
        request.tools = Some(vec![AnthropicToolEntry::Raw(
            json!({"type":"computer_20250124","cache_control":{"type":"ephemeral"}}),
        )]);
        request.messages = vec![json!({"role":"user","content":[
            {"type":"text","text":"a","cache_control":{"type":"ephemeral"}},
            {"type":"tool_result","tool_use_id":"t","content":[{"type":"text","text":"x"}],
             "input":{"cache_control":{"type":"ephemeral"}}}
        ]})];

        apply_ttl(&mut request, Some("1h"));
        let body = serde_json::to_value(&request).unwrap();

        assert_eq!(body["cache_control"]["ttl"], "1h");
        assert_eq!(body["system"][0]["cache_control"]["ttl"], "1h");
        assert_eq!(body["tools"][0]["cache_control"]["ttl"], "1h");
        assert_eq!(
            body["messages"][0]["content"][0]["cache_control"]["ttl"],
            "1h"
        );
        // Caller data that merely looks like a breakpoint is left alone.
        assert!(body["messages"][0]["content"][1]["input"]["cache_control"]["ttl"].is_null());
    }

    #[test]
    fn default_ttl_leaves_the_request_unchanged() {
        let mut request: AnthropicRequest = empty_request();
        request.cache_control = Some(AnthropicCacheControl::ephemeral());
        apply_ttl(&mut request, None);
        let body = serde_json::to_value(&request).unwrap();
        assert_eq!(body["cache_control"], json!({"type":"ephemeral"}));
    }
}
