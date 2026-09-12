// Auto Tool Search Capability
//
// A model-adaptive dispatcher over the three real tool-search mechanisms:
//
//   - `openai_tool_search` (hosted): on models with native OpenAI tool_search
//     support (GPT-5.4+), the LLM driver hides parameter schemas server-side via
//     namespaces + defer_loading. No client-side tool is added.
//   - `claude_tool_search` (hosted): on Claude models with native Anthropic
//     tool_search support (Sonnet 4 / Opus 4 / Haiku 4.5 / Fable 5 and newer),
//     the Anthropic driver marks tools `defer_loading: true` and adds a hosted
//     `tool_search_tool_*_20251119` entry. No client-side tool is added.
//   - `tool_search` (generic, client-side): on every other model (Gemini, OpenAI
//     Completions, Claude/GPT reached via a gateway that masks the hosted format,
//     ...), a `DeferSchemaHook` strips schemas and a `tool_search` tool loads
//     them back on demand.
//
// Unlike picking one of those capabilities by hand, this one chooses at runtime.
// It OWNS the two hosted capabilities and the generic one and implements
// `Capability::resolve_for_model`: capability collection knows the agent's model
// (via `SystemPromptContext::model`) and delegates to whichever inner capability
// fits. Only that one capability's contributions are collected — the hosted
// config for a hosted one, or the hook + tool + system prompt for the generic
// one. No "contribute both, prune later" step is needed.
//
// Use this instead of the individual capabilities when a harness must work well
// across providers.

use super::claude_tool_search::{
    ClaudeToolSearchCapability,
    model_supports_native_tool_search as model_supports_native_claude_tool_search,
};
use super::openai_tool_search::{
    OpenAiToolSearchCapability,
    model_supports_native_tool_search as model_supports_native_openai_tool_search,
};
use super::tool_search::ToolSearchCapability;
use super::{Capability, CapabilityLocalization, CapabilityStatus};

pub use super::openai_tool_search::DEFAULT_TOOL_SEARCH_THRESHOLD;

/// Capability ID for the model-adaptive tool search.
pub const AUTO_TOOL_SEARCH_CAPABILITY_ID: &str = "auto_tool_search";

/// Auto Tool Search capability.
///
/// Holds the three real tool-search capabilities and dispatches to one of them
/// based on the agent's model. `threshold` (minimum number of tools before
/// deferral activates) is shared by all and forwarded at construction.
pub struct AutoToolSearchCapability {
    openai: OpenAiToolSearchCapability,
    claude: ClaudeToolSearchCapability,
    generic: ToolSearchCapability,
}

impl AutoToolSearchCapability {
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_TOOL_SEARCH_THRESHOLD)
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            openai: OpenAiToolSearchCapability::with_threshold(threshold),
            claude: ClaudeToolSearchCapability::with_threshold(threshold),
            generic: ToolSearchCapability::with_threshold(threshold),
        }
    }

    /// Keep the named tools' full schemas under the generic (client-side)
    /// mechanism. Forwarded to the inner [`ToolSearchCapability`]; the hosted
    /// OpenAI path is unaffected (use `DeferrablePolicy::Never` there). See
    /// [`ToolSearchCapability::with_never_defer`].
    pub fn with_never_defer<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.generic = self.generic.with_never_defer(names);
        self
    }
}

impl Default for AutoToolSearchCapability {
    fn default() -> Self {
        Self::new()
    }
}

impl Capability for AutoToolSearchCapability {
    fn id(&self) -> &str {
        AUTO_TOOL_SEARCH_CAPABILITY_ID
    }

    fn name(&self) -> &str {
        "Auto Tool Search"
    }

