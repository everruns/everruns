//! TypeSafe accounts expose typed decisions through the System One protocol.
use crate::systemone::SystemOneDecisionDriver;
pub use crate::systemone::client;
use everruns_contracts::driver_registry::{
    DriverConfig, DriverDescriptor, DriverRegistry, ServiceKind,
};
use everruns_contracts::provider::DriverId;
use everruns_contracts::{BearerAuth, Provider, ProviderKey};

pub fn provider(id: impl Into<ProviderKey>, api_key: impl Into<String>) -> Provider {
    Provider::services(id)
        .with_decisions(SystemOneDecisionDriver::default())
        .base_url("https://api.typesafe.ai/v1")
        .auth(BearerAuth::new(api_key))
        .with_driver_id(DriverId::external("typesafe"))
}

fn configured(config: &DriverConfig) -> Provider {
    let provider = provider(
        config.provider.clone(),
        config.api_key.clone().unwrap_or_default(),
    );
    match &config.base_url {
        Some(url) => provider.base_url(url),
        None => provider,
    }
}

pub fn descriptor() -> DriverDescriptor {
    DriverDescriptor {
        id: DriverId::external("typesafe"),
        display_name: "TypeSafe".into(),
        services: vec![ServiceKind::Decisions],
        credential_schema: everruns_contracts::credential_schema::CredentialFormSchema::api_key(
            "TYPESAFE_API_KEY",
            "Create an API key in your TypeSafe account.",
        ),
        base_url_env: Some("TYPESAFE_BASE_URL".into()),
        oauth: None,
        chat: None,
        embeddings: None,
        provider: Some(std::sync::Arc::new(configured)),
    }
}

pub fn register_driver(registry: &mut DriverRegistry) {
    registry.register_descriptor(descriptor());
}
pub fn from_env(
    id: impl Into<ProviderKey>,
) -> Result<Provider, everruns_contracts::credential_provider::EnvCredentialError> {
    everruns_contracts::credential_provider::provider_from_env(&descriptor(), id)
}
