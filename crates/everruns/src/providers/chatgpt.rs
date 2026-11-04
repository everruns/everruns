//! Host-authenticated ChatGpt connection configuration (`chatgpt` feature).
use crate::Provider;

/// A ChatGpt connection using host-owned authentication and token refresh.
#[derive(Clone)]
pub struct ChatGpt {
    provider: Provider,
}
impl ChatGpt {
    /// Attach authentication supplied by the host. No environment or browser is read.
    pub fn new(auth: impl everruns_contracts::ProviderAuth + 'static) -> Self {
        Self {
            provider: everruns_drivers::chatgpt::provider("chatgpt", auth)
                .with_driver_id(crate::DriverId::external("chatgpt")),
        }
    }
    /// Override the endpoint used by this connection.
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.provider = self.provider.base_url(url);
        self
    }
}
impl std::fmt::Debug for ChatGpt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatGpt")
            .field("auth", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}
impl From<ChatGpt> for Provider {
    fn from(config: ChatGpt) -> Self {
        config.provider
    }
}
