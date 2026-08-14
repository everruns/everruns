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

    struct NoCatalog;

    #[async_trait::async_trait]
    impl crate::ChatDriver for NoCatalog {
        async fn chat_completion_stream(
            &self,
            _endpoint: &crate::ProviderEndpoint,
            _messages: Vec<crate::LlmMessage>,
            _config: &crate::LlmCallConfig,
        ) -> crate::Result<crate::LlmResponseStream> {
            unreachable!()
        }
    }

    fn bare_discovered(model_id: &str) -> DiscoveredModel {
        DiscoveredModel {
            capabilities: vec!["chat".to_string()],
            model_id: model_id.to_string(),
            display_name: None,
            created_at: None,
            owned_by: None,
            discovered_profile: None,
        }
    }

    fn model(id: &str) -> DiscoveredProviderModel {
        DiscoveredProviderModel {
            model_id: id.to_string(),
            display_name: None,
            description: None,
        }
    }

    #[tokio::test]
    async fn discovery_is_unsupported_for_llmsim() {
        // The offline simulator has no models API; discovery must signal
        // "unsupported" rather than erroring, so callers keep curated lists.
        let mut registry = DriverRegistry::new();
        registry.register(DriverId::LlmSim, |_| Box::new(NoCatalog));
        let result = discover_provider_models(&registry, &ProviderConfig::new(DriverId::LlmSim))
            .await
            .expect("llmsim discovery should not error");
        assert!(result.is_none());
    }

    #[test]
    fn enrichment_fills_names_and_descriptions_from_profiles() {
        let enriched = enrich_with_profiles(&DriverId::OpenAI, vec![bare_discovered("gpt-5.5")]);

        assert_eq!(enriched.len(), 1);
        assert_eq!(enriched[0].model_id, "gpt-5.5");
        assert_eq!(enriched[0].display_name.as_deref(), Some("GPT-5.5"));
        assert!(
            enriched[0].description.is_some(),
            "profile description should be carried over"
        );
    }

    #[test]
    fn enrichment_prefers_api_display_name_over_profile() {
        let mut discovered = bare_discovered("gpt-5.5");
        discovered.display_name = Some("GPT-5.5 (via gateway)".to_string());

        let enriched = enrich_with_profiles(&DriverId::OpenAI, vec![discovered]);

        assert_eq!(
            enriched[0].display_name.as_deref(),
            Some("GPT-5.5 (via gateway)")
        );
    }

    #[test]
    fn enrichment_keeps_unknown_models_with_bare_ids() {
        let enriched = enrich_with_profiles(
            &DriverId::OpenAI,
            vec![bare_discovered("totally-new-model")],
        );

        assert_eq!(enriched[0].model_id, "totally-new-model");
        assert!(enriched[0].display_name.is_none());
        assert!(enriched[0].description.is_none());
    }

    #[test]
    fn normalization_strips_gemini_style_prefixes_and_sorts_newest_first() {
        let mut older = bare_discovered("models/qwen3");
        older.created_at = chrono::DateTime::from_timestamp(1_600_000_000, 0);
        let mut newer = bare_discovered("llama3.2:latest");
        newer.created_at = chrono::DateTime::from_timestamp(1_700_000_000, 0);

        let normalized = normalize_and_enrich(&DriverId::OpenAI, vec![older, newer]);

        let ids: Vec<&str> = normalized.iter().map(|m| m.model_id.as_str()).collect();
        assert_eq!(ids, &["llama3.2:latest", "qwen3"]);
    }

    #[test]
    fn aggregator_ranking_puts_curated_and_current_first_then_sorts_rest() {
        let ranked = rank_discovered_models(
            &DriverId::OpenRouter,
            vec![
                model("zai/glm-5"),
                model("openai/gpt-5.5"),
                model("anthropic/claude-opus-4-8"),
                model("moon/kimi-k3"),
            ],
            Some("moon/kimi-k3"),
            &["openai/gpt-5.5", "anthropic/claude-opus-4-8"],
        );

        assert_eq!(ranked.recommended_count, 3);
        let ids: Vec<&str> = ranked.models.iter().map(|m| m.model_id.as_str()).collect();
        assert_eq!(
            ids,
            &[
                "openai/gpt-5.5",
                "anthropic/claude-opus-4-8",
                "moon/kimi-k3",
                "zai/glm-5",
            ]
        );
    }

    #[test]
    fn curated_ids_absent_from_the_catalog_are_not_recommended() {
        let ranked = rank_discovered_models(
            &DriverId::OpenRouter,
            vec![model("zai/glm-5")],
            None,
            &["openai/gpt-5.5"],
        );

        assert_eq!(ranked.recommended_count, 0);
        assert_eq!(ranked.models.len(), 1);
    }

    #[test]
    fn single_vendor_providers_keep_discovery_order() {
        let ranked = rank_discovered_models(
            &DriverId::OpenAI,
            vec![model("gpt-5.5"), model("gpt-5.2")],
            None,
            &["gpt-5.2"],
        );

        assert_eq!(ranked.recommended_count, 0);
        let ids: Vec<&str> = ranked.models.iter().map(|m| m.model_id.as_str()).collect();
        assert_eq!(ids, &["gpt-5.5", "gpt-5.2"]);
    }

    #[test]
    fn bare_model_id_strips_reasoning_effort_suffix() {
        assert_eq!(
            bare_model_id("nvidia/nemotron-3-super-120b-a12b high"),
            "nvidia/nemotron-3-super-120b-a12b"
        );
    }

    /// The explicit OpenAI-compatible discovery helper uses provider-resolved
    /// request metadata rather than inferring auth from the endpoint host.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn openai_compatible_fallback_lists_models() {
        use std::io::{Read, Write};

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind mock server");
        let addr = listener.local_addr().expect("mock server addr");
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let body = r#"{"object":"list","data":[
                {"id":"llama3.2:latest","object":"model","created":1700000000,"owned_by":"library"},
                {"id":"models/qwen3","object":"model"}
            ]}"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                body.len(),
            );
            stream
                .write_all(response.as_bytes())
                .expect("write response");
        });

        let discovered = list_openai_compatible_models_with_client(
            &reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("build mock HTTP client"),
            &crate::ResolvedProviderRequest {
                url: format!("http://{addr}/v1/models"),
                headers: Vec::new(),
            },
        )
        .await
        .expect("fallback discovery should succeed")
        .expect("endpoint lists models");
        server.join().expect("mock server thread");

        let presented = normalize_and_enrich(&DriverId::OpenAI, discovered);
        let ids: Vec<&str> = presented.iter().map(|m| m.model_id.as_str()).collect();
        assert!(ids.contains(&"llama3.2:latest"), "ids: {ids:?}");
        // Gemini-style `models/` prefixes are normalized to bare ids.
        assert!(ids.contains(&"qwen3"), "ids: {ids:?}");
    }

    #[tokio::test]
    async fn openai_compatible_fallback_blocks_internal_addresses() {
        list_openai_compatible_models_with_client(
            &shared_request_http_client(),
            &crate::ResolvedProviderRequest {
                url: "http://127.0.0.1:9/v1/models".into(),
                headers: Vec::new(),
            },
        )
        .await
        .expect_err("model discovery must use the provider SSRF guard");
    }

    fn presented(id: &str, display_name: Option<&str>) -> DiscoveredProviderModel {
        DiscoveredProviderModel {
            model_id: id.to_string(),
            display_name: display_name.map(str::to_string),
            description: None,
        }
    }

    #[test]
    fn matching_is_case_insensitive_over_id_and_display_name() {
        let catalog = vec![
            presented("openai/gpt-5.5", Some("GPT-5.5")),
            presented("moon/luna-1", None),
            presented("acme/nebula", Some("Luna Nebula")),
        ];

        let by_id = match_models("openrouter", &catalog, "LUNA");
        let ids: Vec<&str> = by_id.iter().map(|m| m.model_id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["moon/luna-1", "acme/nebula"],
            "a display-name hit counts as much as an id hit"
        );
        assert!(by_id.iter().all(|m| m.provider == "openrouter"));
    }

    #[test]
    fn an_empty_query_matches_nothing_rather_than_everything() {
        // Returning the whole catalog for an empty query would flood the caller
        // and, in a tool context, the model.
        let catalog = vec![presented("openai/gpt-5.5", None)];
        assert!(match_models("openrouter", &catalog, "   ").is_empty());
    }

    #[tokio::test]
    async fn search_skips_catalog_less_providers_and_records_failures() {
        let mut registry = DriverRegistry::new();
        registry.register(DriverId::LlmSim, |_| Box::new(NoCatalog));
        let providers = vec![
            // No catalog to offer: skipped, not an error.
            ("sim".to_string(), ProviderConfig::new(DriverId::LlmSim)),
            // No driver registered and no base URL for the HTTP fallback: an
            // error against this provider, which must not sink the search.
            (
                "broken".to_string(),
                ProviderConfig::new(DriverId::OpenAI).with_api_key("k"),
            ),
        ];

        let result = search_provider_models(&registry, &providers, "gpt").await;

        assert!(result.matches.is_empty());
        assert!(
            !result.providers_searched.contains(&"sim".to_string()),
            "a provider with no catalog was not searched"
        );
        assert_eq!(result.provider_errors.len(), 1);
        assert!(result.provider_errors[0].starts_with("broken: "));
    }

    #[tokio::test]
    async fn an_empty_query_short_circuits_before_any_provider_call() {
        let registry = DriverRegistry::new();
        let providers = vec![(
            "broken".to_string(),
            ProviderConfig::new(DriverId::OpenAI).with_api_key("k"),
        )];

        let result = search_provider_models(&registry, &providers, "").await;

        assert_eq!(result, ModelSearchResult::default());
    }
}
