use everruns_contracts::{ProviderConfig, ProviderKey};
use everruns_internal_protocol::proto::GetDefaultProviderCredentialsResponse;

pub(super) fn from_response(
    provider_id: &str,
    response: GetDefaultProviderCredentialsResponse,
) -> Option<ProviderConfig> {
    if !response.found {
        return None;
    }
    let provider_type = response
        .provider_type
        .parse()
        .unwrap_or_else(|_| unreachable!());
    // Older servers send neither options nor API selection; retain driver defaults.
    let request_options = super::non_empty_string(response.request_options_json)
        .and_then(|json| serde_json::from_str(&json).ok())
        .unwrap_or_default();
    let mut config = ProviderConfig::for_provider(ProviderKey::new(provider_id), provider_type);
    config.api_key = super::non_empty_string(response.api_key);
    config.base_url = super::non_empty_string(response.base_url);
    config.request_options = request_options;
    config.api = super::non_empty_string(response.api);
    Some(config)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selected_api_and_connection_options_survive_worker_transport() {
        let response = GetDefaultProviderCredentialsResponse {
            found: true,
            provider_type: "mistral".into(),
            api: "responses".into(),
            request_options_json: r#"{"cache_diagnostics":true}"#.into(),
            ..Default::default()
        };
        let config = from_response("connection", response).unwrap();
        assert_eq!(config.api.as_deref(), Some("responses"));
        assert_eq!(config.provider.as_str(), "connection");
        assert!(config.request_options.cache_diagnostics);
    }

    #[test]
    fn older_servers_and_missing_connections_retain_defaults() {
        assert!(from_response("connection", Default::default()).is_none());
        let config = from_response(
            "connection",
            GetDefaultProviderCredentialsResponse {
                found: true,
                provider_type: "mistral".into(),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(config.api.is_none());
        assert!(config.request_options.is_empty());
    }
}
