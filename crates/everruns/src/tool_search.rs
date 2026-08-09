//! Typed configuration for model-adaptive deferred tool loading.

/// Model-adaptive tool-search capability configuration.
///
/// [`ToolSearch::automatic`] activates the existing `auto_tool_search`
/// implementation: models with native hosted search use it, and every other
/// model uses the provider-agnostic client-side search capability. The default
/// activation threshold is owned by that implementation.
///
/// The optional `threshold` and `never_defer` values map directly to the
/// existing built-in configuration. `never_defer` affects the generic fallback;
/// hosted providers continue to honor the deferrability policy on each tool.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToolSearch {
    threshold: Option<usize>,
    never_defer: Vec<String>,
}

impl ToolSearch {
    /// Select hosted tool search when the model supports it and the generic
    /// client-side implementation otherwise.
    pub fn automatic() -> Self {
        Self::default()
    }

    /// Override the minimum total tool count at which schemas are deferred.
    pub fn threshold(mut self, threshold: usize) -> Self {
        self.threshold = Some(threshold);
        self
    }

    /// Keep these tools' full schemas visible under the generic fallback.
    ///
    /// Names are validated with the same model-facing rules as ordinary
    /// function tools when [`AgentBuilder::build`](crate::AgentBuilder::build)
    /// runs.
    pub fn never_defer<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.never_defer.extend(names.into_iter().map(Into::into));
        self
    }
}

impl crate::IntoCapability for ToolSearch {
    fn into_capability(self) -> crate::CapabilitySpec {
        let mut config = serde_json::Map::new();
        if let Some(threshold) = self.threshold {
            config.insert("threshold".to_string(), threshold.into());
        }
        if !self.never_defer.is_empty() {
            config.insert("never_defer".to_string(), self.never_defer.into());
        }
        crate::CapabilityRef::new("auto_tool_search")
            .config(serde_json::Value::Object(config))
            .into()
    }
}
