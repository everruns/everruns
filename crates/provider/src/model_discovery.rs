//! Provider model discovery and display ranking.
//!
//! Ported from yolop, where every host that offers a model picker had to
//! reimplement the same three steps: ask the driver for a catalog, fall back to
//! the OpenAI-compatible `GET <base>/models` for endpoints the drivers decline,
//! and merge the answer with [`crate::model_profiles`] so bare ids still render
//! human-readable names. None of that is host-specific, so it lives beside the
//! driver registry and the profile registry it depends on.
//!
//! Discovery is deliberately three-valued: `Ok(None)` means "this provider has
//! no catalog to offer" and callers should keep their curated suggestions,
//! while `Err` means the catalog request itself failed.

#[cfg(feature = "http")]
use crate::driver_helpers::shared_request_http_client;
use crate::driver_registry::{DiscoveredModel, DriverId, DriverRegistry, ProviderConfig};
#[cfg(feature = "http")]
use crate::error::AgentLoopError;
use crate::error::Result;
use crate::model_profiles::get_model_profile;

/// One model offered by a provider, ready for display: the bare id plus
/// human-readable metadata merged from the provider's API response and the
/// model profile registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiscoveredProviderModel {
    /// Bare model id, as chat calls and profile lookups expect it.
    pub model_id: String,
    /// Human-readable name, when the provider or a profile supplies one.
    pub display_name: Option<String>,
    /// Short description, when a profile or the provider supplies one.
    pub description: Option<String>,
}

/// Query a provider's models API through its driver.
///
/// Returns `Ok(None)` when the provider (or its custom endpoint) does not
/// support model listing; callers should fall back to curated suggestions in
/// that case rather than treating it as an error.
///
pub async fn discover_provider_models(
    registry: &DriverRegistry,
    config: &ProviderConfig,
) -> Result<Option<Vec<DiscoveredProviderModel>>> {
    let driver = registry.create_chat_driver(config)?;
    let Some(models) = driver
        .list_models(&crate::runtime_provider::ProviderEndpoint::default())
        .await?
    else {
        return Ok(None);
    };

    Ok(Some(normalize_and_enrich(&config.provider_type, models)))
}

/// Normalize discovered ids, sort newest-first, and merge in profile metadata.
///
/// Split out from [`discover_provider_models`] so a host that obtained a
/// catalog some other way (a cached response, a proxy's own endpoint) gets the
/// same presentation.
pub fn normalize_and_enrich(
    provider_type: &DriverId,
    mut models: Vec<DiscoveredModel>,
) -> Vec<DiscoveredProviderModel> {
    for model in models.iter_mut() {
        // Gemini's OpenAI-compatible surface reports ids as `models/<id>`; the
        // bare id is what chat calls and profile lookups expect.
        if let Some(bare) = model.model_id.strip_prefix("models/") {
            model.model_id = bare.to_string();
        }
    }
    models.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| a.model_id.cmp(&b.model_id))
    });
    enrich_with_profiles(provider_type, models)
}

/// Merge each discovered model with metadata from the model profile registry.
///
/// The curated profile wins for descriptions (short, written for display); the
/// provider's API response wins for display names, since it knows its own
/// catalog best (e.g. OpenRouter's `name` field), with the profile filling the
/// gap for APIs that return bare ids (e.g. OpenAI).
pub fn enrich_with_profiles(
    provider_type: &DriverId,
    models: Vec<DiscoveredModel>,
) -> Vec<DiscoveredProviderModel> {
    models
        .into_iter()
        .map(|model| {
            let core_profile = get_model_profile(provider_type, &model.model_id);
            let api_profile = model.discovered_profile;
            let display_name = model
                .display_name
                .filter(|name| !name.is_empty() && *name != model.model_id)
                .or_else(|| core_profile.as_ref().map(|profile| profile.name.clone()));
            let description = core_profile
                .as_ref()
                .and_then(|profile| profile.description.clone())
                .or_else(|| {
                    api_profile
                        .as_ref()
                        .and_then(|profile| profile.description.clone())
                });
            DiscoveredProviderModel {
                model_id: model.model_id,
                display_name,
                description,
            }
        })
        .collect()
}

