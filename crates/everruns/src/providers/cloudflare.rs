//! Cloudflare AI Gateway connection configuration (`cloudflare` feature).
use crate::Provider;
use everruns_contracts::credential_provider::EnvCredentialError;

/// Cloudflare account, gateway and credential configuration.
#[derive(Clone)]
pub struct Cloudflare {
    account_id: String,
    api_token: String,
    gateway_id: Option<String>,
    base_url: Option<String>,
}
impl Cloudflare {
    /// Configure an account with an explicit API token.
    pub fn new(account_id: impl Into<String>, api_token: impl Into<String>) -> Self {
        Self {
            account_id: account_id.into(),
            api_token: api_token.into(),
            gateway_id: None,
            base_url: None,
        }
    }
    /// Resolve the account, token and optional gateway from driver-declared variables.
    pub fn from_env() -> Result<Self, EnvCredentialError> {
        let credentials = super::resolve_env(&everruns_drivers::cloudflare::descriptor())?;
        Ok(Self {
            account_id: credentials.field("account_id").unwrap_or_default().into(),
            api_token: credentials.api_key().unwrap_or_default().into(),
            gateway_id: credentials.field("gateway_id").map(str::to_owned),
            base_url: credentials.base_url().map(str::to_owned),
        })
    }
    /// Select a gateway (required for Workers AI models).
    pub fn gateway(mut self, gateway_id: impl Into<String>) -> Self {
        self.gateway_id = Some(gateway_id.into());
        self
    }
    /// Override the account API endpoint.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = Some(url.into());
        self
    }
}
impl std::fmt::Debug for Cloudflare {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Cloudflare")
            .field("api_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
impl From<Cloudflare> for Provider {
    fn from(config: Cloudflare) -> Self {
        let provider = everruns_drivers::cloudflare::provider(
            "cloudflare",
            &config.account_id,
            config.api_token,
            config.gateway_id.as_deref(),
        )
        .with_driver_id(crate::DriverId::Cloudflare);
        match config.base_url {
            Some(url) => provider.base_url(url),
            None => provider,
        }
    }
}
