//! Fireworks chat and native dataset-backed batch configuration (`fireworks` feature).
use crate::Provider;
use everruns_contracts::credential_provider::{EnvCredentialError, EnvCredentialProvider};
#[cfg(not(target_arch = "wasm32"))]
pub use everruns_drivers::fireworks::FireworksBatchConfig;
pub use everruns_drivers::fireworks::{FireworksChatDriver, register_driver};

/// Explicit Fireworks provider. Batch support requires an account and trusted
/// artifact origins via [`Self::batch`]. Credentials are redacted from Debug.
#[derive(Clone)]
pub struct Fireworks {
    api_key: String,
    base_url: Option<String>,
    #[cfg(not(target_arch = "wasm32"))]
    batch: Option<FireworksBatchConfig>,
}
impl Fireworks {
    /// Configure the provider with an explicit key; reads no environment.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: None,
            #[cfg(not(target_arch = "wasm32"))]
            batch: None,
        }
    }
    /// Resolve FIREWORKS_API_KEY and optional FIREWORKS_BASE_URL through the
    /// driver's credential schema. Account and artifact policy stay explicit.
    pub fn from_env() -> Result<Self, EnvCredentialError> {
        let descriptor = everruns_drivers::fireworks::descriptor();
        let credentials =
            EnvCredentialProvider::resolve_with(&descriptor, |name| std::env::var(name).ok())
                .filter(|c| c.api_key().is_some())
                .ok_or_else(|| EnvCredentialError::Missing {
                    driver: descriptor.id.to_string(),
                    variables: descriptor.declared_env_vars(),
                })?;
        let mut provider = Self::new(credentials.api_key().expect("filtered above"));
        provider.base_url = credentials.base_url().map(str::to_owned);
        Ok(provider)
    }
    /// Override the inference origin; it must also serve the management API.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
    /// Enable native batches with a trusted account and artifact policy.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn batch(mut self, config: FireworksBatchConfig) -> Self {
        self.batch = Some(config);
        self
    }
}
impl std::fmt::Debug for Fireworks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Fireworks")
            .field("api_key", &"[REDACTED]")
            .field("base_url", &self.base_url.as_ref().map(|_| "<configured>"))
            .finish()
    }
}
impl From<Fireworks> for Provider {
    fn from(config: Fireworks) -> Self {
        let driver = FireworksChatDriver::new();
        #[cfg(not(target_arch = "wasm32"))]
        let driver = match config.batch {
            Some(batch) => driver.with_batch(batch),
            None => driver,
        };
        Provider::new("fireworks", driver)
            .with_driver_id(crate::DriverId::Fireworks)
            .base_url(
                config
                    .base_url
                    .unwrap_or_else(|| "https://api.fireworks.ai/inference/v1".into()),
            )
            .auth(crate::BearerAuth::new(config.api_key))
    }
}
