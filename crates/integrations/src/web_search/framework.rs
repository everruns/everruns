//! Framework builder for the `web_search` capability.

use everruns_contracts::capability::{CapabilityRef, CapabilitySpec, IntoCapability};
use everruns_contracts::runtime::web_search::{SearchFreshness, WEB_SEARCH_CAPABILITY_ID};

use super::config::{FallbackMode, ProviderSetting, WebSearchConfig};

/// Web search for `AgentBuilder::capability`.
///
/// The builder only writes capability config; keys never enter it. For
/// Brave Search, set `BRAVE_SEARCH_API_KEY` in the environment (the facade's
/// `web-search` feature registers the capability with
/// [`WebSearchCapability::from_env`](super::WebSearchCapability::from_env)),
/// or store the key as the session secret of the same name.
///
/// ```
/// use everruns_contracts::capability::IntoCapability;
/// use everruns_integrations::web_search::WebSearch;
///
/// // Brave when a key is configured, otherwise free search.
/// let automatic = WebSearch::auto().into_capability();
/// // Brave only: fail rather than fall back to free search.
/// let brave = WebSearch::brave().no_fallback().limit(5).into_capability();
/// # let _ = (automatic, brave);
/// ```
#[derive(Debug, Clone, Default)]
pub struct WebSearch {
    config: WebSearchConfig,
}

impl WebSearch {
    fn with_provider(provider: ProviderSetting) -> Self {
        Self {
            config: WebSearchConfig {
                provider,
                ..WebSearchConfig::default()
            },
        }
    }

    /// The model's own search when it has one, then Brave when a key is
    /// configured, then free search.
    pub fn auto() -> Self {
        Self::with_provider(ProviderSetting::Auto)
    }

    /// The model provider's own search, with free search as the fallback.
    pub fn native() -> Self {
        Self::with_provider(ProviderSetting::Native)
    }

    /// Brave Search, with free search as the fallback.
    pub fn brave() -> Self {
        Self::with_provider(ProviderSetting::Brave)
    }

    /// Keyless best-effort search only.
    pub fn free() -> Self {
        Self::with_provider(ProviderSetting::Free)
    }

    /// Use only the chosen provider; a failure is reported instead of falling
    /// back to free search.
    pub fn no_fallback(mut self) -> Self {
        self.config.fallback = Some(FallbackMode::None);
        self
    }

    /// Results per search when the model does not ask for a number (1 to 20).
    pub fn limit(mut self, limit: u32) -> Self {
        self.config.limit = Some(limit.clamp(1, 20));
        self
    }

    pub fn freshness(mut self, freshness: SearchFreshness) -> Self {
        self.config.freshness = Some(freshness);
        self
    }

    /// Only results from these sites.
    pub fn allowed_domains<I, S>(mut self, domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.allowed_domains = domains.into_iter().map(Into::into).collect();
        self
    }

    /// Never results from these sites.
    pub fn blocked_domains<I, S>(mut self, domains: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.config.blocked_domains = domains.into_iter().map(Into::into).collect();
        self
    }

    /// Tell the model which provider answered each search.
    pub fn show_provider(mut self) -> Self {
        self.config.show_provider = true;
        self
    }
}

impl IntoCapability for WebSearch {
    fn into_capability(self) -> CapabilitySpec {
        let config = serde_json::to_value(&self.config).unwrap_or_default();
        CapabilityRef::with_config(WEB_SEARCH_CAPABILITY_ID, config).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writes_only_config() {
        let spec = WebSearch::brave()
            .no_fallback()
            .blocked_domains(["pinterest.com"])
            .into_capability();
        let value = serde_json::to_value(spec.capability_ref()).unwrap();
        let text = value.to_string();
        assert!(text.contains("\"web_search\""), "{text}");
        assert!(text.contains("\"provider\":\"brave\""), "{text}");
        assert!(text.contains("\"fallback\":\"none\""), "{text}");
    }
}
