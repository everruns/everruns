//! Azure OpenAI resource configuration (`openai` feature).
use crate::Provider;
use everruns_contracts::credential_provider::EnvCredentialError;

/// An Azure OpenAI resource using the Responses API.
#[derive(Clone)]
pub struct AzureOpenAI {
    api_key: String,
    base_url: String,
}
impl AzureOpenAI {
    /// Configure the versioned resource URL and API key explicitly.
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into(),
        }
    }
    /// Read AZURE_OPENAI_API_KEY, keeping the versioned resource URL explicit.
    pub fn from_env(base_url: impl Into<String>) -> Result<Self, EnvCredentialError> {
        let credentials = super::resolve_env(&everruns_drivers::openai::azure_descriptor())?;
        Ok(Self::new(
            base_url,
            credentials.api_key().unwrap_or_default(),
        ))
    }
    /// Override the versioned resource URL.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }
}
impl std::fmt::Debug for AzureOpenAI {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AzureOpenAI")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &"<configured>")
            .finish()
    }
}
impl From<AzureOpenAI> for Provider {
    fn from(config: AzureOpenAI) -> Self {
        everruns_drivers::openai::azure_provider("azure_openai", config.base_url, config.api_key)
            .with_driver_id(crate::DriverId::AzureOpenAI)
    }
}