#[cfg(feature = "http")]
#[derive(serde::Deserialize)]
struct OpenAiCompatibleModelsResponse {
    data: Vec<OpenAiCompatibleModel>,
}

#[cfg(feature = "http")]
#[derive(serde::Deserialize)]
struct OpenAiCompatibleModel {
    id: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    owned_by: Option<String>,
}

/// Discovery fallback for OpenAI-compatible endpoints no driver recognizes:
/// `GET <base>/models` with bearer auth.
#[cfg(feature = "http")]
pub async fn list_openai_compatible_models(
    endpoint: &crate::runtime_provider::ProviderEndpoint,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let url = endpoint
        .url("models")
        .ok_or_else(|| AgentLoopError::config("provider endpoint is not configured"))?;
    // IP literals bypass the HTTP client's DNS resolver. Validate the target
    // before resolving authentication, then retain request-time DNS checks.
    crate::url_validation::validate_safe_url(&url)
        .map_err(|error| AgentLoopError::config(format!("unsafe models endpoint: {error}")))?;
    let resolved = endpoint.resolve("GET", url, &[]).await?;
    // THREAT[TM-API-013]: Provider base URLs are org-configurable. Use the
    // shared client so redirects, private DNS results, and hung responses are
    // rejected at request time.
    list_openai_compatible_models_with_client(&shared_request_http_client(), &resolved).await
}

#[cfg(feature = "http")]
async fn list_openai_compatible_models_with_client(
    client: &reqwest::Client,
    resolved: &crate::runtime_provider::ResolvedProviderRequest,
) -> Result<Option<Vec<DiscoveredModel>>> {
    let mut request = client.get(&resolved.url);
    for (name, value) in &resolved.headers {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|error| AgentLoopError::llm(format!("fetch models: {error}")))?;
    if !response.status().is_success() {
        return Err(AgentLoopError::llm(format!(
            "models API returned {}",
            response.status()
        )));
    }
    let parsed: OpenAiCompatibleModelsResponse = response
        .json()
        .await
        .map_err(|error| AgentLoopError::llm(format!("parse models response: {error}")))?;
    let models = parsed
        .data
        .into_iter()
        .map(|model| DiscoveredModel {
            capabilities: vec!["chat".to_string()],
            created_at: model
                .created
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0)),
            display_name: None,
            owned_by: model.owned_by,
            model_id: model.id,
            discovered_profile: None,
        })
        .collect();
    Ok(Some(models))
}

/// Models reordered for display, plus how many leading entries belong in the
/// recommended section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RankedDiscoveredModels {
    /// Recommended models first, then the rest of the catalog.
    pub models: Vec<DiscoveredProviderModel>,
    /// How many leading entries of `models` are recommendations.
    pub recommended_count: usize,
}

/// Cap on the recommended block, so it stays a shortlist rather than a second
/// full catalog.
const RECOMMENDED_CAP: usize = 20;

/// Reorder discovered models for a picker.
///
/// Aggregator catalogs (OpenRouter lists several hundred models) get a short
/// recommended block — `curated` ids that are actually offered, then the active
/// model, then profile-known flagships from major vendors — followed by the rest
/// sorted by id. Single-vendor providers already return a useful order
/// (newest-first from discovery) and are left alone.
///
/// `curated` entries may carry a trailing reasoning-effort suffix
/// (`"vendor/model high"`); only the leading token is matched.
pub fn rank_discovered_models(
    provider_type: &DriverId,
    models: Vec<DiscoveredProviderModel>,
    current_model: Option<&str>,
    curated: &[&str],
) -> RankedDiscoveredModels {
    if provider_type == &DriverId::OpenRouter {
        rank_aggregator_models(provider_type, models, current_model, curated)
    } else {
        RankedDiscoveredModels {
            recommended_count: 0,
            models,
        }
    }
}

