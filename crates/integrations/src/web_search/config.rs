//! Per-agent web search settings (the capability config).

use everruns_contracts::runtime::web_search::{SearchFreshness, WEB_SEARCH_MAX_RESULTS};
use serde::{Deserialize, Serialize};

/// Which provider an agent searches with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderSetting {
    /// Follow the organization's default. Until organizations can set one,
    /// this behaves as [`Self::Auto`].
    #[default]
    OrgDefault,
    /// Model provider search when the model has it, then a configured search
    /// account, then free search.
    Auto,
    /// The model provider's own search. When the running model has none, the
    /// tool is executed and the rest of the plan answers.
    Native,
    /// Brave Search with a key from the session, the user, or the application.
    Brave,
    /// Keyless best-effort search.
    Free,
}

impl ProviderSetting {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OrgDefault => "org_default",
            Self::Auto => "auto",
            Self::Native => "native",
            Self::Brave => "brave",
            Self::Free => "free",
        }
    }
}

/// What happens when the chosen provider cannot answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FallbackMode {
    /// Try the next provider in the plan; free search is always last.
    #[default]
    Next,
    /// Use only the chosen provider (for example to keep every search on a
    /// paid index).
    None,
}

/// The `web_search` capability config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub provider: ProviderSetting,
    #[serde(default)]
    pub fallback: FallbackMode,
    /// Results per search when the model does not ask for a number.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freshness: Option<SearchFreshness>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_domains: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_domains: Vec<String>,
    /// Tell the model which provider answered and why (fallback notes).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub show_provider: bool,
}

impl WebSearchConfig {
    /// Parse and validate. `null` and `{}` are the defaults.
    pub fn from_value(value: &serde_json::Value) -> Result<Self, String> {
        if value.is_null() {
            return Ok(Self::default());
        }
        let config: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid web_search config: {error}"))?;
        if let Some(limit) = config.limit
            && !(1..=WEB_SEARCH_MAX_RESULTS).contains(&limit)
        {
            return Err(format!(
                "web_search limit must be between 1 and {WEB_SEARCH_MAX_RESULTS}, got {limit}"
            ));
        }
        for domain in config.allowed_domains.iter().chain(&config.blocked_domains) {
            let domain = domain.trim();
            if domain.is_empty() || domain.contains(['/', ' ', ':']) {
                return Err(format!(
                    "web_search domains are bare host names like example.com, got {domain:?}"
                ));
            }
        }
        Ok(config)
    }
}

pub(super) fn schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "additionalProperties": false,
        "properties": {
            "provider": {
                "type": "string",
                "title": "Search provider",
                "enum": ["org_default", "auto", "native", "brave", "free"],
                "enumNames": [
                    "Organization default",
                    "Automatic",
                    "Model provider search",
                    "Brave Search",
                    "Free search"
                ],
                "default": "org_default",
                "description": "Automatic uses the model's own search when it has one (billed by \
                                the model provider, about $10 per 1,000 searches), then a \
                                connected search account, then free search."
            },
            "fallback": {
                "type": "string",
                "title": "When the provider cannot answer",
                "enum": ["next", "none"],
                "enumNames": ["Fall back to the next provider, then free search", "Stop"],
                "default": "next"
            },
            "limit": {
                "type": "integer",
                "title": "Results per search",
                "minimum": 1,
                "maximum": WEB_SEARCH_MAX_RESULTS,
                "default": 8
            },
            "freshness": {
                "type": "string",
                "title": "Only results from the last",
                "enum": ["day", "week", "month", "year"]
            },
            "allowed_domains": {
                "type": "array",
                "title": "Only these sites",
                "items": {"type": "string"}
            },
            "blocked_domains": {
                "type": "array",
                "title": "Never these sites",
                "items": {"type": "string"}
            },
            "show_provider": {
                "type": "boolean",
                "title": "Tell the agent which provider answered",
                "default": false
            }
        }
    })
}

pub(super) fn ui_schema() -> serde_json::Value {
    serde_json::json!({
        "ui:order": [
            "provider", "fallback", "limit", "freshness",
            "allowed_domains", "blocked_domains", "show_provider"
        ],
        "allowed_domains": {"ui:options": {"orderable": false}},
        "blocked_domains": {"ui:options": {"orderable": false}}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_follow_the_organization_and_fall_back() {
        let config = WebSearchConfig::from_value(&serde_json::json!({})).unwrap();
        assert_eq!(config.provider, ProviderSetting::OrgDefault);
        assert_eq!(config.fallback, FallbackMode::Next);
        assert!(!config.show_provider);
    }

    #[test]
    fn rejects_urls_as_domains() {
        assert!(
            WebSearchConfig::from_value(&serde_json::json!({
                "blocked_domains": ["https://pinterest.com"]
            }))
            .is_err()
        );
    }

    #[test]
    fn round_trips() {
        let value = serde_json::json!({
            "provider": "brave", "fallback": "none", "limit": 5,
            "freshness": "week", "blocked_domains": ["pinterest.com"], "show_provider": true
        });
        let config = WebSearchConfig::from_value(&value).unwrap();
        assert_eq!(serde_json::to_value(&config).unwrap(), value);
    }
}
