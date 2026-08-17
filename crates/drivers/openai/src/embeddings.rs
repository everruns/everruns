use async_trait::async_trait;
use everruns_provider::driver_helpers::shared_request_http_client;
use everruns_provider::llm_retry::{LlmRetryConfig, is_transient_error};
use everruns_provider::{EmbedRequest, EmbedResponse, EmbeddingsDriver, EmbeddingsDriverError};
use serde::{Deserialize, Serialize};

/// Embeddings driver for OpenAI's `/v1/embeddings` endpoint.
pub struct OpenAIEmbeddingsDriver {
    client: reqwest::Client,
}

impl OpenAIEmbeddingsDriver {
    pub fn new() -> Self {
        Self {
            client: shared_request_http_client(),
        }
    }
}

#[derive(Serialize)]
struct EmbeddingsApiRequest {
    input: Vec<String>,
    model: String,
    encoding_format: &'static str,
}

#[derive(Deserialize)]
struct EmbeddingsApiResponse {
    data: Vec<EmbeddingObject>,
    usage: EmbeddingsUsage,
}

#[derive(Deserialize)]
struct EmbeddingObject {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Deserialize)]
struct EmbeddingsUsage {
    total_tokens: u32,
    /// OpenAI-compatible gateways (OpenRouter and friends) report the real
    /// post-routing cost inline. Direct OpenAI omits it (EVE-894).
    #[serde(default)]
    cost: Option<f64>,
}

#[async_trait]
impl EmbeddingsDriver for OpenAIEmbeddingsDriver {
    async fn embed(
        &self,
        endpoint: &everruns_provider::ProviderEndpoint,
        request: EmbedRequest,
    ) -> Result<EmbedResponse, EmbeddingsDriverError> {
        let url = endpoint
            .url("embeddings")
            .ok_or_else(|| EmbeddingsDriverError::Provider("provider has no base URL".into()))?;
        let body = EmbeddingsApiRequest {
            input: request.texts,
            model: request.model,
            encoding_format: "float",
        };
        // EVE-635: retry transient failures (network errors, 429/5xx) with the
        // shared exponential-backoff policy. Non-transient errors (4xx other
        // than 429, parse failures) fail fast.
        let retry = LlmRetryConfig::default();
        let mut attempt: u32 = 0;
        loop {
            let bytes = serde_json::to_vec(&body)
                .map_err(|error| EmbeddingsDriverError::Transport(error.to_string()))?;
            let resolved = endpoint
                .resolve("POST", &url, &bytes)
                .await
                .map_err(|error| EmbeddingsDriverError::Provider(error.to_string()))?;
            let mut builder = self
                .client
                .post(&resolved.url)
                .header("content-type", "application/json");
            for (name, value) in resolved.headers {
                builder = builder.header(name, value);
            }
            let send_result = builder.body(bytes).send().await;

            let transient_err: EmbeddingsDriverError = match send_result {
                Ok(response) => {
                    let status = response.status();
                    if status.is_success() {
                        let api_resp: EmbeddingsApiResponse = response
                            .json()
                            .await
                            .map_err(|e| EmbeddingsDriverError::Transport(e.to_string()))?;
                        // Sort by index to ensure output order matches input order.
                        let mut data = api_resp.data;
                        data.sort_by_key(|e| e.index);
                        return Ok(EmbedResponse {
                            embeddings: data.into_iter().map(|e| e.embedding).collect(),
                            usage_tokens: Some(api_resp.usage.total_tokens),
                            actual_cost_usd: api_resp.usage.cost,
                        });
                    }
                    let text = response.text().await.unwrap_or_default();
                    let err = EmbeddingsDriverError::Provider(format!("HTTP {status}: {text}"));
                    // Only transient HTTP statuses are worth retrying.
                    if !is_transient_error(status) {
                        return Err(err);
                    }
                    err
                }
                // Network/transport errors are transient.
                Err(e) => EmbeddingsDriverError::Transport(e.to_string()),
            };

            if attempt >= retry.max_retries {
                return Err(transient_err);
            }
            let backoff = retry.calculate_backoff(attempt);
            tracing::warn!(
                attempt,
                backoff_secs = backoff.as_secs_f64(),
                error = %transient_err,
                "embeddings request transient failure; retrying"
            );
            tokio::time::sleep(backoff).await;
            attempt += 1;
        }
    }
}

impl Default for OpenAIEmbeddingsDriver {
    fn default() -> Self {
        Self::new()
    }
}
