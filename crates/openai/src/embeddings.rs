use async_trait::async_trait;
use everruns_core::{EmbedRequest, EmbedResponse, EmbeddingsDriver, EmbeddingsDriverError};
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
            client: reqwest::Client::new(),
        }
    }

    pub fn with_base_url(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
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
        let response = self
            .client
            .post(&url)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|e| EmbeddingsDriverError::Transport(e.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(EmbeddingsDriverError::Provider(format!(
                "HTTP {status}: {text}"
            )));
        }
        let api_resp: EmbeddingsApiResponse = response
            .json()
            .await
            .map_err(|e| EmbeddingsDriverError::Transport(e.to_string()))?;
        // Sort by index to ensure output order matches input order.
        let mut data = api_resp.data;
        data.sort_by_key(|e| e.index);
        Ok(EmbedResponse {
            embeddings: data.into_iter().map(|e| e.embedding).collect(),
            usage_tokens: Some(api_resp.usage.total_tokens),
        })
    }
}
