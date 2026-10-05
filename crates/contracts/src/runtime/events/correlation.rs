//! Provider correlation carried in an event's `metadata`.
//!
//! A runtime backend that hands the loop to a provider (the OpenAI Agents
//! API, EVE-1125) keeps Everruns ids as the event's identity and records the
//! provider's ids beside them, so exporters can link a span to the provider's
//! own records without either id replacing the other.

use serde_json::Value;

/// Runtime backend that produced the event (`openai_agents_api`).
pub const RUNTIME_BACKEND: &str = "runtime_backend";
/// Provider session the event belongs to.
pub const PROVIDER_SESSION_ID: &str = "provider_session_id";
/// Provider turn the event belongs to.
pub const PROVIDER_TURN_ID: &str = "provider_turn_id";
/// Provider item (message, call) the event projects.
pub const PROVIDER_ITEM_ID: &str = "provider_item_id";
/// Provider subagent that did the work, when not the root agent.
pub const PROVIDER_SUBAGENT_ID: &str = "provider_subagent_id";
/// Where the provider's own trace of the session can be exported.
pub const PROVIDER_TRACE_URL: &str = "provider_trace_url";

/// Every correlation key, in a stable order.
pub const PROVIDER_CORRELATION_KEYS: [&str; 6] = [
    RUNTIME_BACKEND,
    PROVIDER_SESSION_ID,
    PROVIDER_TURN_ID,
    PROVIDER_ITEM_ID,
    PROVIDER_SUBAGENT_ID,
    PROVIDER_TRACE_URL,
];

/// The non-empty string correlation values in an event's metadata.
pub fn provider_correlation(metadata: Option<&Value>) -> Vec<(&'static str, String)> {
    let Some(metadata) = metadata else {
        return Vec::new();
    };
    PROVIDER_CORRELATION_KEYS
        .iter()
        .filter_map(|key| {
            let value = metadata.get(*key)?.as_str()?;
            (!value.is_empty()).then(|| (*key, value.to_string()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_present_string_ids_are_correlated() {
        let metadata = json!({
            "runtime_backend": "openai_agents_api",
            "provider_session_id": "sess_1",
            "provider_turn_id": null,
            "provider_item_id": "",
            "unrelated": "x",
        });
        assert_eq!(
            provider_correlation(Some(&metadata)),
            vec![
                (RUNTIME_BACKEND, "openai_agents_api".to_string()),
                (PROVIDER_SESSION_ID, "sess_1".to_string()),
            ]
        );
        assert!(provider_correlation(None).is_empty());
    }
}
