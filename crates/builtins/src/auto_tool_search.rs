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

use everruns_core::capabilities::claude_tool_search::{
    ClaudeToolSearchCapability,
    model_supports_native_tool_search as model_supports_native_claude_tool_search,
};
use everruns_core::capabilities::openai_tool_search::{
    OpenAiToolSearchCapability,
    model_supports_native_tool_search as model_supports_native_openai_tool_search,
};
use everruns_core::capabilities::tool_search::ToolSearchCapability;
use everruns_core::capabilities::{Capability, CapabilityLocalization, CapabilityStatus};

pub use everruns_core::capabilities::openai_tool_search::DEFAULT_TOOL_SEARCH_THRESHOLD;

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
    use everruns_core::capabilities::*;
    use everruns_core::capabilities::{
        CLAUDE_TOOL_SEARCH_CAPABILITY_ID, OPENAI_TOOL_SEARCH_CAPABILITY_ID,
        TOOL_SEARCH_CAPABILITY_ID,
    };

    // Metadata constants covered by builtin_capabilities_satisfy_registry_invariants.

    #[test]
    fn test_resolves_to_generic_without_model() {
        // No model known → safe provider-agnostic client-side mechanism.
        let cap = AutoToolSearchCapability::new();
        let resolved = cap.resolve_for_model(None).expect("dispatches");
        assert_eq!(resolved.id(), TOOL_SEARCH_CAPABILITY_ID);
        // The generic mechanism carries the client-side tool + hook.
        assert_eq!(resolved.tools().len(), 1);
        assert_eq!(resolved.tool_definition_hooks().len(), 1);
    }

    #[test]
    fn test_resolves_to_generic_on_non_native_model() {
        // A model with no hosted tool_search support on either provider (here a
        // pre-4 Claude) falls back to the safe client-side mechanism.
        let cap = AutoToolSearchCapability::new();
        let resolved = cap
            .resolve_for_model(Some("claude-3-5-haiku"))
            .expect("dispatches");
        assert_eq!(resolved.id(), TOOL_SEARCH_CAPABILITY_ID);
    }

    #[test]
    fn test_resolves_to_hosted_on_native_openai_model() {
        let cap = AutoToolSearchCapability::new();
        let resolved = cap.resolve_for_model(Some("gpt-5.4")).expect("dispatches");
        assert_eq!(resolved.id(), OPENAI_TOOL_SEARCH_CAPABILITY_ID);
        // The hosted mechanism contributes no client-side tool or hook.
        assert!(resolved.tools().is_empty());
        assert!(resolved.tool_definition_hooks().is_empty());
    }

    #[test]
    fn test_resolves_to_hosted_on_native_claude_model() {
        let cap = AutoToolSearchCapability::new();
        let resolved = cap
            .resolve_for_model(Some("claude-opus-4-8"))
            .expect("dispatches");
        assert_eq!(resolved.id(), CLAUDE_TOOL_SEARCH_CAPABILITY_ID);
        // Hosted: no client-side tool or hook contributed.
        assert!(resolved.tools().is_empty());
        assert!(resolved.tool_definition_hooks().is_empty());
    }
}
