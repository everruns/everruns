//! Decision-model catalogs published by System One hosts.
//!
//! TypeSafe lists its models at `GET {base}/models` as
//! `{"models": [{"name", "description", "release_date"}]}`. OpenRouter lists
//! every vendor's decision models in its regular catalog, filtered with
//! `?output_modalities=decisions`; an entry's `architecture.output_modalities`
//! is `["decisions"]`. Both are answered by the same `{base}/systemone` call,
//! so whatever these catalogs list can be asked directly.
//!
//! Decision: discovery only runs against the vendors' own hosts. A custom base
//! URL may resolve to private infrastructure at request time, so a provider
//! with one keeps the registry's decision models only (the same rule as
//! OpenRouter chat discovery).

use chrono::{DateTime, TimeZone, Utc};
use everruns_contracts::ProviderEndpoint;
use everruns_contracts::error::{AgentLoopError, Result};
use everruns_contracts::model::{DecisionModelProfile, ModelCost, ModelProfile};
use everruns_contracts::model_discovery::DiscoveredModel;
use everruns_contracts::openai_protocol::url_host_eq;
use serde::Deserialize;

const TYPESAFE_HOST: &str = "api.typesafe.ai";
const OPENROUTER_HOST: &str = "openrouter.ai";
/// Catalog responses are small (OpenRouter's whole decision list is ~35 KB).
const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;

/// List TypeSafe's models, or `None` when the endpoint is not TypeSafe's host.
pub(super) async fn typesafe(
    http: &reqwest::Client,
    endpoint: &ProviderEndpoint,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let Some(url) = endpoint
        .url("models")
        .filter(|url| url_host_eq(url, TYPESAFE_HOST))
    else {
        return Ok(None);
    };
    let response: TypeSafeModels = fetch(http, endpoint, url).await?;
    Ok(Some(
        response
            .models
            .into_iter()
            .map(TypeSafeModel::into_discovered)
            .collect(),
    ))
}

/// List OpenRouter's decision models, or `None` when the endpoint is not
/// OpenRouter's host.
pub(super) async fn openrouter(
    http: &reqwest::Client,
    endpoint: &ProviderEndpoint,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let Some(url) = endpoint
        .url("models?output_modalities=decisions")
        .filter(|url| url_host_eq(url, OPENROUTER_HOST))
    else {
        return Ok(None);
    };
    let response: OpenRouterModels = fetch(http, endpoint, url).await?;
    Ok(Some(
        response
            .data
            .into_iter()
            // The filter is a query parameter; keep the guarantee local too.
            .filter(OpenRouterModel::is_decision_model)
            .map(OpenRouterModel::into_discovered)
            .collect(),
    ))
}

