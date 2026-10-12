//! Native (model provider) web search on the shared `web_search` tool.
//!
//! The `web_search` capability asks for the model's own search by contributing
//! [`NATIVE_WEB_SEARCH_OPTION`] in `LlmCallConfig::driver_options` while
//! native search comes first in the agent's search plan. A driver that can run
//! its provider's hosted search for the call's model swaps the `web_search`
//! function tool for that hosted tool; the provider runs the searches inside
//! the generation and the answer carries its citations.
//!
//! Decision: like [`crate::native_computer`], the option is a request, not a
//! requirement. A driver that cannot honor it for this model (no hosted
//! search, a model without it, or domain filters the provider cannot apply)
//! leaves the function tool in place, and the next provider of the plan
//! answers when the model calls it. Every other driver ignores the option.
//!
//! Decision: blocked domains are a policy, so a provider that cannot exclude
//! domains declines rather than searching them. Freshness is a ranking hint
//! no native search takes; it never makes a driver decline.
//!
//! Decision: the option carries the agent's provider setting, not a verdict.
//! Whether native comes first also depends on the organization's current web
//! search defaults (`allow_native`, the default provider, the Automatic
//! order), which a host applies to each turn before the call
//! (`runtime::web_search::resolve_native_web_search`). A host without
//! organization settings (Framework, serve) keeps the built-in defaults.
//!
//! Decision: which models search natively is a model-id rule here, one rule
//! per vendor, as for the native computer tool: it changes with vendor
//! launches and is not worth a profile field on every model.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::driver_registry::LlmCallConfig;
use crate::openai_hosted_tools::{WebSearchTool, WebSearchUserLocation};

/// `driver_options` key carrying [`NativeWebSearch`].
pub const NATIVE_WEB_SEARCH_OPTION: &str = "everruns/web_search";

/// The provider-neutral function tool a native search stands in for.
pub const WEB_SEARCH_FUNCTION_TOOL: &str = "web_search";

/// Request for the model provider's own search on this call.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NativeWebSearch {
    /// The agent's provider setting (`org_default`, `auto` or `native`), which
    /// the host resolves against the organization's defaults each turn.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub provider: String,
    /// Only results from these sites (bare host names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    /// Never results from these sites, the organization's included.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_domains: Vec<String>,
    /// `day`, `week`, `month` or `year`. A hint no native search applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<String>,
    /// Most searches the model may run in one generation, where the provider
    /// takes a cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_uses: Option<u32>,
    /// Approximate location to localize results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_location: Option<WebSearchUserLocation>,
}

impl NativeWebSearch {
    /// The `(key, value)` pair a capability contributes.
    pub fn to_driver_option(&self) -> (String, Value) {
        (
            NATIVE_WEB_SEARCH_OPTION.to_string(),
            serde_json::to_value(self).unwrap_or(Value::Null),
        )
    }

    /// Read the option. A malformed payload means "not requested": the
    /// function tool still works, so this never fails a call.
    pub fn from_driver_options(options: &std::collections::HashMap<String, Value>) -> Option<Self> {
        options
            .get(NATIVE_WEB_SEARCH_OPTION)
            .and_then(|raw| serde_json::from_value(raw.clone()).ok())
    }

    /// The native search request for this call: the option is set and the
    /// call offers the `web_search` function tool it stands in for.
    pub fn requested(config: &LlmCallConfig) -> Option<Self> {
        let native = Self::from_driver_options(&config.driver_options)?;
        config
            .tools
            .iter()
            .any(|tool| tool.name() == WEB_SEARCH_FUNCTION_TOOL)
            .then_some(native)
    }

    /// OpenAI's Responses `web_search` tool for this request, or `None` when
    /// OpenAI cannot honor it: it takes allowed domains but cannot exclude any.
    pub fn openai_tool(&self) -> Option<WebSearchTool> {
        self.blocked_domains.is_empty().then(|| WebSearchTool {
            search_context_size: None,
            user_location: self.user_location.clone(),
            allowed_domains: self.allowed_domains.clone(),
        })
    }

