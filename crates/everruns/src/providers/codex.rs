//! Host-authenticated Codex connection configuration (`codex` feature).
use crate::Provider;

/// A Codex connection using host-owned authentication and token refresh.
#[derive(Clone)]
pub struct Codex {
    provider: Provider,
}
impl Codex {
    /// Attach authentication supplied by the host. No environment or browser is read.
    pub fn new(auth: impl everruns_contracts::ProviderAuth + 'static) -> Self {
        Self {
            provider: everruns_drivers::codex::provider("codex", auth)
                .with_driver_id(crate::DriverId::external("openai-codex")),
        }
    }
    /// Override the endpoint used by this connection.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.provider = self.provider.base_url(url);
        self
    }
}
impl std::fmt::Debug for Codex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Codex")
            .field("auth", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
impl From<Codex> for Provider {
    fn from(config: Codex) -> Self {
        config.provider
    }
}
