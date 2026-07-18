use async_trait::async_trait;
use everruns_provider::driver_helpers::shared_request_http_client;
use everruns_provider::llm_retry::{LlmRetryConfig, is_transient_error};
use everruns_provider::{EmbedRequest, EmbedResponse, EmbeddingsDriver, EmbeddingsDriverError};
use serde::{Deserialize, Serialize};

/// Embeddings driver for OpenAI's `/v1/embeddings` endpoint.
pub struct OpenAIEmbeddingsDriver {
    api_key: String,
    base_url: String,
    client: reqwest::Client,
}

impl OpenAIEmbeddingsDriver {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: "https://api.openai.com/v1".to_string(),
            client: shared_request_http_client(),
        }
    }

    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
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
}

#[async_trait]
impl EmbeddingsDriver for OpenAIEmbeddingsDriver {
    async fn embed(&self, request: EmbedRequest) -> Result<EmbedResponse, EmbeddingsDriverError> {
        let url = format!("{}/embeddings", self.base_url);
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
            let send_result = self
                .client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
                .await;

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