fn rank_aggregator_models(
    provider_type: &DriverId,
    models: Vec<DiscoveredProviderModel>,
    current_model: Option<&str>,
    curated: &[&str],
) -> RankedDiscoveredModels {
    let mut recommended_ids: Vec<String> = Vec::new();

    for suggestion in curated {
        let bare = bare_model_id(suggestion);
        if models.iter().any(|model| model.model_id == bare) {
            push_unique(&mut recommended_ids, bare.to_string());
        }
    }

    if let Some(current) = current_model.map(bare_model_id)
        && models.iter().any(|model| model.model_id == current)
    {
        push_unique(&mut recommended_ids, current.to_string());
    }

    // Curated and active selections share the same display cap as profiles.
    recommended_ids.truncate(RECOMMENDED_CAP);

    let mut profile_candidates: Vec<String> = models
        .iter()
        .filter(|model| {
            !recommended_ids.contains(&model.model_id)
                && is_major_vendor_model(&model.model_id)
                && get_model_profile(provider_type, &model.model_id).is_some()
        })
        .map(|model| model.model_id.clone())
        .collect();
    profile_candidates.sort();
    for model_id in profile_candidates {
        if recommended_ids.len() >= RECOMMENDED_CAP {
            break;
        }
        push_unique(&mut recommended_ids, model_id);
    }

    let recommended_count = recommended_ids.len();
    let mut ranked = Vec::with_capacity(models.len());
    for model_id in &recommended_ids {
        if let Some(index) = models.iter().position(|model| &model.model_id == model_id) {
            ranked.push(models[index].clone());
        }
    }

    let mut rest: Vec<DiscoveredProviderModel> = models
        .into_iter()
        .filter(|model| !recommended_ids.contains(&model.model_id))
        .collect();
    rest.sort_by(|a, b| a.model_id.cmp(&b.model_id));
    ranked.extend(rest);

    RankedDiscoveredModels {
        models: ranked,
        recommended_count,
    }
}

/// Strip a trailing reasoning-effort suffix from a model spec
/// (`"nvidia/nemotron-3 high"` → `"nvidia/nemotron-3"`).
pub fn bare_model_id(spec: &str) -> &str {
    spec.split_whitespace().next().unwrap_or(spec)
}

fn push_unique(ids: &mut Vec<String>, id: String) {
    if !ids.contains(&id) {
        ids.push(id);
    }
}

fn is_major_vendor_model(model_id: &str) -> bool {
    model_id.starts_with("openai/")
        || model_id.starts_with("anthropic/")
        || model_id.starts_with("google/")
        || model_id.starts_with("nvidia/")
}

/// One model matched by [`search_provider_models`], qualified by the provider
/// it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelSearchMatch {
    /// Caller-supplied label for the provider that offers this model.
    pub provider: String,
    /// Exact model id, as it must be passed back to the provider.
    pub model_id: String,
    /// Human-readable name, when known.
    pub display_name: Option<String>,
}

/// Outcome of a search across several providers.
///
/// Partial results are the normal case, so failures are reported alongside
/// matches rather than replacing them: one provider being down or holding a
/// stale key should not hide the models the others offer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ModelSearchResult {
    /// Matches, sorted by provider then model id.
    pub matches: Vec<ModelSearchMatch>,
    /// Providers that were actually queried (a provider with no catalog is
    /// skipped rather than reported as an error).
    pub providers_searched: Vec<String>,
    /// Per-provider failures, as `"<provider>: <error>"`.
    pub provider_errors: Vec<String>,
}

