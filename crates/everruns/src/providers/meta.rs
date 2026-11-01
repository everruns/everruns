//! Meta provider configuration (`meta` feature).
use crate::Provider;
use everruns_contracts::credential_provider::EnvCredentialError;

/// Typed Meta connection configuration. Credentials are redacted from Debug.
#[derive(Clone)]
pub struct Meta {
    api_key: String,
    base_url: Option<String>,
}
impl Meta {
    /// Configure a connection with an explicit API key; reads no environment.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
        }
    }
    /// Resolve this driver's declared credential and endpoint variables.
    pub fn from_env() -> Result<Self, EnvCredentialError> {
        let credentials = super::resolve_env(&everruns_drivers::meta::descriptor())?;
        Ok(Self {
            api_key: credentials.api_key().unwrap_or_default().into(),
            base_url: credentials.base_url().map(str::to_owned),
        })
    }
    /// Override the API endpoint.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
}
impl std::fmt::Debug for Meta {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meta")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .finish()
    }
}
impl From<Meta> for Provider {
    fn from(config: Meta) -> Self {
        let provider = everruns_drivers::meta::provider("meta", config.api_key)
            .with_driver_id(crate::DriverId::Meta);
        match config.base_url {
            Some(url) => provider.base_url(url),
            None => provider,
        }
    }
}