async fn fetch<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    endpoint: &ProviderEndpoint,
    url: String,
) -> Result<T> {
    let resolved = endpoint.resolve("GET", url, &[]).await?;
    let mut request = http.get(&resolved.url);
    for (name, value) in &resolved.headers {
        request = request.header(name, value);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| invalid("Decision model catalog transport failed"))?;
    if !response.status().is_success() {
        return Err(invalid(format!(
            "Decision model catalog returned HTTP {}",
            response.status().as_u16()
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| invalid("Decision model catalog transport failed"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_CATALOG_BYTES {
            return Err(invalid("Decision model catalog exceeds size limit"));
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| invalid("Invalid decision model catalog"))
}

fn invalid(message: impl Into<String>) -> AgentLoopError {
    AgentLoopError::llm(message)
}

/// What every System One model answers through our driver: all three
/// primitives, calibrated, within the driver's own limits.
fn decision_profile(name: String, family: String) -> ModelProfile {
    let mut profile = ModelProfile::minimal(name);
    profile.family = family;
    profile.structured_output = true;
    profile.decisions = Some(DecisionModelProfile {
        primitives: vec!["noul".into(), "choice".into(), "score".into()],
        calibrated: true,
        max_choice_options: Some(255),
        max_score_levels: Some(10),
        state_tokens: None,
        request_tokens: None,
    });
    profile
}

fn decision_model(
    model_id: String,
    display_name: Option<String>,
    created_at: Option<DateTime<Utc>>,
    owned_by: Option<String>,
    profile: ModelProfile,
) -> DiscoveredModel {
    DiscoveredModel {
        model_id,
        display_name,
        created_at,
        owned_by,
        capabilities: vec!["decisions".into()],
        discovered_profile: Some(profile),
    }
}

#[derive(Deserialize)]
struct TypeSafeModels {
    #[serde(default)]
    models: Vec<TypeSafeModel>,
}

#[derive(Deserialize)]
struct TypeSafeModel {
    name: String,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    release_date: Option<DateTime<Utc>>,
}

impl TypeSafeModel {
    fn into_discovered(self) -> DiscoveredModel {
        let mut profile = decision_profile(self.name.clone(), "jev".into());
        profile.description = self.description;
        profile.release_date = self
            .release_date
            .map(|date| date.date_naive().format("%Y-%m-%d").to_string());
        decision_model(
            self.name,
            None,
            self.release_date,
            Some("typesafe".into()),
            profile,
        )
    }
}

#[derive(Deserialize)]
struct OpenRouterModels {
    #[serde(default)]
    data: Vec<OpenRouterModel>,
}

#[derive(Deserialize)]
struct OpenRouterModel {
    id: String,
    #[serde(default)]
    canonical_slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    context_length: Option<i64>,
    #[serde(default)]
    architecture: Option<OpenRouterArchitecture>,
    #[serde(default)]
    pricing: Option<OpenRouterPricing>,
}

#[derive(Deserialize)]
struct OpenRouterArchitecture {
    #[serde(default)]
    output_modalities: Vec<String>,
}

#[derive(Deserialize)]
struct OpenRouterPricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

impl OpenRouterModel {
    fn is_decision_model(&self) -> bool {
        self.architecture.as_ref().is_some_and(|arch| {
            arch.output_modalities
                .iter()
                .any(|m| m.eq_ignore_ascii_case("decisions"))
        })
    }

    fn into_discovered(self) -> DiscoveredModel {
        let name = self.name.clone().unwrap_or_else(|| self.id.clone());
        let family = self.canonical_slug.unwrap_or_else(|| self.id.clone());
        let mut profile = decision_profile(name, family);
        profile.description = self.description;
        profile.cost = self.pricing.as_ref().and_then(OpenRouterPricing::to_cost);
        if let Some(decisions) = profile.decisions.as_mut() {
            // OpenRouter reports one window for the whole request.
            decisions.request_tokens = self
                .context_length
                .filter(|tokens| *tokens > 0)
                .and_then(|tokens| usize::try_from(tokens).ok());
        }
        decision_model(
            self.id.clone(),
            self.name,
            self.created
                .and_then(|seconds| Utc.timestamp_opt(seconds, 0).single()),
            self.id
                .trim_start_matches('~')
                .split('/')
                .next()
                .map(str::to_owned),
            profile,
        )
    }
}

impl OpenRouterPricing {
    /// OpenRouter prices per token as decimal strings; profiles price per 1M.
    fn to_cost(&self) -> Option<ModelCost> {
        let per_million = |price: &str| {
            price
                .parse::<f64>()
                .ok()
                .filter(|price| price.is_finite() && *price >= 0.0)
                .map(|price| price * 1_000_000.0)
        };
        let input = per_million(self.prompt.as_deref()?)?;
        let output = self
            .completion
            .as_deref()
            .and_then(per_million)
            .unwrap_or(0.0);
        Some(ModelCost::new(input, output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use everruns_contracts::BearerAuth;
    use serde_json::json;
    use wiremock::matchers::{header, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn endpoint(base: &str) -> ProviderEndpoint {
        everruns_contracts::Provider::services("test")
            .base_url(base)
            .auth(BearerAuth::new("key"))
            .endpoint()
            .clone()
    }

    #[tokio::test]
    async fn catalogs_skip_hosts_other_than_the_vendors() {
        // A custom base URL never gets a discovery request: no server is
        // listening, so any request would fail rather than return `None`.
        let http = reqwest::Client::new();
        let custom = endpoint("http://127.0.0.1:9/v1");
        assert!(typesafe(&http, &custom).await.unwrap().is_none());
        assert!(openrouter(&http, &custom).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn typesafe_models_parse_into_calibrated_decision_models() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", "Bearer key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": [
                {"name": "jev-latest", "description": "Latest Jev", "release_date": "2026-10-22T02:00:00+00:00"},
                {"name": "jev-preview"}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        let response: TypeSafeModels = fetch(
            &reqwest::Client::new(),
            &endpoint(&server.uri()),
            format!("{}/v1/models", server.uri()),
        )
        .await
        .unwrap();
        let models: Vec<_> = response
            .models
            .into_iter()
            .map(TypeSafeModel::into_discovered)
            .collect();
        assert_eq!(models[0].model_id, "jev-latest");
        assert_eq!(models[0].capabilities, ["decisions"]);
        let profile = models[0].discovered_profile.as_ref().unwrap();
        assert_eq!(profile.release_date.as_deref(), Some("2026-10-22"));
        assert!(profile.decisions.as_ref().unwrap().calibrated);
        assert_eq!(models[1].model_id, "jev-preview");
    }

    #[tokio::test]
    async fn openrouter_keeps_only_decision_models_and_prices_per_million() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/models"))
            .and(query_param("output_modalities", "decisions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
                {"id": "~typesafe/jev-latest", "name": "TypeSafe: Jev Latest", "created": 1790000000,
                 "context_length": 262144,
                 "architecture": {"output_modalities": ["decisions"]},
                 "pricing": {"prompt": "0.000000042", "completion": "0"}},
                {"id": "openai/gpt-6-luna", "architecture": {"output_modalities": ["text"]}}
            ]})))
            .expect(1)
            .mount(&server)
            .await;
        let response: OpenRouterModels = fetch(
            &reqwest::Client::new(),
            &endpoint(&server.uri()),
            format!("{}/api/v1/models?output_modalities=decisions", server.uri()),
        )
        .await
        .unwrap();
        let models: Vec<_> = response
            .data
            .into_iter()
            .filter(OpenRouterModel::is_decision_model)
            .map(OpenRouterModel::into_discovered)
            .collect();
        assert_eq!(models.len(), 1);
        let model = &models[0];
        assert_eq!(model.model_id, "~typesafe/jev-latest");
        assert_eq!(model.owned_by.as_deref(), Some("typesafe"));
        let profile = model.discovered_profile.as_ref().unwrap();
        let cost = profile.cost.as_ref().unwrap();
        assert!((cost.input - 0.042).abs() < 1e-9);
        assert_eq!(
            profile.decisions.as_ref().unwrap().request_tokens,
            Some(262_144)
        );
    }

    #[tokio::test]
    async fn catalog_errors_surface_as_failures() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
        let result: Result<TypeSafeModels> = fetch(
            &reqwest::Client::new(),
            &endpoint(&server.uri()),
            format!("{}/v1/models", server.uri()),
        )
        .await;
        assert!(result.is_err());
    }
}