    fn description(&self) -> &str {
        "Model-adaptive deferred tool loading. Uses the provider's hosted \
         tool_search on models that support it (OpenAI GPT-5.4+ and Claude \
         Sonnet 4 / Opus 4 / Haiku 4.5 / Fable 5 and newer) and a \
         provider-agnostic client-side fallback on every other model. Reduces \
         token usage for agents with many tools, regardless of provider."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Автоматичний пошук інструментів",
            "Відкладене завантаження інструментів, що адаптується до моделі. Використовує хостований tool_search провайдера на моделях, які його підтримують (OpenAI GPT-5.4+ та Claude Sonnet 4 / Opus 4 / Haiku 4.5 / Fable 5 і новіші), та незалежний від провайдера клієнтський резервний механізм на всіх інших моделях. Зменшує використання токенів для агентів із багатьма інструментами незалежно від провайдера.",
        )]
    }

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    // The dispatch itself: capability collection calls this with the agent's
    // model and collects the resolved capability's contributions in place of this
    // one's. Models with native OpenAI or Anthropic support get the matching
    // hosted mechanism (no client-side tool or hook); everything else — including
    // an unknown model — gets the provider-agnostic client-side mechanism, which
    // is safe everywhere. OpenAI and Anthropic profiles never both claim the same
    // model id, so the order between the two hosted checks is immaterial.
    fn resolve_for_model(&self, model: Option<&str>) -> Option<&dyn Capability> {
        match model {
            Some(m) if model_supports_native_openai_tool_search(m) => Some(&self.openai),
            Some(m) if model_supports_native_claude_tool_search(m) => Some(&self.claude),
            _ => Some(&self.generic),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tool_types::{BuiltinTool, DeferrablePolicy, ToolDefinition, ToolHints, ToolPolicy};
    use serde_json::json;

    #[test]
    fn model_dispatch_preserves_hosted_configuration_or_generic_deferral_behavior() {
        let cap = AutoToolSearchCapability::with_threshold(2).with_never_defer(["keep"]);
        for (model, expected_id) in [
            (None, "tool_search"),
            (Some("unknown-model"), "tool_search"),
            (Some("claude-3-5-haiku"), "tool_search"),
            (Some("gpt-5.4"), "openai_tool_search"),
            (Some("claude-opus-4-8"), "claude_tool_search"),
        ] {
            let resolved = cap.resolve_for_model(model).expect("resolved capability");
            assert_eq!(resolved.id(), expected_id, "model={model:?}");
            if expected_id != "tool_search" {
                for (config, threshold) in [(json!({}), 2), (json!({"threshold":4}), 4)] {
                    let actual = resolved
                        .tool_search_config(&config)
                        .expect("hosted configuration");
                    assert!(actual.enabled);
                    assert_eq!(actual.threshold, threshold);
                }
                assert!(resolved.tools().is_empty());
                assert!(resolved.tool_definition_hooks().is_empty());
            } else {
                assert!(resolved.tool_search_config(&json!({})).is_none());
                assert_eq!(
                    resolved
                        .tools()
                        .iter()
                        .map(|t| t.name())
                        .collect::<Vec<_>>(),
                    vec!["tool_search"]
                );
                let hooks = resolved.tool_definition_hooks();
                assert_eq!(hooks.len(), 1);
                assert!(!hooks[0].applies_with_native_tool_search());
                let tools:Vec<_>=["keep","defer"].into_iter().map(|name|ToolDefinition::Builtin(BuiltinTool {
                    name:name.into(),display_name:None,description:format!("{name} description"),parameters:json!({"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}),policy:ToolPolicy::Auto,category:None,deferrable:DeferrablePolicy::Automatic,hints:ToolHints::default(),full_parameters:None,
                })).collect();
                // Below the forwarded threshold, even a non-allowlisted schema survives.
                assert_eq!(
                    serde_json::to_value(hooks[0].transform(vec![tools[1].clone()])).unwrap(),
                    serde_json::to_value(vec![tools[1].clone()]).unwrap()
                );
                let actual = hooks[0].transform(tools.clone());
                let mut expected = tools;
                let ToolDefinition::Builtin(deferred) = &mut expected[1] else {
                    unreachable!()
                };
                deferred.full_parameters = Some(deferred.parameters.clone());
                deferred.parameters = json!({"type":"object","additionalProperties":true});
                assert_eq!(
                    serde_json::to_value(actual).unwrap(),
                    serde_json::to_value(expected).unwrap()
                );
            }
        }
    }
}