/// Search a provider's already-discovered catalog for `query`.
///
/// Case-insensitive substring match over the model id and display name. Split
/// out from the fan-out so the matching rule is testable on its own and reusable
/// by a host that keeps its own catalog.
pub fn match_models(
    provider: &str,
    models: &[DiscoveredProviderModel],
    query: &str,
) -> Vec<ModelSearchMatch> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    models
        .iter()
        .filter(|model| {
            model.model_id.to_lowercase().contains(&needle)
                || model
                    .display_name
                    .as_deref()
                    .is_some_and(|name| name.to_lowercase().contains(&needle))
        })
        .map(|model| ModelSearchMatch {
            provider: provider.to_string(),
            model_id: model.model_id.clone(),
            display_name: model.display_name.clone(),
        })
        .collect()
}

/// Search every supplied provider's catalog for `query`.
///
/// This is what turns "use the luna model" into a set of exact, provider-
/// qualified ids a caller can act on, instead of sending an invented literal to
/// a provider. Each entry in `providers` is a caller-chosen label paired with
/// the config to query; the label is what comes back on each match.
///
/// Providers are queried in the order given. A provider with no catalog
/// (`Ok(None)`) is silently skipped — that is "nothing to search", not a
/// failure — while a provider that errors is recorded in `provider_errors` and
/// the search continues.
pub async fn search_provider_models(
    registry: &DriverRegistry,
    providers: &[(String, ProviderConfig)],
    query: &str,
) -> ModelSearchResult {
    let mut result = ModelSearchResult::default();
    if query.trim().is_empty() {
        return result;
    }

    for (label, config) in providers {
        match discover_provider_models(registry, config).await {
            Ok(Some(models)) => {
                result.providers_searched.push(label.clone());
                result.matches.extend(match_models(label, &models, query));
            }
            Ok(None) => {}
            Err(error) => result.provider_errors.push(format!("{label}: {error}")),
        }
    }

    result
        .matches
        .sort_by(|a, b| (&a.provider, &a.model_id).cmp(&(&b.provider, &b.model_id)));
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct NoCatalog;
    #[async_trait::async_trait]
    impl crate::ChatDriver for NoCatalog {
        async fn chat_completion_stream(
            &self,
            _: &crate::ProviderEndpoint,
            _: Vec<crate::LlmMessage>,
            _: &crate::LlmCallConfig,
        ) -> crate::Result<crate::LlmResponseStream> {
            unreachable!()
        }
    }
    fn bare_discovered(id: &str) -> DiscoveredModel {
        DiscoveredModel {
            model_id: id.into(),
            display_name: None,
            created_at: None,
            owned_by: None,
            capabilities: vec!["chat".into()],
            discovered_profile: None,
        }
    }
    fn model(id: &str) -> DiscoveredProviderModel {
        DiscoveredProviderModel {
            model_id: id.into(),
            display_name: None,
            description: None,
        }
    }

    #[test]
    fn enrichment_preserves_complete_records_and_metadata_precedence() {
        let description = "Flagship reasoning model. Best for complex multi-step tasks, code generation, and deep analysis.";
        for name in [None, Some(""), Some("gpt-5.5"), Some("Gateway name")] {
            let mut input = bare_discovered("gpt-5.5");
            input.display_name = name.map(str::to_string);
            let mut api_profile = get_model_profile(&DriverId::OpenAI, "gpt-5.5").unwrap();
            api_profile.description = Some("API description".into());
            input.discovered_profile = Some(api_profile.clone());
            assert_eq!(
                enrich_with_profiles(&DriverId::OpenAI, vec![input]),
                vec![DiscoveredProviderModel {
                    model_id: "gpt-5.5".into(),
                    display_name: Some(
                        if name == Some("Gateway name") {
                            "Gateway name"
                        } else {
                            "GPT-5.5"
                        }
                        .into()
                    ),
                    description: Some(description.into())
                }]
            );
            let mut unknown = bare_discovered("totally-new-model");
            unknown.display_name = Some("New model".into());
            unknown.discovered_profile = Some(api_profile);
            assert_eq!(
                enrich_with_profiles(
                    &DriverId::OpenAI,
                    vec![unknown, bare_discovered("bare-unknown")]
                ),
                vec![
                    DiscoveredProviderModel {
                        model_id: "totally-new-model".into(),
                        display_name: Some("New model".into()),
                        description: Some("API description".into())
                    },
                    model("bare-unknown")
                ]
            );
        }
    }

    #[test]
    fn normalization_preserves_payload_and_sorts_dates_then_bare_ids() {
        let mut older = bare_discovered("models/qwen3");
        older.created_at = chrono::DateTime::from_timestamp(1_600_000_000, 0);
        older.display_name = Some("Qwen display".into());
        let mut newer = bare_discovered("llama3.2:latest");
        newer.created_at = chrono::DateTime::from_timestamp(1_700_000_000, 0);
        let mut tied = bare_discovered("models/alpha");
        tied.created_at = newer.created_at;
        assert_eq!(
            normalize_and_enrich(
                &DriverId::OpenAI,
                vec![
                    bare_discovered("z-no-date"),
                    older,
                    newer,
                    tied,
                    bare_discovered("a-no-date")
                ]
            ),
            vec![
                model("alpha"),
                model("llama3.2:latest"),
                DiscoveredProviderModel {
                    model_id: "qwen3".into(),
                    display_name: Some("Qwen display".into()),
                    description: None
                },
                model("a-no-date"),
                model("z-no-date")
            ]
        );
    }

    #[test]
    fn aggregator_ranking_deduplicates_present_selections_and_sorts_remainder() {
        let ranked = rank_discovered_models(
            &DriverId::OpenRouter,
            vec![
                model("zai/glm-5"),
                model("openai/gpt-5.5"),
                model("anthropic/claude-opus-4-8"),
                model("moon/kimi-k3"),
                model("acme/alpha"),
            ],
            Some("moon/kimi-k3 high"),
            &[
                "missing/model",
                "openai/gpt-5.5 high",
                "anthropic/claude-opus-4-8",
                "openai/gpt-5.5",
            ],
        );
        assert_eq!(
            ranked,
            RankedDiscoveredModels {
                recommended_count: 3,
                models: vec![
                    model("openai/gpt-5.5"),
                    model("anthropic/claude-opus-4-8"),
                    model("moon/kimi-k3"),
                    model("acme/alpha"),
                    model("zai/glm-5")
                ]
            }
        );
        assert_eq!(
            rank_discovered_models(
                &DriverId::OpenRouter,
                vec![model("zai/glm-5")],
                Some("absent"),
                &["openai/gpt-5.5"]
            ),
            RankedDiscoveredModels {
                recommended_count: 0,
                models: vec![model("zai/glm-5")]
            }
        );
        let profile_ranked = rank_discovered_models(
            &DriverId::OpenRouter,
            vec![
                model("zai/glm-5"),
                model("nvidia/nemotron-3-super-120b-a12b"),
            ],
            None,
            &[],
        );
        assert_eq!(
            profile_ranked,
            RankedDiscoveredModels {
                recommended_count: 1,
                models: vec![
                    model("nvidia/nemotron-3-super-120b-a12b"),
                    model("zai/glm-5")
                ]
            }
        );
        for (input, expected) in [
            (
                "nvidia/nemotron-3-super-120b-a12b high",
                "nvidia/nemotron-3-super-120b-a12b",
            ),
            ("  model\thigh ", "model"),
            ("model", "model"),
            ("", ""),
        ] {
            assert_eq!(bare_model_id(input), expected);
        }
    }

    #[test]
    fn single_vendor_providers_keep_complete_discovery_order() {
        let input = vec![
            DiscoveredProviderModel {
                model_id: "gpt-5.5".into(),
                display_name: Some("Gateway".into()),
                description: Some("Keep me".into()),
            },
            model("gpt-5.2"),
        ];
        assert_eq!(
            rank_discovered_models(
                &DriverId::OpenAI,
                input.clone(),
                Some("gpt-5.2"),
                &["gpt-5.2"]
            ),
            RankedDiscoveredModels {
                recommended_count: 0,
                models: input
            }
        );
    }

    #[tokio::test]
    async fn compatible_http_catalog_preserves_request_and_complete_model_fields() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{header, method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/models")).and(header("authorization","Bearer synthetic-key")).and(header("x-gateway","tenant-a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[{"id":"llama3.2:latest","created":1700000000,"owned_by":"library"},{"id":"models/qwen3"},{"id":"bad-date","created":9223372036854775807_i64}]}))).expect(1).mount(&server).await;
        let input = crate::ResolvedProviderRequest {
            url: format!("{}/v1/models", server.uri()),
            headers: vec![
                ("authorization".into(), "Bearer synthetic-key".into()),
                ("x-gateway".into(), "tenant-a".into()),
            ],
        };
        let result = list_openai_compatible_models_with_client(
            &reqwest::Client::builder().no_proxy().build().unwrap(),
            &input,
        )
        .await
        .unwrap()
        .unwrap();
        let payload:Vec<_>=result.iter().map(|model|serde_json::json!({"model_id":model.model_id,"display_name":model.display_name,"created_at":model.created_at,"owned_by":model.owned_by,"capabilities":model.capabilities,"discovered_profile":model.discovered_profile})).collect();
        let payload = serde_json::json!(payload);
        assert_eq!(
            payload,
            serde_json::json!([
                {"model_id":"llama3.2:latest","display_name":null,"created_at":"2023-11-14T22:13:20Z","owned_by":"library","capabilities":["chat"],"discovered_profile":null},
                {"model_id":"models/qwen3","display_name":null,"created_at":null,"owned_by":null,"capabilities":["chat"],"discovered_profile":null},
                {"model_id":"bad-date","display_name":null,"created_at":null,"owned_by":null,"capabilities":["chat"],"discovered_profile":null}
            ])
        );
    }

    #[tokio::test]
    async fn compatible_http_empty_catalog_and_failures_remain_distinct() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
        let server = MockServer::start().await;
        for (route, status, body) in [
            ("/empty", 200, r#"{"data":[]}"#),
            ("/denied", 401, "private diagnostic"),
            ("/malformed", 200, r#"{"data":null}"#),
        ] {
            Mock::given(path(route))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .mount(&server)
                .await;
        }
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let empty = list_openai_compatible_models_with_client(
            &client,
            &crate::ResolvedProviderRequest {
                url: format!("{}/empty", server.uri()),
                headers: vec![],
            },
        )
        .await
        .unwrap()
        .unwrap();
        assert!(empty.is_empty());
        let denied = list_openai_compatible_models_with_client(
            &client,
            &crate::ResolvedProviderRequest {
                url: format!("{}/denied", server.uri()),
                headers: vec![],
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            denied.to_string(),
            "LLM error: models API returned 401 Unauthorized"
        );
        let malformed = list_openai_compatible_models_with_client(
            &client,
            &crate::ResolvedProviderRequest {
                url: format!("{}/malformed", server.uri()),
                headers: vec![],
            },
        )
        .await
        .unwrap_err();
        assert!(
            matches!(malformed,AgentLoopError::Llm(ref error) if error.message.starts_with("parse models response: "))
        );
    }

    #[test]
    fn matching_preserves_provider_and_display_fields_without_empty_query_floods() {
        let catalog = vec![
            model("openai/gpt-5.5"),
            model("moon/luna-1"),
            DiscoveredProviderModel {
                model_id: "acme/nebula".into(),
                display_name: Some("Luna Nebula".into()),
                description: None,
            },
        ];
        assert_eq!(
            match_models("gateway", &catalog, " LUNA "),
            vec![
                ModelSearchMatch {
                    provider: "gateway".into(),
                    model_id: "moon/luna-1".into(),
                    display_name: None
                },
                ModelSearchMatch {
                    provider: "gateway".into(),
                    model_id: "acme/nebula".into(),
                    display_name: Some("Luna Nebula".into())
                }
            ]
        );
        for query in ["", "   ", "no-match"] {
            assert!(match_models("gateway", &catalog, query).is_empty());
        }
    }

    struct Catalog {
        id: String,
        calls: Arc<Mutex<Vec<String>>>,
        models: Option<Vec<DiscoveredModel>>,
        fail: bool,
    }
    #[async_trait::async_trait]
    impl crate::ChatDriver for Catalog {
        async fn chat_completion_stream(
            &self,
            _: &crate::ProviderEndpoint,
            _: Vec<crate::LlmMessage>,
            _: &crate::LlmCallConfig,
        ) -> crate::Result<crate::LlmResponseStream> {
            unreachable!()
        }
        async fn list_models(
            &self,
            _: &crate::ProviderEndpoint,
        ) -> Result<Option<Vec<DiscoveredModel>>> {
            self.calls.lock().unwrap().push(self.id.clone());
            if self.fail {
                Err(AgentLoopError::config("unavailable"))
            } else {
                Ok(self.models.clone())
            }
        }
    }

    #[tokio::test]
    async fn search_keeps_partial_success_and_sorts_provider_qualified_matches() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut registry = DriverRegistry::new();
        let mut providers = vec![];
        for (id, models, fail) in [
            (
                "z",
                Some(vec![
                    bare_discovered("models/luna-b"),
                    bare_discovered("luna-a"),
                ]),
                false,
            ),
            ("broken", None, true),
            ("unsupported", None, false),
            ("empty", Some(vec![]), false),
            ("a", Some(vec![bare_discovered("luna-c")]), false),
        ] {
            let observed = calls.clone();
            registry.register_external(id, move |_| {
                Box::new(Catalog {
                    id: id.into(),
                    calls: observed.clone(),
                    models: models.clone(),
                    fail,
                })
            });
            providers.push((id.into(), ProviderConfig::new(DriverId::external(id))));
        }
        assert_eq!(
            search_provider_models(&registry, &providers, "luna").await,
            ModelSearchResult {
                matches: vec![
                    ModelSearchMatch {
                        provider: "a".into(),
                        model_id: "luna-c".into(),
                        display_name: None
                    },
                    ModelSearchMatch {
                        provider: "z".into(),
                        model_id: "luna-a".into(),
                        display_name: None
                    },
                    ModelSearchMatch {
                        provider: "z".into(),
                        model_id: "luna-b".into(),
                        display_name: None
                    }
                ],
                providers_searched: vec!["z".into(), "empty".into(), "a".into()],
                provider_errors: vec!["broken: Configuration error: unavailable".into()]
            }
        );
        assert_eq!(
            *calls.lock().unwrap(),
            ["z", "broken", "unsupported", "empty", "a"]
        );
        calls.lock().unwrap().clear();
        assert_eq!(
            search_provider_models(&registry, &providers, " \t").await,
            ModelSearchResult::default()
        );
        assert!(calls.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn public_discovery_rejects_responding_loopback_before_requesting() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({"data":[]})),
            )
            .mount(&server)
            .await;
        let provider = crate::Provider::new("local", NoCatalog).base_url(server.uri());
        let result = list_openai_compatible_models(provider.endpoint()).await;
        assert!(
            matches!(result, Err(AgentLoopError::Configuration(_))),
            "a responding private endpoint must be rejected, got {result:?}"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn aggregator_recommendation_cap_includes_curated_and_current_entries() {
        let ids: Vec<String> = (0..22).map(|n| format!("custom/model-{n:02}")).collect();
        let catalog: Vec<_> = ids.iter().rev().map(|id| model(id)).collect();
        let curated: Vec<&str> = ids.iter().map(String::as_str).collect();
        let ranked = rank_discovered_models(
            &DriverId::OpenRouter,
            catalog,
            Some("custom/model-21"),
            &curated,
        );
        assert_eq!(ranked.recommended_count, 20);
        assert_eq!(
            ranked.models,
            ids.iter().map(|id| model(id)).collect::<Vec<_>>()
        );
    }
}
