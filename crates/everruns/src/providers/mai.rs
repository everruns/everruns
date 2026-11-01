//! Microsoft MAI connection configuration (`mai` feature).
use crate::Provider;
pub use everruns_drivers::mai::{EntraOAuthConfig, MaiAuth};

/// Microsoft MAI resource and API-key or Entra authentication.
#[derive(Clone)]
pub struct Mai {
    base_url: String,
    auth: MaiAuth,
}
impl Mai {
    /// Configure a Foundry resource with typed authentication.
    pub fn new(base_url: impl Into<String>, auth: MaiAuth) -> Self {
        Self {
            base_url: base_url.into(),
            auth,
        }
    }
    /// Resolve the resource and credentials using the MAI driver's declarations.
    pub fn from_env() -> everruns_contracts::error::Result<Self> {
        let credentials = super::resolve_env(&everruns_drivers::mai::descriptor())
            .map_err(|error| everruns_contracts::AgentLoopError::config(error.to_string()))?;
        let base_url = credentials
            .base_url()
            .filter(|url| !url.trim().is_empty())
            .ok_or_else(|| {
                everruns_contracts::AgentLoopError::config("AZURE_AI_ENDPOINT is required")
            })?;
        let mut config = everruns_contracts::ProviderConfig::new(crate::DriverId::Mai);
        config.api_key = credentials.document();
        let driver_config =
            everruns_contracts::driver_registry::DriverConfig::from_provider_config(&config);
        Ok(Self::new(
            base_url,
            MaiAuth::from_driver_config(&driver_config)?,
        ))
    }
    /// Override the Foundry resource endpoint.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}
impl std::fmt::Debug for Mai {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Mai")
            .field("auth", &"[REDACTED]")
            .field("base_url", &"<configured>")
            .finish()
    }
}
impl From<Mai> for Provider {
    fn from(config: Mai) -> Self {
        everruns_drivers::mai::provider("mai", config.base_url, config.auth)
            .with_driver_id(crate::DriverId::Mai)
    }
}
