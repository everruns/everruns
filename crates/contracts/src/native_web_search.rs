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
        if let Some(location) = self.approximate_location() {
            parameters["user_location"] = location;
        }
        Some(parameters)
    }

    /// Anthropic's `web_search` server tool for this request, or `None` when
    /// Anthropic cannot honor it: it takes allowed or blocked domains, never
    /// both in one request.
    ///
    /// Decision: the basic `web_search_20250305` version. The later versions
    /// (`_20260209`, `_20260318`) run searches from inside code execution
    /// (dynamic filtering), which is not ZDR eligible by default and needs
    /// models with programmatic tool calling; the basic one runs everywhere
    /// Claude has web search. Source: Anthropic's web search tool guide
    /// (2026-10).
    ///
    /// Decision: without an agent cap, at most
    /// [`ANTHROPIC_DEFAULT_MAX_USES`] searches per generation. The API runs
    /// searches in a server-side loop that pauses (`pause_turn`) when it runs
    /// long; the driver does not continue a paused turn, so a cap well under
    /// that loop's limit keeps a generation from ending mid-search.
    pub fn anthropic_tool(&self) -> Option<Value> {
        let allowed = !self.allowed_domains.is_empty();
        let blocked = !self.blocked_domains.is_empty();
        if allowed && blocked {
            return None;
        }
        let mut tool = serde_json::json!({
            "type": ANTHROPIC_WEB_SEARCH_TOOL_TYPE,
            "name": WEB_SEARCH_FUNCTION_TOOL,
            "max_uses": self.max_uses.unwrap_or(ANTHROPIC_DEFAULT_MAX_USES),
        });
        if allowed {
            tool["allowed_domains"] = serde_json::json!(self.allowed_domains);
        }
        if blocked {
            tool["blocked_domains"] = serde_json::json!(self.blocked_domains);
        }
        if let Some(location) = self.approximate_location() {
            tool["user_location"] = location;
        }
        Some(tool)
    }

    /// Gemini's Grounding with Google Search tool for this request, or `None`
    /// when Gemini cannot honor it: grounding takes no domain filters, allowed
    /// or blocked. It takes no location or search cap either; those are
    /// dropped, as the freshness hint is everywhere.
    pub fn gemini_tool(&self) -> Option<Value> {
        (self.allowed_domains.is_empty() && self.blocked_domains.is_empty())
            .then(|| serde_json::json!({ "google_search": {} }))
    }

    /// `user_location` as `{"type": "approximate", ...}`, or `None` when no
    /// field is set (Anthropic and OpenRouter reject an empty location).
    fn approximate_location(&self) -> Option<Value> {
        let mut wire = serde_json::to_value(self.user_location.as_ref()?).ok()?;
        if wire.as_object().is_none_or(|fields| fields.is_empty()) {
            return None;
        }
        wire["type"] = Value::from("approximate");
        Some(wire)
    }
}

/// Anthropic's web search server tool version (see
/// [`NativeWebSearch::anthropic_tool`]).
pub const ANTHROPIC_WEB_SEARCH_TOOL_TYPE: &str = "web_search_20250305";

/// Searches per generation on Anthropic when the agent sets no cap.
pub const ANTHROPIC_DEFAULT_MAX_USES: u32 = 5;

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

/// Whether a Claude model takes Anthropic's web search tool: Claude 3.5
/// Haiku, 3.7 Sonnet, and every Claude 4 and later model. Takes first-party
/// ids (`claude-sonnet-4-5-20250929`, `claude-3-5-haiku-20241022`,
/// `claude-opus-5-5[1m]`) and OpenRouter names (`claude-3.7-sonnet`,
/// `claude-sonnet-4.5`), with or without an `anthropic/` prefix. Source: the
/// web search tool guide and tool reference (2026-10).
pub fn anthropic_has_native_web_search(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    let Some(rest) = model.strip_prefix("claude-") else {
        return false;
    };
    let Some(digits) = rest.find(|c: char| c.is_ascii_digit()) else {
        return false;
    };
    match version(&rest[digits..]) {
        Some((major, _)) if major >= 4 => true,
        Some((3, Some(5))) => rest.contains("haiku"),
        Some((3, Some(7))) => rest.contains("sonnet"),
        _ => false,
    }
}

