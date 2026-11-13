//! Native typed extraction transports. No chat fallback: confidence and evidence
//! are provider observations, not values reconstructed from generated prose.

#[cfg(feature = "cloudflare")]
pub mod cloudflare;
#[cfg(feature = "openrouter")]
pub mod openrouter;

use everruns_contracts::ProviderEndpoint;
use everruns_contracts::error::{AgentLoopError, Result};

pub(crate) fn invalid(message: impl Into<String>) -> AgentLoopError {
    AgentLoopError::llm(message)
}

pub(crate) fn client() -> Option<reqwest::Client> {
    everruns_contracts::install_default_crypto_provider();
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .dns_resolver(std::sync::Arc::new(
            everruns_contracts::driver_helpers::SsrfGuardResolver::system(),
        ))
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .ok()
}

/// Bound the response and suppress provider error bodies, which may echo input
/// documents or account credentials. Authentication is resolved for each call.
// THREAT[TM-API-013]: pin connections to public DNS answers, even after registration.
// THREAT[TM-EXTRACT-002]: bounded transport without redirects or echoed error bodies.
pub(crate) async fn json(
    http: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    method: reqwest::Method,
    url: String,
    body: Option<serde_json::Value>,
) -> Result<serde_json::Value> {
    let bytes = body
        .map(|body| serde_json::to_vec(&body))
        .transpose()
        .map_err(|_| invalid("Invalid extraction request"))?
        .unwrap_or_default();
    let resolved = endpoint.resolve(method.as_str(), url, &bytes).await?;
    let mut request = http
        .request(method, &resolved.url)
        .header("content-type", "application/json");
    for (key, value) in resolved.headers {
        request = request.header(key, value);
    }
    let mut response = request
        .body(bytes)
        .send()
        .await
        .map_err(|_| invalid("Extraction provider transport failed"))?;
    if !response.status().is_success() {
        let status = response.status().as_u16();
        return Err(AgentLoopError::llm_http(
            status,
            "",
            format!("Extraction provider returned HTTP {status}"),
        ));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| invalid("Extraction response transport failed"))?
    {
        if bytes.len().saturating_add(chunk.len()) > 4 * 1024 * 1024 {
            return Err(invalid("Extraction response exceeds size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid extraction response JSON"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn provider_redirect_is_not_followed_with_credentials() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/extract"))
            .respond_with(
                ResponseTemplate::new(302)
                    .insert_header("location", format!("{}/unexpected", server.uri())),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/unexpected"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(0)
            .mount(&server)
            .await;
        let endpoint = ProviderEndpoint::from_parts(
            server.uri(),
            everruns_contracts::BearerAuth::new("synthetic-key"),
        );
        let error = json(
            &client().unwrap(),
            &endpoint,
            reqwest::Method::POST,
            format!("{}/extract", server.uri()),
            Some(serde_json::json!({})),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("302"));
    }

    #[tokio::test]
    async fn oversized_response_is_rejected_before_json_decoding() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string(" ".repeat(4 * 1024 * 1024 + 1)),
            )
            .mount(&server)
            .await;
        let endpoint = ProviderEndpoint::from_parts(
            server.uri(),
            everruns_contracts::BearerAuth::new("synthetic-key"),
        );
        let error = json(
            &client().unwrap(),
            &endpoint,
            reqwest::Method::POST,
            server.uri(),
            Some(serde_json::json!({})),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("exceeds size limit"));
    }
}
