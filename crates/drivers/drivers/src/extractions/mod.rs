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
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .ok()
}

/// Bound the response and suppress provider error bodies, which may echo input
/// documents or account credentials. Authentication is resolved for each call.
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
