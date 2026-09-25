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
use chrono::{DateTime, Utc};

use crate::driver_registry::{DriverId, DriverRegistry, ProviderConfig};
#[cfg(feature = "http")]
use crate::error::AgentLoopError;
use crate::error::Result;
use crate::model_profiles::get_model_profile;

/// Model information discovered from a provider's list_models API
///
/// Represents a model available from a provider. Used for dynamic model discovery
/// to sync available models from provider APIs into the database.
///
/// The `discovered_profile` field carries structured capability/limit metadata
/// parsed from the provider's API response (e.g., Anthropic's capabilities object).
/// During model sync, this profile is merged with hardcoded profiles: hardcoded
/// values take precedence (they include cost data not available from APIs),
/// but discovered data fills gaps for models without hardcoded profiles.
#[derive(Debug, Clone)]
pub struct DiscoveredModel {
    /// Model identifier (e.g., "gpt-5.2", "claude-opus-4-5-20251101")
    pub model_id: String,
    /// Human-readable display name (if provided by API)
    pub display_name: Option<String>,
    /// When the model was created/released
    pub created_at: Option<DateTime<Utc>>,
    /// Owner or organization (e.g., "openai", "system")
    pub owned_by: Option<String>,
    /// Service capabilities advertised for this concrete model (for example,
    /// `chat` or `embeddings`). These are distinct from provider-level
    /// services: an OpenAI provider supports both, but each model does not.
    pub capabilities: Vec<String>,
    /// Structured profile built from provider API metadata (capabilities, limits).
    /// Populated by drivers that return rich model metadata (e.g., Anthropic /v1/models).
    pub discovered_profile: Option<crate::model::ModelProfile>,
}

/// One model offered by a provider, ready for display: the bare id plus
/// human-readable metadata merged from the provider's API response and the
/// model profile registry.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct DiscoveredProviderModel {
    /// Bare model id, as chat calls and profile lookups expect it.
    pub model_id: String,
    /// Human-readable name, when the provider or a profile supplies one.
    pub display_name: Option<String>,
    /// Short description, when a profile or the provider supplies one.
    pub description: Option<String>,
    /// What this model can do: context window, modalities, reasoning and
    /// tool-calling support, prices.
    ///
    /// Merged from the curated registry and what the provider's own API
    /// reported, so a model the registry has never heard of — a self-hosted
    /// deployment, an id released this morning — still carries the limits its
    /// provider advertises. Enrichment used to drop this, leaving a catalog
    /// consumer with an id and a label and nothing to answer a capability
    /// question with.
    pub profile: Option<crate::model::ModelProfile>,
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
                profile: merge_profiles(core_profile, api_profile),
            }
        })
        .collect()
}

/// Combine the curated profile with the one the provider's API reported.
///
/// The curated profile wins where both speak: it carries what an API never
/// returns (prices, knowledge cutoff) and is written for display. The API
/// fills what curation has not covered — the whole answer for a model the
/// registry does not carry, and the gaps for one it does.
fn merge_profiles(
    core: Option<crate::model::ModelProfile>,
    api: Option<crate::model::ModelProfile>,
) -> Option<crate::model::ModelProfile> {
    match (core, api) {
        (Some(mut core), Some(api)) => {
            core.limits = core.limits.or(api.limits);
            core.modalities = core.modalities.or(api.modalities);
            core.knowledge = core.knowledge.or(api.knowledge);
            core.release_date = core.release_date.or(api.release_date);
            Some(core)
        }
        (core, api) => core.or(api),
    }
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
    // Everything below is metadata a gateway or self-hosted server *may*
    // advertise, in shapes that differ per vendor (OpenRouter, LM Studio,
    // vLLM, llama.cpp, ...). Kept as loose `serde_json::Value` so one field
    // of an unexpected type (a stringly-typed `context_length`, say) cannot
    // fail parsing of the whole catalog entry; `value_*` helpers below
    // coerce each into the type we want, defaulting to "unknown" instead of
    // erroring.
    #[serde(default)]
    name: Option<serde_json::Value>,
    #[serde(default)]
    display_name: Option<serde_json::Value>,
    #[serde(default)]
    description: Option<serde_json::Value>,
    #[serde(default)]
    context_length: Option<serde_json::Value>,
    #[serde(default)]
    context_window: Option<serde_json::Value>,
    #[serde(default)]
    max_context_length: Option<serde_json::Value>,
    #[serde(default)]
    max_model_len: Option<serde_json::Value>,
    #[serde(default)]
    top_provider: Option<serde_json::Value>,
    #[serde(default)]
    meta: Option<serde_json::Value>,
    #[serde(default)]
    max_completion_tokens: Option<serde_json::Value>,
    #[serde(default)]
    max_output_tokens: Option<serde_json::Value>,
    #[serde(default)]
    supported_parameters: Option<serde_json::Value>,
    #[serde(default)]
    reasoning: Option<serde_json::Value>,
    #[serde(default)]
    thinking: Option<serde_json::Value>,
    #[serde(default)]
    supports_reasoning: Option<serde_json::Value>,
    #[serde(default)]
    capabilities: Option<serde_json::Value>,
}