/// Whether a Gemini model takes Grounding with Google Search (the
/// `google_search` tool) on the Gemini API: Gemini 2.0 Flash, the 2.5 family
/// and every Gemini 3 and later model, conservatively leaving out the 2.0
/// Flash-Lite, speech, live, embedding and image generation variants of the
/// 2.x line. Older models only take the retired `google_search_retrieval`.
/// Source: the Grounding with Google Search guide (2026-10).
pub fn gemini_has_native_web_search(model: &str) -> bool {
    let model = model.rsplit('/').next().unwrap_or(model);
    let Some(rest) = model.strip_prefix("gemini-") else {
        return false;
    };
    if ["embedding", "tts", "live", "audio"]
        .iter()
        .any(|variant| rest.contains(variant))
    {
        return false;
    }
    match version(rest) {
        Some((major, _)) if major >= 3 => true,
        Some((2, Some(5))) => !rest.contains("image"),
        Some((2, Some(0))) => {
            rest.starts_with("2.0-flash") && !rest.contains("lite") && !rest.contains("image")
        }
        _ => false,
    }
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
            anthropic_has_native_web_search(name).then_some(NativeSearchVendor::Anthropic)
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
    fn anthropic_models_with_web_search() {
        for model in [
            "claude-opus-5-5",
            "claude-haiku-5-5",
            "claude-opus-5-5[1m]",
            "claude-sonnet-4-5-20250929",
            "claude-opus-4-1-20250805",
            "claude-sonnet-4-20250514",
            "claude-3-7-sonnet-20250219",
            "claude-3-5-haiku-20241022",
            "anthropic/claude-sonnet-4.5",
        ] {
            assert!(anthropic_has_native_web_search(model), "{model}");
        }
        for model in [
            "claude-3-5-sonnet-20241022",
            "claude-3-opus-20240229",
            "claude-3-haiku-20240307",
            "claude-2.1",
            "gpt-5",
            "claude",
        ] {
            assert!(!anthropic_has_native_web_search(model), "{model}");
        }
    }

    #[test]
    fn gemini_models_with_google_search() {
        for model in [
            "gemini-3.8-flash",
            "gemini-3.5-flash-lite",
            "gemini-3.1-pro-preview",
            "gemini-3-flash-preview",
            "gemini-2.5-pro",
            "gemini-2.5-flash-lite",
            "gemini-2.0-flash",
            "models/gemini-2.5-flash",
        ] {
            assert!(gemini_has_native_web_search(model), "{model}");
        }
        for model in [
            "gemini-2.0-flash-lite",
            "gemini-1.5-pro",
            "gemini-2.5-flash-preview-tts",
            "gemini-2.5-flash-image",
            "gemini-embedding-001",
            "gemini-live-2.5-flash",
            "gpt-5",
        ] {
            assert!(!gemini_has_native_web_search(model), "{model}");
        }
    }

    #[test]
    fn anthropic_tool_takes_one_kind_of_domain_filter() {
        assert_eq!(
            NativeWebSearch::default().anthropic_tool(),
            Some(json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": 5 }))
        );
        let blocked = NativeWebSearch {
            blocked_domains: vec!["reddit.com".into()],
            max_uses: Some(2),
            user_location: Some(WebSearchUserLocation {
                city: Some("Kyiv".into()),
                country: Some("UA".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(
            blocked.anthropic_tool(),
            Some(json!({
                "type": "web_search_20250305", "name": "web_search", "max_uses": 2,
                "blocked_domains": ["reddit.com"],
                "user_location": { "type": "approximate", "city": "Kyiv", "country": "UA" }
            }))
        );
        let allowed = NativeWebSearch {
            allowed_domains: vec!["docs.rs".into()],
            user_location: Some(WebSearchUserLocation::default()),
            ..Default::default()
        };
        // An empty location is left out rather than sent empty.
        assert_eq!(
            allowed.anthropic_tool(),
            Some(
                json!({ "type": "web_search_20250305", "name": "web_search", "max_uses": 5,
                         "allowed_domains": ["docs.rs"] })
            )
        );
        let both = NativeWebSearch {
            allowed_domains: vec!["docs.rs".into()],
            blocked_domains: vec!["reddit.com".into()],
            ..Default::default()
        };
        assert_eq!(both.anthropic_tool(), None);
    }

    #[test]
    fn gemini_tool_declines_any_domain_filter() {
        let located = NativeWebSearch {
            max_uses: Some(3),
            freshness: Some("week".into()),
            user_location: Some(WebSearchUserLocation {
                country: Some("UA".into()),
                ..Default::default()
            }),
            ..Default::default()
        };
        assert_eq!(located.gemini_tool(), Some(json!({ "google_search": {} })));
        for filtered in [
            NativeWebSearch {
                allowed_domains: vec!["docs.rs".into()],
                ..Default::default()
            },
            NativeWebSearch {
                blocked_domains: vec!["reddit.com".into()],
                ..Default::default()
            },
        ] {
            assert_eq!(filtered.gemini_tool(), None);
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
