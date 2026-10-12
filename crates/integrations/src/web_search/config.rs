//! Per-agent web search settings (the capability config).

use everruns_contracts::runtime::web_search::{
    SearchFreshness, WEB_SEARCH_MAX_RESULTS, normalize_domains,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// What happens when the chosen provider cannot answer.
pub use everruns_contracts::runtime::web_search::SearchFallbackMode as FallbackMode;
/// Which provider an agent searches with (`org_default` follows the
/// organization's `web_search_defaults`).
pub use everruns_contracts::runtime::web_search::SearchProviderSetting as ProviderSetting;

/// The `web_search` capability config.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WebSearchConfig {
    #[serde(default)]
    pub provider: ProviderSetting,
    /// The organization search account when `provider` is `account`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<Uuid>,
    /// Unset follows the organization's fallback (fall back by default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fallback: Option<FallbackMode>,
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
        match (config.provider, config.account_id) {
            (ProviderSetting::Account, None) => {
                return Err("web_search account_id is required when provider is account".into());
            }
            (ProviderSetting::Account, Some(_)) | (_, None) => {}
            (_, Some(_)) => {
                return Err(
                    "web_search account_id is only allowed when provider is account".into(),
                );
            }
        }
        normalize_domains(&config.allowed_domains)
            .and(normalize_domains(&config.blocked_domains))
            .map_err(|error| format!("web_search {error}"))?;
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
                "enum": ["org_default", "auto", "native", "brave", "account", "free"],
                "enumNames": [
                    "Organization default",
                    "Automatic",
                    "Model provider search",
                    "Brave Search",
                    "A search account",
                    "Free search"
                ],
                "default": "org_default",
                "description": "Automatic uses the model's own search when it has one (billed by \
                                the model provider, about $10 per 1,000 searches), then a \
                                connected search account, then free search."
            },
            "account_id": {
                "type": "string",
                "format": "uuid",
                "title": "Search account",
                "description": "The organization search account, when the provider is a search account."
            },
            "fallback": {
                "type": "string",
                "title": "When the provider cannot answer",
                "description": "Unset follows the organization default.",
                "enum": ["next", "none"],
                "enumNames": ["Fall back to the next provider, then free search", "Stop"]
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
            "provider", "account_id", "fallback", "limit", "freshness",
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
        assert_eq!(config.fallback, None);
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
    fn account_id_goes_with_the_account_provider() {
        let id = "00000000-0000-0000-0000-000000000007";
        assert!(
            WebSearchConfig::from_value(
                &serde_json::json!({"provider": "account", "account_id": id})
            )
            .is_ok()
        );
        assert!(WebSearchConfig::from_value(&serde_json::json!({"provider": "account"})).is_err());
        assert!(
            WebSearchConfig::from_value(&serde_json::json!({"provider": "free", "account_id": id}))
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
