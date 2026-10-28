//! Application-owned model transport: a fixed provider origin, verified TLS,
//! deployment HTTP(S) proxies and no redirects. Arbitrary provider URLs are not
//! accepted here; the shared platform HTTP clients retain their SSRF guards.
use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use everruns_contracts::http_transport::{HttpError, HttpRequest, HttpResponse, HttpTransport};

struct OpenAiTransport(reqwest::Client);

fn allowed(uri: &str) -> bool {
    let Ok(url) = reqwest::Url::parse(uri) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("api.openai.com")
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && url.path().starts_with("/v1/")
}

#[async_trait::async_trait]
impl HttpTransport for OpenAiTransport {
    async fn execute(&self, request: HttpRequest) -> Result<HttpResponse, HttpError> {
        if !allowed(&request.uri().to_string()) {
            return Err(HttpError::new(
                "Data agent model transport refuses this origin",
                false,
            ));
        }
        // The host owns this fixed-origin client. Default proxy configuration
        // and system CA trust are preserved; no request headers are logged.
        HttpTransport::execute(&self.0, request).await
    }
}

pub fn provider_from_env() -> Result<everruns::Provider> {
    let key = std::env::var("OPENAI_API_KEY").context("Set OPENAI_API_KEY")?;
    anyhow::ensure!(!key.trim().is_empty(), "Set OPENAI_API_KEY");
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(60))
        .build()?;
    let driver = everruns::providers::openai::OpenAIChatDriver::new()
        .with_transport(Arc::new(OpenAiTransport(client)));
    Ok(everruns::Provider::new("openai", driver)
        .with_driver_id(everruns::DriverId::OpenAI)
        .base_url("https://api.openai.com/v1")
        .auth(everruns::BearerAuth::new(key)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_transport_refuses_credentials_downgrades_and_other_origins() {
        assert!(allowed("https://api.openai.com/v1/responses"));
        for url in [
            "http://api.openai.com/v1/responses",
            "https://api.openai.com:444/v1/responses",
            "https://api.openai.com.attacker.test/v1/responses",
            "https://attacker.test/v1/responses",
            "https://api.openai.com@attacker.test/v1/responses",
            "https://user:secret@api.openai.com/v1/responses",
            "https://127.0.0.1/v1/responses",
            "https://169.254.169.254/latest/meta-data",
            "https://api.openai.com/other",
        ] {
            assert!(!allowed(url), "accepted {url}");
        }
    }
}
