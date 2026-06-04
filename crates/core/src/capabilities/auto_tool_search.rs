// Auto Tool Search Capability
//
// A single capability that adapts deferred tool loading to the agent's model:
//
//   - On models with native tool_search support (OpenAI GPT-5.4+), it behaves
//     like `openai_tool_search`: the LLM driver hides parameter schemas
//     server-side via namespaces + defer_loading. No client-side tool is added.
//   - On every other model (Anthropic, Gemini, OpenAI Completions, ...), it
//     behaves like the generic `tool_search`: a client-side `DeferSchemaHook`
//     strips schemas and a `tool_search` tool loads them back on demand.
//
// The model is not known when capabilities are collected, so this capability
// contributes BOTH mechanisms up front (the hosted config plus the client-side
// hook + tool). The choice between them is made in
// `RuntimeAgentBuilder::build`, which knows the model: when hosted deferral
// wins it skips the client-side hook and drops the now-redundant `tool_search`
// tool; when the model lacks native support it drops the hosted config so the
// client-side path takes over.
//
// Use this instead of picking `openai_tool_search` or `tool_search` by hand
// when a harness must work well across providers.

use super::tool_search::{DeferSchemaHook, ToolSearchTool};
use super::{Capability, CapabilityStatus, ToolDefinitionHook};
use crate::tools::Tool;
use std::sync::Arc;

pub use super::openai_tool_search::DEFAULT_TOOL_SEARCH_THRESHOLD;

/// Capability ID for the model-adaptive tool search.
pub const AUTO_TOOL_SEARCH_CAPABILITY_ID: &str = "auto_tool_search";

/// Auto Tool Search capability.
///
/// `threshold` controls the minimum number of tools before deferral activates
/// (default: [`DEFAULT_TOOL_SEARCH_THRESHOLD`]). It is honored by both the
/// hosted and client-side paths.
pub struct AutoToolSearchCapability {
    threshold: usize,
}

impl AutoToolSearchCapability {
    pub fn new() -> Self {
        Self {
            threshold: DEFAULT_TOOL_SEARCH_THRESHOLD,
        }
    }

    pub fn with_threshold(threshold: usize) -> Self {
        Self { threshold }
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

    fn status(&self) -> CapabilityStatus {
        CapabilityStatus::Available
    }

    fn category(&self) -> Option<&str> {
        Some("Optimization")
    }

    // The client-side fallback path: the same tool and hook the generic
    // `tool_search` capability uses. On models with native support, build()
    // skips the hook and removes this tool. No system-prompt note is added
    // because it would be inaccurate on native models, and the tool's own
    // description already guides the model on the fallback path.
    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(ToolSearchTool)]
    }

    fn tool_definition_hooks(&self) -> Vec<Arc<dyn ToolDefinitionHook>> {
        vec![Arc::new(DeferSchemaHook::new(self.threshold))]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::{CapabilityRegistry, TOOL_SEARCH_TOOL_NAME};

    #[test]
    fn test_capability_metadata() {
        let cap = AutoToolSearchCapability::new();
        assert_eq!(cap.id(), AUTO_TOOL_SEARCH_CAPABILITY_ID);
        assert_eq!(cap.name(), "Auto Tool Search");
        assert_eq!(cap.category(), Some("Optimization"));
        // Contributes the client-side fallback tool + hook.
        assert_eq!(cap.tools().len(), 1);
        assert_eq!(cap.tools()[0].name(), TOOL_SEARCH_TOOL_NAME);
        assert_eq!(cap.tool_definition_hooks().len(), 1);
    }

    #[test]
    fn test_capability_registered_in_builtins() {
        let registry = CapabilityRegistry::with_builtins();
        let cap = registry.get(AUTO_TOOL_SEARCH_CAPABILITY_ID).unwrap();
        assert_eq!(cap.id(), AUTO_TOOL_SEARCH_CAPABILITY_ID);
    }
}