/// Read a JSON string out of a loosely-typed optional field.
#[cfg(feature = "http")]
fn value_str(value: &Option<serde_json::Value>) -> Option<String> {
    value.as_ref()?.as_str().map(str::to_string)
}

/// Read a JSON number out of a loosely-typed optional field, clamped into a
/// non-negative `i32` (context windows and token caps are always positive).
#[cfg(feature = "http")]
fn value_i32(value: &Option<serde_json::Value>) -> Option<i32> {
    let raw = value.as_ref()?;
    let number = raw.as_i64().or_else(|| raw.as_f64().map(|f| f as i64))?;
    Some(number.clamp(0, i32::MAX as i64) as i32)
}

/// Same as [`value_i32`], for one field of a nested object (e.g.
/// `top_provider.context_length`).
#[cfg(feature = "http")]
fn value_nested_i32(value: &Option<serde_json::Value>, key: &str) -> Option<i32> {
    value_i32(&value.as_ref().and_then(|v| v.get(key)).cloned())
}

/// Read a JSON boolean out of a loosely-typed optional field; anything else
/// (missing, wrong type) is simply "not advertised".
#[cfg(feature = "http")]
fn value_bool(value: &Option<serde_json::Value>) -> bool {
    value
        .as_ref()
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Read a JSON array of strings out of a loosely-typed optional field,
/// dropping any element that is not itself a string.
#[cfg(feature = "http")]
fn value_string_list(value: &Option<serde_json::Value>) -> Vec<String> {
    value
        .as_ref()
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Build the discovered profile for one OpenAI-compatible catalog entry, or
/// `None` when the entry advertised nothing beyond the bare `{id, created,
/// owned_by}` shape.
#[cfg(feature = "http")]
fn openai_compatible_profile(
    model: &OpenAiCompatibleModel,
) -> (Option<String>, Option<crate::model::ModelProfile>) {
    let display_name = value_str(&model.name).or_else(|| value_str(&model.display_name));
    let description = value_str(&model.description);
    let context = value_i32(&model.context_length)
        .or_else(|| value_i32(&model.context_window))
        .or_else(|| value_i32(&model.max_context_length))
        .or_else(|| value_i32(&model.max_model_len))
        .or_else(|| value_nested_i32(&model.top_provider, "context_length"))
        .or_else(|| value_nested_i32(&model.meta, "n_ctx_train"));
    let max_output = value_i32(&model.max_completion_tokens)
        .or_else(|| value_i32(&model.max_output_tokens))
        .or_else(|| value_nested_i32(&model.top_provider, "max_completion_tokens"));
    let supported_parameters = value_string_list(&model.supported_parameters);
    let capabilities = value_string_list(&model.capabilities);
    let has = |list: &[String], needle: &str| list.iter().any(|p| p.eq_ignore_ascii_case(needle));

    let reasoning = has(&supported_parameters, "reasoning")
        || has(&supported_parameters, "reasoning_effort")
        || has(&supported_parameters, "thinking")
        || value_bool(&model.reasoning)
        || value_bool(&model.thinking)
        || value_bool(&model.supports_reasoning)
        || has(&capabilities, "thinking")
        || has(&capabilities, "reasoning");
    let tool_call = has(&supported_parameters, "tools")
        || has(&supported_parameters, "tool_choice")
        || has(&capabilities, "tools")
        || has(&capabilities, "tool_use")
        || has(&capabilities, "function_calling");

    let temperature = has(&supported_parameters, "temperature");
    let has_metadata = description.is_some()
        || context.is_some()
        || reasoning
        || tool_call
        || !supported_parameters.is_empty();

    let profile = has_metadata.then(|| crate::model::ModelProfile {
        name: display_name.clone().unwrap_or_else(|| model.id.clone()),
        family: model.id.clone(),
        description: description.clone(),
        release_date: None,
        last_updated: None,
        attachment: false,
        reasoning,
        temperature,
        knowledge: None,
        tool_call,
        structured_output: false,
        open_weights: false,
        cost: None,
        limits: context.map(|context| crate::model::ModelLimits {
            context,
            input: None,
            output: max_output.unwrap_or(context),
            max_media: None,
        }),
        modalities: None,
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        tool_search: false,
        supported_parameters,
        supports_phases: false,
        supports_server_compaction: false,
    });

    (display_name, profile)
}

/// Discovery fallback for OpenAI-compatible endpoints no driver recognizes:
/// `GET <base>/models` with bearer auth.
///
/// Beyond the universal `{id, created, owned_by}` shape, this also picks up
/// whatever richer metadata the gateway or self-hosted server advertises —
/// display name (`name`/`display_name`), `description`, context window
/// (`context_length`, `context_window`, `max_context_length`,
/// `max_model_len`, `top_provider.context_length`, `meta.n_ctx_train`), max
/// output tokens, reasoning/thinking support, and tool-calling support — so a
/// vLLM, LM Studio, llama.cpp, Ollama, or OpenRouter-style catalog fills in
/// `discovered_profile` the same way a native driver would. A bare
/// OpenAI-shaped entry that advertises none of this still gets
/// `discovered_profile: None`, unchanged from before.
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

/// The same fallback, for a driver that has declined to list a host itself.
///
/// A driver returns no catalog for an endpoint it does not recognize — a
/// proxy, a gateway, a self-hosted server. Most of those do serve
/// `GET <base>/models`, and the ones that do not simply answer with an error,
/// so trying costs one request and turns "no catalog" into a real catalog for
/// the common case. A failure is *not* propagated: the caller asked whether a
/// catalog exists, and for an endpoint nobody promised one for, "no" is the
/// answer, not an error.
#[cfg(feature = "http")]
pub async fn list_openai_compatible_models_best_effort(
    endpoint: &crate::runtime_provider::ProviderEndpoint,
) -> Option<Vec<DiscoveredModel>> {
    match list_openai_compatible_models(endpoint).await {
        Ok(models) => models.filter(|models| !models.is_empty()),
        Err(error) => {
            tracing::debug!(%error, "endpoint serves no OpenAI-compatible model listing");
            None
        }
    }
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
        .map(|model| {
            let (display_name, discovered_profile) = openai_compatible_profile(&model);
            DiscoveredModel {
                capabilities: vec!["chat".to_string()],
                created_at: model
                    .created
                    .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0)),
                display_name,
                owned_by: model.owned_by,
                model_id: model.id,
                discovered_profile,
            }
        })
        .collect();
    Ok(Some(models))
}

