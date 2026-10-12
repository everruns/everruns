// OpenRouter generation lookup client.
//
// Fetches authoritative usage and cost data for a completed OpenRouter
// generation using the GET /api/v1/generation endpoint. The generation ID
// ("gen-...") is the value the OpenRouter API returns as `response.id`.
//
// Reference: https://openrouter.ai/docs/api-reference/get-a-generation

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

const OPENROUTER_GENERATION_URL: &str = "https://openrouter.ai/api/v1/generation";

/// Authoritative usage and cost data returned by OpenRouter for a completed
/// generation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenRouterGenerationData {
    pub id: String,
    /// Model as reported by OpenRouter (may differ from the requested model).
    pub model: Option<String>,
    /// Actual upstream provider name (e.g. "OpenAI", "Anthropic").
    pub provider_name: Option<String>,
    /// Authoritative prompt token count.
    pub tokens_prompt: Option<i64>,
    /// Authoritative completion token count.
    pub tokens_completion: Option<i64>,
    /// Total cost in USD charged by OpenRouter.
    pub total_cost: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct GenerationResponse {
    data: GenerationResponseData,
}

#[derive(Debug, Deserialize)]
struct GenerationResponseData {
    id: String,
    model: Option<String>,
    provider_name: Option<String>,
    tokens_prompt: Option<i64>,
    tokens_completion: Option<i64>,
    total_cost: Option<f64>,
}

/// Fetch authoritative generation data from OpenRouter.
///
/// A non-2xx HTTP response or parse error returns `Err`. Callers must treat
/// errors as transient (log and retry later) — they must never block the user
/// turn.
pub async fn fetch_openrouter_generation(
    client: &reqwest::Client,
    api_key: &str,
    generation_id: &str,
) -> Result<OpenRouterGenerationData> {
    fetch_openrouter_generation_from(client, api_key, generation_id, OPENROUTER_GENERATION_URL)
        .await
}

async fn fetch_openrouter_generation_from(
    client: &reqwest::Client,
    api_key: &str,
    generation_id: &str,
    base_url: &str,
) -> Result<OpenRouterGenerationData> {
    if generation_id.is_empty() {
        bail!("generation_id must not be empty");
    }

    let url = format!("{base_url}?id={}", urlencoding::encode(generation_id));
    let resp = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json")
        .send()
        .await
        .context("fetch_openrouter_generation HTTP request failed")?;

    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        bail!("OpenRouter generation API returned {status}: {body}");
    }

    let envelope: GenerationResponse = resp
        .json()
        .await
        .context("failed to deserialise OpenRouter generation response")?;

    let d = envelope.data;
    Ok(OpenRouterGenerationData {
        id: d.id,
        model: d.model,
        provider_name: d.provider_name,
        tokens_prompt: d.tokens_prompt,
        tokens_completion: d.tokens_completion,
        total_cost: d.total_cost,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{header_exists, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn rejects_empty_generation_id() {
        let client = reqwest::Client::new();
        let err = fetch_openrouter_generation(&client, "key", "")
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("must not be empty"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn returns_ok_with_mocked_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .and(query_param("id", "gen-abc123"))
            .and(header_exists("Authorization"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "data": {
                    "id": "gen-abc123",
                    "model": "openai/gpt-4o",
                    "provider_name": "OpenAI",
                    "tokens_prompt": 100,
                    "tokens_completion": 50,
                    "total_cost": 0.0015
                }
            })))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let base_url = mock_server.uri();
        let result = fetch_openrouter_generation_from(&client, "test-key", "gen-abc123", &base_url)
            .await
            .expect("should succeed");

        assert_eq!(result.id, "gen-abc123");
        assert_eq!(result.model.as_deref(), Some("openai/gpt-4o"));
        assert_eq!(result.provider_name.as_deref(), Some("OpenAI"));
        assert_eq!(result.tokens_prompt, Some(100));
        assert_eq!(result.tokens_completion, Some(50));
        assert_eq!(result.total_cost, Some(0.0015));
    }

    #[tokio::test]
    async fn returns_err_on_non_2xx() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(429).set_body_string("rate limited"))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let base_url = mock_server.uri();
        let err = fetch_openrouter_generation_from(&client, "key", "gen-err", &base_url)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("429"),
            "expected 429 in error: {err}"
        );
    }

    #[tokio::test]
    async fn returns_err_on_malformed_response() {
        let mock_server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string("not json"))
            .mount(&mock_server)
            .await;

        let client = reqwest::Client::new();
        let base_url = mock_server.uri();
        let err = fetch_openrouter_generation_from(&client, "key", "gen-bad", &base_url)
            .await
            .unwrap_err();
        assert!(
            err.to_string().contains("deserialise"),
            "expected deserialization error: {err}"
        );
    }
}
