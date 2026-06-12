// Auto Tool Search Capability
//
// A model-adaptive dispatcher over the two real tool-search mechanisms:
//
//   - `openai_tool_search` (hosted): on models with native tool_search support
//     (OpenAI GPT-5.4+), the LLM driver hides parameter schemas server-side via
//     namespaces + defer_loading. No client-side tool is added.
//   - `tool_search` (generic, client-side): on every other model (Anthropic,
//     Gemini, OpenAI Completions, ...), a `DeferSchemaHook` strips schemas and a
//     `tool_search` tool loads them back on demand.
//
// Unlike picking one of those capabilities by hand, this one chooses at runtime.
// It OWNS an `OpenAiToolSearchCapability` and a `ToolSearchCapability` and
// implements `Capability::resolve_for_model`: capability collection knows the
// agent's model (via `SystemPromptContext::model`) and delegates to whichever
// inner capability fits. Only that one capability's contributions are collected —
// the hosted config for the OpenAI one, or the hook + tool + system prompt for
// the generic one. No "contribute both, prune later" step is needed.
//
// Use this instead of `openai_tool_search` or `tool_search` when a harness must
// work well across providers.

use super::openai_tool_search::{OpenAiToolSearchCapability, model_supports_native_tool_search};
use super::tool_search::ToolSearchCapability;
use super::{Capability, CapabilityLocalization, CapabilityStatus};

pub use super::openai_tool_search::DEFAULT_TOOL_SEARCH_THRESHOLD;

/// Capability ID for the model-adaptive tool search.
pub const AUTO_TOOL_SEARCH_CAPABILITY_ID: &str = "auto_tool_search";

/// Auto Tool Search capability.
///
/// Holds the two real tool-search capabilities and dispatches to one of them
/// based on the agent's model. `threshold` (minimum number of tools before
/// deferral activates) is shared by both and forwarded at construction.
pub struct AutoToolSearchCapability {
    openai: OpenAiToolSearchCapability,
    generic: ToolSearchCapability,
}

impl AutoToolSearchCapability {
    pub fn new() -> Self {
        Self::with_threshold(DEFAULT_TOOL_SEARCH_THRESHOLD)
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self {
            openai: OpenAiToolSearchCapability::with_threshold(threshold),
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
        "Model-adaptive deferred tool loading. Uses OpenAI's hosted tool_search \
         on models that support it (GPT-5.4 and newer) and a provider-agnostic \
         client-side fallback on every other model. Reduces token usage for \
         agents with many tools, regardless of provider."
    }

    fn localizations(&self) -> Vec<CapabilityLocalization> {
        vec![CapabilityLocalization::text(
            "uk",
            "Автоматичний пошук інструментів",
            "Відкладене завантаження інструментів, що адаптується до моделі. Використовує хостований tool_search від OpenAI на моделях, які його підтримують (GPT-5.4 і новіші), та незалежний від провайдера клієнтський резервний механізм на всіх інших моделях. Зменшує використання токенів для агентів із багатьма інструментами незалежно від провайдера.",
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
    // one's. Models with native support get the hosted OpenAI mechanism (no
    // client-side tool or hook); everything else — including an unknown model —
    // gets the provider-agnostic client-side mechanism, which is safe everywhere.
    fn resolve_for_model(&self, model: Option<&str>) -> Option<&dyn Capability> {
        if model.is_some_and(model_supports_native_tool_search) {
            Some(&self.openai)
        } else {
            Some(&self.generic)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{
        CapabilityRegistry, OPENAI_TOOL_SEARCH_CAPABILITY_ID, TOOL_SEARCH_CAPABILITY_ID,
    };

    #[test]
    fn test_capability_metadata() {
        let cap = AutoToolSearchCapability::new();
        assert_eq!(cap.id(), AUTO_TOOL_SEARCH_CAPABILITY_ID);
        assert_eq!(cap.name(), "Auto Tool Search");
        assert_eq!(cap.category(), Some("Optimization"));
    }

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
        let cap = AutoToolSearchCapability::new();
        let resolved = cap
            .resolve_for_model(Some("claude-sonnet-4-5-20250514"))
            .expect("dispatches");
        assert_eq!(resolved.id(), TOOL_SEARCH_CAPABILITY_ID);
    }

    #[test]
    fn test_resolves_to_hosted_on_native_model() {
        let cap = AutoToolSearchCapability::new();
        let resolved = cap.resolve_for_model(Some("gpt-5.4")).expect("dispatches");
        assert_eq!(resolved.id(), OPENAI_TOOL_SEARCH_CAPABILITY_ID);
        // The hosted mechanism contributes no client-side tool or hook.
        assert!(resolved.tools().is_empty());
        assert!(resolved.tool_definition_hooks().is_empty());
    }

    #[test]
    fn test_capability_registered_in_builtins() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get(AUTO_TOOL_SEARCH_CAPABILITY_ID).unwrap();
        assert_eq!(cap.id(), AUTO_TOOL_SEARCH_CAPABILITY_ID);
    }
}