    /// Parameters of OpenRouter's `openrouter:web_search` server tool on its
    /// native engine for `model` (an OpenRouter slug), or `None` when
    /// OpenRouter would not run the vendor's own search for it.
    ///
    /// OpenRouter falls back to its own engine (Exa) for a model without
    /// native search, even with `engine: native`, and no request field turns
    /// that off. Sending the tool only for models on its native list is what
    /// keeps the fallback with the search plan.
    pub fn openrouter_parameters(&self, model: &str) -> Option<Value> {
        let vendor = openrouter_native_vendor(model)?;
        let allowed = !self.allowed_domains.is_empty();
        let blocked = !self.blocked_domains.is_empty();
        let honored = match vendor {
            NativeSearchVendor::OpenAi => !blocked,
            NativeSearchVendor::Google => !allowed && !blocked,
            // Allowed and excluded domains are mutually exclusive there.
            NativeSearchVendor::Anthropic
            | NativeSearchVendor::XAi
            | NativeSearchVendor::Perplexity => !(allowed && blocked),
        };
        if !honored {
            return None;
        }
        let mut parameters = serde_json::json!({ "engine": "native" });
        if allowed {
            parameters["allowed_domains"] = serde_json::json!(self.allowed_domains);
        }
        if blocked {
            parameters["excluded_domains"] = serde_json::json!(self.blocked_domains);
        }
        if let Some(max_uses) = self.max_uses {
            parameters["max_uses"] = Value::from(max_uses);
        }
        if let Some(location) = &self.user_location {
            let mut wire = serde_json::to_value(location).unwrap_or_default();
            if wire.as_object().is_some_and(|fields| !fields.is_empty()) {
                wire["type"] = Value::from("approximate");
                parameters["user_location"] = wire;
            }
        }
        Some(parameters)
    }
}

/// Whose search a native call runs, for domain-filter support and pricing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeSearchVendor {
    OpenAi,
    Anthropic,
    Google,
    XAi,
    Perplexity,
}

