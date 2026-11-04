use crate::services::provider_resolver::ResolvedRuntimeProviderConfig;
use everruns_contracts::{ProviderConfig, ProviderKey};

/// Keep session-scoped and org-scoped execution on the same connection settings.
pub(super) fn into_config(
    provider: ProviderKey,
    value: ResolvedRuntimeProviderConfig,
) -> ProviderConfig {
    let mut config = ProviderConfig::for_provider(
        provider,
        super::string_to_provider_type(&value.provider_type),
    );
    config.api_key = value.api_key;
    config.base_url = value.base_url;
    config.request_options = value.request_options;
    config.api = value.api;
    config
}