/// Models reordered for display, plus how many leading entries belong in the
/// recommended section.
#[derive(Clone, Debug, PartialEq)]
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
            _: Vec<crate::Message>,
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
            profile: None,
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
            let curated = get_model_profile(&DriverId::OpenAI, "gpt-5.5");
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
                    description: Some(description.into()),
                    // Curation wins for a model it carries, so the merged
                    // profile is the registry's.
                    profile: curated.clone(),
                }]
            );
            let mut unknown = bare_discovered("totally-new-model");
            unknown.display_name = Some("New model".into());
            unknown.discovered_profile = Some(api_profile.clone());
            assert_eq!(
                enrich_with_profiles(
                    &DriverId::OpenAI,
                    vec![unknown, bare_discovered("bare-unknown")]
                ),
                vec![
                    DiscoveredProviderModel {
                        model_id: "totally-new-model".into(),
                        display_name: Some("New model".into()),
                        description: Some("API description".into()),
                        // Nothing curated to merge with, so what the provider
                        // reported is the whole answer — and it survives.
                        profile: Some(api_profile.clone()),
                    },
                    model("bare-unknown")
                ]
            );
        }
    }

    #[test]
    fn capability_metadata_survives_enrichment_for_a_model_no_registry_carries() {
        // The case a self-hosted or brand-new id lands in: the registry has
        // nothing, so what the endpoint reported is the only answer there is.
        let mut reported = get_model_profile(&DriverId::OpenAI, "gpt-5.5").unwrap();
        reported.name = "Local Qwen".into();
        let mut model = bare_discovered("qwen3-30b");
        model.discovered_profile = Some(reported.clone());

        let enriched = enrich_with_profiles(&DriverId::OpenAI, vec![model]);
        let profile = enriched[0]
            .profile
            .as_ref()
            .expect("the reported capabilities must survive");
        assert_eq!(profile.limits, reported.limits);
        assert_eq!(profile.reasoning, reported.reasoning);
        assert_eq!(profile.tool_call, reported.tool_call);
    }

    #[test]
    fn curation_wins_over_the_api_but_only_where_it_speaks() {
        let curated = get_model_profile(&DriverId::OpenAI, "gpt-5.5").unwrap();
        let mut api = curated.clone();
        api.name = "Gateway label".into();
        api.knowledge = Some("2099-01-01".into());

        let mut bare_curated = curated.clone();
        bare_curated.knowledge = None;
        let merged = merge_profiles(Some(bare_curated), Some(api.clone()))
            .expect("both sides present yields a profile");
        // Curation owns the name; the API filled the gap curation left.
        assert_eq!(merged.name, curated.name);
        assert_eq!(merged.knowledge.as_deref(), Some("2099-01-01"));
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
                    description: None,
                    profile: None,
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
                profile: None,
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
    async fn compatible_http_catalog_preserves_advertised_metadata_and_tolerates_bad_types() {
        use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"data": [
                    // Bare OpenAI-shaped entry: no metadata advertised at all.
                    {"id": "bare-model", "created": 1700000000, "owned_by": "acme"},
                    // OpenRouter-style: name, description, context_length, and a
                    // supported_parameters list implying reasoning + tools.
                    {
                        "id": "openrouter/model",
                        "name": "OpenRouter Model",
                        "description": "A capable chat model.",
                        "context_length": 128000,
                        "supported_parameters": ["reasoning", "tools", "temperature"]
                    },
                    // vLLM-style: max_model_len for context.
                    {"id": "vllm-model", "max_model_len": 32768},
                    // LM Studio-style: max_context_length for context.
                    {"id": "lmstudio-model", "max_context_length": 8192},
                    // A field of the wrong JSON type must not fail the whole
                    // catalog parse; this entry still parses, just without a
                    // usable context window.
                    {
                        "id": "wrong-type-model",
                        "context_length": "big",
                        "description": "Still has a description though."
                    },
                ]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let result = list_openai_compatible_models_with_client(
            &reqwest::Client::builder().no_proxy().build().unwrap(),
            &crate::ResolvedProviderRequest {
                url: server.uri(),
                headers: vec![],
            },
        )
        .await
        .unwrap()
        .unwrap();

        let bare = result.iter().find(|m| m.model_id == "bare-model").unwrap();
        assert_eq!(bare.display_name, None);
        assert!(bare.discovered_profile.is_none());

        let openrouter = result
            .iter()
            .find(|m| m.model_id == "openrouter/model")
            .unwrap();
        assert_eq!(openrouter.display_name.as_deref(), Some("OpenRouter Model"));
        let profile = openrouter.discovered_profile.as_ref().unwrap();
        assert_eq!(profile.name, "OpenRouter Model");
        assert_eq!(profile.family, "openrouter/model");
        assert_eq!(
            profile.description.as_deref(),
            Some("A capable chat model.")
        );
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert_eq!(
            profile.supported_parameters,
            vec![
                "reasoning".to_string(),
                "tools".to_string(),
                "temperature".to_string()
            ]
        );
        let limits = profile.limits.as_ref().unwrap();
        assert_eq!(limits.context, 128000);
        assert_eq!(limits.output, 128000);

        let vllm = result.iter().find(|m| m.model_id == "vllm-model").unwrap();
        let vllm_profile = vllm.discovered_profile.as_ref().unwrap();
        assert_eq!(vllm_profile.limits.as_ref().unwrap().context, 32768);

        let lmstudio = result
            .iter()
            .find(|m| m.model_id == "lmstudio-model")
            .unwrap();
        let lmstudio_profile = lmstudio.discovered_profile.as_ref().unwrap();
        assert_eq!(lmstudio_profile.limits.as_ref().unwrap().context, 8192);

        let wrong_type = result
            .iter()
            .find(|m| m.model_id == "wrong-type-model")
            .unwrap();
        let wrong_type_profile = wrong_type.discovered_profile.as_ref().unwrap();
        // The wrongly-typed context_length is simply ignored, not fatal.
        assert!(wrong_type_profile.limits.is_none());
        assert_eq!(
            wrong_type_profile.description.as_deref(),
            Some("Still has a description though.")
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
                profile: None,
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
            _: Vec<crate::Message>,
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