/// Leading `major` (and optional `.minor`/`-minor`) version in `rest`.
fn version(rest: &str) -> Option<(u32, Option<u32>)> {
    let mut parts = rest.split(['.', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|minor| minor.parse().ok());
    Some((major, minor))
}

/// Whether an OpenAI model takes the Responses `web_search` tool: GPT-4o,
/// GPT-4.1 (not nano), GPT-5 and later, o3 and o4-mini. Source: OpenAI's web
/// search tool guide (2026-10).
pub fn openai_has_native_web_search(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    if let Some(rest) = model.strip_prefix("gpt-") {
        return match version(rest) {
            Some((major, _)) if major >= 5 => true,
            Some((4, Some(1))) => !model.starts_with("gpt-4.1-nano"),
            _ => model.starts_with("gpt-4o"),
        };
    }
    model == "o3" || model.starts_with("o3-pro") || model.starts_with("o4-mini")
}

/// The vendor whose own search OpenRouter runs for `model`, from OpenRouter's
/// native search list (2026-10), or `None` when OpenRouter would answer with
/// its fallback engine instead.
pub fn openrouter_native_vendor(model: &str) -> Option<NativeSearchVendor> {
    let model = model.split(':').next().unwrap_or(model);
    let (vendor, name) = model.split_once('/')?;
    match vendor {
        "openai" => {
            let rest = name.strip_prefix("gpt-");
            let native = match rest.and_then(version) {
                Some((major, _)) if major >= 5 => true,
                // GPT-4o is not on the list: OpenRouter falls back to Exa.
                Some((4, Some(1))) => true,
                _ => name == "o3" || name.starts_with("o3-pro") || name.starts_with("o4-mini"),
            };
            native.then_some(NativeSearchVendor::OpenAi)
        }
        "anthropic" => {
            let rest = name.strip_prefix("claude-")?;
            // `claude-3.7-sonnet`, `claude-sonnet-4.5`, `claude-opus-5-5`.
            let digits = rest.find(|c: char| c.is_ascii_digit())?;
            let native = match version(&rest[digits..])? {
                (major, _) if major >= 4 => true,
                (3, Some(5)) => rest.contains("haiku"),
                (3, Some(7)) => rest.contains("sonnet"),
                _ => false,
            };
            native.then_some(NativeSearchVendor::Anthropic)
        }
        "google" => version(name.strip_prefix("gemini-")?)
            .is_some_and(|(major, _)| major >= 3)
            .then_some(NativeSearchVendor::Google),
        "x-ai" => version(name.strip_prefix("grok-")?)
            .is_some_and(|(major, _)| major >= 4)
            .then_some(NativeSearchVendor::XAi),
        "perplexity" => Some(NativeSearchVendor::Perplexity),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, ToolDefinition};
    use serde_json::json;

    fn tool(name: &str) -> ToolDefinition {
        ToolDefinition::Builtin(BuiltinTool {
            name: name.to_string(),
            display_name: None,
            description: String::new(),
            parameters: json!({}),
            policy: Default::default(),
            category: None,
            deferrable: Default::default(),
            hints: Default::default(),
            full_parameters: None,
        })
    }

    #[test]
    fn requested_needs_the_option_and_the_web_search_tool() {
        let mut config = LlmCallConfig::new("gpt-6.1-sol");
        config.tools = vec![tool("web_search")];
        assert_eq!(NativeWebSearch::requested(&config), None);
        let (key, value) = NativeWebSearch::default().to_driver_option();
        config.driver_options.insert(key, value);
        assert_eq!(
            NativeWebSearch::requested(&config),
            Some(NativeWebSearch::default())
        );
        config.tools = vec![tool("web_fetch")];
        assert_eq!(NativeWebSearch::requested(&config), None);
        // A malformed option is not a request.
        config.tools = vec![tool("web_search")];
        config.driver_options.insert(
            NATIVE_WEB_SEARCH_OPTION.into(),
            json!({ "max_uses": "many" }),
        );
        assert_eq!(NativeWebSearch::requested(&config), None);
    }

    #[test]
    fn openai_declines_blocked_domains_and_keeps_allowed_ones() {
        let native = NativeWebSearch {
            allowed_domains: vec!["docs.rs".into()],
            ..Default::default()
        };
        assert_eq!(
            native.openai_tool().unwrap().wire(),
            json!({ "type": "web_search", "filters": { "allowed_domains": ["docs.rs"] } })
        );
        let blocked = NativeWebSearch {
            blocked_domains: vec!["pinterest.com".into()],
            ..Default::default()
        };
        assert_eq!(blocked.openai_tool(), None);
    }

    #[test]
    fn openai_models_with_web_search() {
        for model in [
            "gpt-6.1-sol",
            "gpt-6-luna",
            "gpt-5",
            "gpt-5.4-mini",
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4o",
            "o3",
            "o4-mini",
        ] {
            assert!(openai_has_native_web_search(model), "{model}");
        }
        for model in [
            "gpt-4.1-nano",
            "gpt-4-turbo",
            "gpt-3.5-turbo",
            "o3-mini",
            "o1",
        ] {
            assert!(!openai_has_native_web_search(model), "{model}");
        }
    }

    #[test]
    fn openrouter_native_list() {
        let cases = [
            ("openai/gpt-5.2", Some(NativeSearchVendor::OpenAi)),
            ("openai/gpt-4.1-mini", Some(NativeSearchVendor::OpenAi)),
            ("openai/gpt-4o", None),
            ("openai/o4-mini", Some(NativeSearchVendor::OpenAi)),
            (
                "anthropic/claude-sonnet-4.5",
                Some(NativeSearchVendor::Anthropic),
            ),
            (
                "anthropic/claude-opus-5-5",
                Some(NativeSearchVendor::Anthropic),
            ),
            (
                "anthropic/claude-3.7-sonnet",
                Some(NativeSearchVendor::Anthropic),
            ),
            (
                "anthropic/claude-3.5-haiku",
                Some(NativeSearchVendor::Anthropic),
            ),
            ("anthropic/claude-3.5-sonnet", None),
            ("google/gemini-3-flash", Some(NativeSearchVendor::Google)),
            ("google/gemini-2.5-pro", None),
            ("x-ai/grok-4", Some(NativeSearchVendor::XAi)),
            ("x-ai/grok-3", None),
            ("perplexity/sonar", Some(NativeSearchVendor::Perplexity)),
            ("mistralai/mistral-large", None),
            (
                "anthropic/claude-sonnet-4.5:online",
                Some(NativeSearchVendor::Anthropic),
            ),
            ("gpt-5.2", None),
        ];
        for (model, expected) in cases {
            assert_eq!(openrouter_native_vendor(model), expected, "{model}");
        }
    }

    #[test]
    fn openrouter_parameters_follow_vendor_domain_support() {
        let both = NativeWebSearch {
            allowed_domains: vec!["arxiv.org".into()],
            blocked_domains: vec!["reddit.com".into()],
            max_uses: Some(3),
            ..Default::default()
        };
        assert_eq!(
            both.openrouter_parameters("anthropic/claude-opus-5-5"),
            None
        );
        assert_eq!(both.openrouter_parameters("openai/gpt-5.2"), None);
        let blocked = NativeWebSearch {
            blocked_domains: vec!["reddit.com".into()],
            max_uses: Some(3),
            ..Default::default()
        };
        assert_eq!(
            blocked.openrouter_parameters("anthropic/claude-opus-5-5"),
            Some(json!({ "engine": "native", "excluded_domains": ["reddit.com"], "max_uses": 3 }))
        );
        assert_eq!(blocked.openrouter_parameters("google/gemini-3-pro"), None);
        assert_eq!(
            NativeWebSearch::default().openrouter_parameters("google/gemini-3-pro"),
            Some(json!({ "engine": "native" }))
        );
        assert_eq!(
            NativeWebSearch::default().openrouter_parameters("mistralai/mistral-large"),
            None
        );
        let located = NativeWebSearch {
            user_location: Some(WebSearchUserLocation {
                country: Some("UA".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            located.openrouter_parameters("openai/gpt-5.2").unwrap()["user_location"],
            json!({ "type": "approximate", "country": "UA" })
        );
    }
}
