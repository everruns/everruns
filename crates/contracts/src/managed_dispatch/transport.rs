#[cfg(not(target_arch = "wasm32"))]
use crate::http_transport::HttpError;
use crate::http_transport::{HttpClient, HttpTransport};
use std::sync::Arc;

/// No generic/default client conversion: implicit native protocol retries are
/// unsafe for a financial start even when a driver has no explicit retry loop.
pub struct DispatchTransport(pub(super) HttpClient);

impl DispatchTransport {
    /// Trusted host adapter. One execute must mean one attempted send, with no
    /// automatic redirects/retries. Cancellation must abort send/body reading.
    /// The host must qualify these properties; this constructor is not proof.
    pub fn injected(transport: Arc<dyn HttpTransport>) -> Self {
        Self(HttpClient::new(transport))
    }

    /// Native single-attempt transport. Builder failures refuse configuration;
    /// never fall back to a client that loses retry, redirect or DNS guards.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn native() -> Result<Self, HttpError> {
        crate::install_default_crypto_provider();
        let client = reqwest::Client::builder()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .dns_resolver(Arc::new(crate::driver_helpers::SsrfGuardResolver::system()))
            .connect_timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|_| HttpError::new("single-attempt transport unavailable", false))?;
        Ok(Self(HttpClient::new(Arc::new(client))))
    }
}
