// Hardcoded LLM Model Profiles
//
// This module provides model profiles based on models.dev structure.
// Profiles are matched by provider_type + model_id.
//
// IMPORTANT: Never guess or extrapolate profile data (pricing, limits, capabilities).
// Always source from https://github.com/sst/models.dev/tree/dev/providers
// and cross-reference with official provider documentation. If a model is not
// yet listed on models.dev, wait until the data is available before adding it.
//
// NOTE: Currently only includes profiles for selected models.
// Additional model profiles can be added as needed by extending the match arms.
//
// Data source: https://github.com/sst/models.dev/tree/dev/providers
// Cross-referenced with official Anthropic and OpenAI documentation

use crate::driver_registry::ServiceKind;
use crate::model::{
    CostTier, DriverId, Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile,
    ModelVendor, ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue, Speed, SpeedConfig,
    SpeedValue, Verbosity, VerbosityConfig, VerbosityValue,
};

// Helper functions for creating reasoning effort configurations

fn effort(value: ReasoningEffort, name: &str) -> ReasoningEffortValue {
    ReasoningEffortValue {
        value,
        name: name.into(),
    }
}

/// Standard reasoning efforts for pre-gpt-5.1 reasoning models (o3, o4-mini)
/// Default: medium, supports: low, medium, high
fn reasoning_effort_standard() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Reasoning effort for pro-tier reasoning models (only high)
fn reasoning_effort_high_only() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![effort(ReasoningEffort::High, "High")],
        default: ReasoningEffort::High,
    }
}

/// Reasoning effort for pre-gpt-5.1 models (gpt-5, gpt-5-mini, gpt-5-nano, gpt-5-codex)
/// Default: medium, supports: low, medium, high (no none)
fn reasoning_effort_gpt5_pre51() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Reasoning effort for gpt-5.1 models
/// Default: none, supports: none, low, medium, high
fn reasoning_effort_gpt51() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::None, "None"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
        ],
        default: ReasoningEffort::None,
    }
}

/// Reasoning effort for models after gpt-5.1-codex-max (gpt-5.2, gpt-5.2-pro, gpt-5.2-codex)
/// Default: none, supports: none, low, medium, high, xhigh
fn reasoning_effort_gpt52() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::None, "None"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::None,
    }
}

/// Reasoning effort for gpt-5.5 and the gpt-5.6 series (Sol, Terra, Luna)
/// Default: medium, supports: none, low, medium, high, xhigh
fn reasoning_effort_gpt55() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::None, "None"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Reasoning effort for OpenAI Realtime voice sessions.
fn reasoning_effort_realtime() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Minimal, "Minimal"),
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::Low,
    }
}

/// Reasoning effort for gpt-5.2-pro
/// Default: medium, supports: medium, high, xhigh
fn reasoning_effort_gpt52_pro() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Extra High"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Extended thinking config for Anthropic Claude models
/// Maps to thinking budget_tokens: low=1024, medium=4096, high=16384, xhigh=32768
fn reasoning_effort_anthropic_extended_thinking() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low (1K tokens)"),
            effort(ReasoningEffort::Medium, "Medium (4K tokens)"),
            effort(ReasoningEffort::High, "High (16K tokens)"),
            effort(ReasoningEffort::Xhigh, "Extra High (32K tokens)"),
        ],
        default: ReasoningEffort::Medium,
    }
}

/// Adaptive thinking config for recent Claude reasoning models
/// (Fable 5, Opus 4.8, Opus 4.7, Opus 4.6, Sonnet 5, Sonnet 4.6)
/// Uses thinking.type="adaptive" with effort parameter instead of budget_tokens
/// Default: high, supports: low, medium, high, max (mapped to xhigh)
fn reasoning_effort_anthropic_adaptive_thinking() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::Low, "Low"),
            effort(ReasoningEffort::Medium, "Medium"),
            effort(ReasoningEffort::High, "High"),
            effort(ReasoningEffort::Xhigh, "Max"),
        ],
        default: ReasoningEffort::High,
    }
}

// Helper functions for creating speed (service tier) configurations.
//
// Availability is sourced from OpenAI's official tier tables: the API pricing
// page for Flex, the Priority processing page for first-party priority models,
// and the specialized pricing table for Codex priority. A model gets a speed
// config only when it has a Flex and/or Priority row. Chat-latest,
// deep-research, and unlisted variants have no speed config. Display names
// follow Codex's speed selector ("Fast" for priority).

fn speed(value: Speed, name: &str) -> SpeedValue {
    SpeedValue {
        value,
        name: name.into(),
    }
}

/// Speed for models with both flex and priority pricing rows
/// (gpt-5.4, gpt-5.4-mini, gpt-5.5, gpt-5.6 series).
fn speed_flex_priority() -> SpeedConfig {
    SpeedConfig {
        values: vec![
            speed(Speed::Flex, "Flex"),
            speed(Speed::Default, "Standard"),
            speed(Speed::Priority, "Fast"),
        ],
        default: Speed::Default,
    }
}

/// Speed for models with only a flex pricing row
/// (gpt-5.4-nano, gpt-5.4-pro, gpt-5.5-pro).
fn speed_flex_only() -> SpeedConfig {
    SpeedConfig {
        values: vec![
            speed(Speed::Flex, "Flex"),
            speed(Speed::Default, "Standard"),
        ],
        default: Speed::Default,
    }
}

/// Speed for models with only a priority pricing row
/// (gpt-4.1 family, gpt-5/gpt-5-mini,
/// gpt-5-codex, gpt-5.1/gpt-5.1-codex, gpt-5.2, gpt-5.3-codex,
/// o3, o4-mini).
fn speed_priority_only() -> SpeedConfig {
    SpeedConfig {
        values: vec![
            speed(Speed::Default, "Standard"),
            speed(Speed::Priority, "Fast"),
        ],
        default: Speed::Default,
    }
}

fn verbosity(value: Verbosity, name: &str) -> VerbosityValue {
    VerbosityValue {
        value,
        name: name.into(),
    }
}

/// Standard low/medium/high verbosity for OpenAI models that support the
/// `verbosity` request parameter (gpt-5.5, gpt-5.6 series). Medium is the
/// provider default.
fn verbosity_standard() -> VerbosityConfig {
    VerbosityConfig {
        values: vec![
            verbosity(Verbosity::Low, "Low"),
            verbosity(Verbosity::Medium, "Medium"),
            verbosity(Verbosity::High, "High"),
        ],
        default: Verbosity::Medium,
    }
}

/// Flat registry of known models. Lookup is provider-agnostic: a model is
/// resolved by id across this whole list, then filtered by the `surfaces`
/// predicate. `vendor` is a branding tag; the profile payload lives in
/// `profile_data`, keyed by the canonical id (`ids[0]`).
struct ModelDescriptor {
    /// Accepted wire ids. `ids[0]` is the canonical id used to fetch the
    /// profile payload; the rest are aliases (e.g. vendor-prefixed gateway
    /// ids). Matched case-insensitively, by exact match or `"<id>-"` prefix
    /// (which covers dated and `-latest` suffixes).
    ids: &'static [&'static str],
    vendor: ModelVendor,
    /// Provider types (API surfaces) this model is offered under.
    surfaces: &'static [DriverId],
    /// Which provider service this model belongs to (knowledge/foundations/providers.md).
    /// Pickers filter on it: chat pickers never list realtime models.
    service: ServiceKind,
}

const fn md(
    ids: &'static [&'static str],
    vendor: ModelVendor,
    surfaces: &'static [DriverId],
) -> ModelDescriptor {
    ModelDescriptor {
        ids,
        vendor,
        surfaces,
        service: ServiceKind::Chat,
    }
}

const fn md_service(
    ids: &'static [&'static str],
    vendor: ModelVendor,
    surfaces: &'static [DriverId],
    service: ServiceKind,
) -> ModelDescriptor {
    ModelDescriptor {
        ids,
        vendor,
        surfaces,
        service,
    }
}

// OpenAI's own models are served by the Responses API, Azure, and the generic
// Chat Completions path. Third-party OpenAI-compatible models (NVIDIA, Qwen,
// ...) are reachable via Responses-capable gateways (e.g. OpenRouter) and the
// Chat Completions path, but never Azure.
const OPENAI: &[DriverId] = &[
    DriverId::OpenAI,
    DriverId::OpenRouter,
    DriverId::AzureOpenAI,
    DriverId::OpenAICompletions,
];
const OPENAI_COMPAT: &[DriverId] = &[
    DriverId::OpenAI,
    DriverId::OpenRouter,
    DriverId::OpenAICompletions,
];
const ANTHROPIC: &[DriverId] = &[DriverId::Anthropic];
const GEMINI: &[DriverId] = &[DriverId::Gemini];
const LLMSIM: &[DriverId] = &[DriverId::LlmSim];
// Microsoft MAI models are served first-party by the dedicated `Mai` driver
// (Azure AI Foundry) and are also reachable through OpenAI-compatible gateways
// (e.g. OpenRouter). They are never offered through the Azure OpenAI driver,
// which targets OpenAI deployments rather than MAI deployments.
const MICROSOFT_MAI: &[DriverId] = &[
    DriverId::Mai,
    DriverId::OpenAI,
    DriverId::OpenRouter,
    DriverId::OpenAICompletions,
];
// Muse is served first-party by Meta Model API and through OpenAI-compatible
// gateways. The Contributor tier is first-party only because its data-use
// terms are part of Meta's own Model API product.
const META_MUSE: &[DriverId] = &[
    DriverId::Meta,
    DriverId::OpenAI,
    DriverId::OpenRouter,
    DriverId::OpenAICompletions,
];
const META_ONLY: &[DriverId] = &[DriverId::Meta];

static REGISTRY: &[ModelDescriptor] = &[
    // OpenAI
    md_service(
        &["text-embedding-3-small"],
        ModelVendor::OpenAi,
        OPENAI,
        ServiceKind::Embeddings,
    ),
    md_service(
        &["text-embedding-3-large"],
        ModelVendor::OpenAi,
        OPENAI,
        ServiceKind::Embeddings,
    ),
    md_service(
        &["gpt-realtime-2"],
        ModelVendor::OpenAi,
        OPENAI,
        ServiceKind::Realtime,
    ),
    md(&["o3"], ModelVendor::OpenAi, OPENAI),
    md(&["o3-pro"], ModelVendor::OpenAi, OPENAI),
    md(&["o3-deep-research"], ModelVendor::OpenAi, OPENAI),
    md(&["o4-mini"], ModelVendor::OpenAi, OPENAI),
    md(&["o4-mini-deep-research"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-4.1"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-4.1-mini"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-4.1-nano"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5-mini"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5-nano"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5-pro"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5-codex"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5-chat-latest"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.1"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.1-codex"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.1-codex-mini"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.1-codex-max"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.1-chat-latest"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.2"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.2-pro"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.2-codex"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.2-chat-latest"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.3-codex"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.4"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.4-mini"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.4-nano"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.4-pro"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.5"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.5-pro"], ModelVendor::OpenAi, OPENAI),
    // GPT-5.6 series: Sol (flagship), Terra (balanced), Luna (fast/cheap).
    md(&["gpt-5.6-sol"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.6-terra"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-5.6-luna"], ModelVendor::OpenAi, OPENAI),
    // Anthropic
    md(&["claude-fable-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-8"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-7"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-6"], ModelVendor::Anthropic, ANTHROPIC),
    // 1M-context twins. The gateway exposes these `[1m]` ids alongside the
    // 200K base models (e.g. "Opus 4.8" vs "Opus 4.8 (1M)" in the picker); the
    // driver sends the `context-1m` beta header for them. See
    // `anthropic_1m_variant` for how their profiles are derived.
    md(&["claude-fable-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-8[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-7[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-6[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-4-6"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-4-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-haiku-4-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4"], ModelVendor::Anthropic, ANTHROPIC),
    // Google Gemini
    md(&["gemini-3.1-pro-preview"], ModelVendor::Google, GEMINI),
    md(&["gemini-3.5-flash"], ModelVendor::Google, GEMINI),
    md(&["gemini-3.1-flash-lite"], ModelVendor::Google, GEMINI),
    md(&["gemini-2.5-pro"], ModelVendor::Google, GEMINI),
    md(&["gemini-2.5-flash"], ModelVendor::Google, GEMINI),
    md(&["gemini-2.0-flash"], ModelVendor::Google, GEMINI),
    // Third-party, OpenAI-compatible
    md(
        &[
            "nemotron-3-super-120b-a12b",
            "nvidia/nemotron-3-super-120b-a12b",
        ],
        ModelVendor::Nvidia,
        OPENAI_COMPAT,
    ),
    md(
        &["qwen3.7-max", "qwen/qwen3.7-max"],
        ModelVendor::Qwen,
        OPENAI_COMPAT,
    ),
    md(
        &["mai-1-preview", "microsoft/mai-1-preview"],
        ModelVendor::Microsoft,
        MICROSOFT_MAI,
    ),
    md(
        &["mai-code-1-flash", "microsoft/mai-code-1-flash"],
        ModelVendor::Microsoft,
        MICROSOFT_MAI,
    ),
    md(
        &["muse-spark-1.2", "meta/muse-spark-1.2"],
        ModelVendor::Meta,
        META_MUSE,
    ),
    md(
        &["muse-spark-1.2-contributor"],
        ModelVendor::Meta,
        META_ONLY,
    ),
    md(
        &["minimax-m3", "minimax/minimax-m3"],
        ModelVendor::MiniMax,
        OPENAI_COMPAT,
    ),
    md(
        &["kimi-k2-thinking", "moonshotai/kimi-k2-thinking"],
        ModelVendor::Moonshot,
        OPENAI_COMPAT,
    ),
    md(
        &["kimi-k3", "moonshotai/kimi-k3"],
        ModelVendor::Moonshot,
        OPENAI_COMPAT,
    ),
    md(
        &["grok-4.3", "x-ai/grok-4.3", "xai/grok-4.3"],
        ModelVendor::XAi,
        OPENAI_COMPAT,
    ),
    // Test simulator
    md(&["llmsim-default", "llmsim"], ModelVendor::LlmSim, LLMSIM),
];

/// Resolve the registry descriptor for a model id under a provider type.
/// Matching is provider-filtered (the `surfaces` predicate) and picks the
/// longest matching id so specific variants win over their prefixes (e.g.
/// `gpt-5.4-mini` over `gpt-5.4`).
fn resolve_descriptor(
    provider_type: &DriverId,
    model_id: &str,
) -> Option<&'static ModelDescriptor> {
    // Match without allocating: compare bytes case-insensitively. Ids are ASCII.
    let id = model_id.as_bytes();
    let mut longest_match = 0;
    let mut best_for_surface: Option<&'static ModelDescriptor> = None;
    for descriptor in REGISTRY {
        for alias in descriptor.ids {
            let alias = alias.as_bytes();
            // Exact (case-insensitive) match, or a recognized version suffix.
            // Do not treat semantic variants such as `o3-mini` as versions of
            // a shorter registered model.
            let id_matches = if id.len() == alias.len() {
                id.eq_ignore_ascii_case(alias)
            } else {
                id.len() > alias.len()
                    && id[alias.len()] == b'-'
                    && id[..alias.len()].eq_ignore_ascii_case(alias)
                    && is_version_suffix(&id[alias.len() + 1..])
            };
            if !id_matches {
                continue;
            }

            // Resolve the most specific known model identity before checking
            // its provider surface. Otherwise a shorter generic prefix can
            // swallow an exact tier/variant that is intentionally unavailable
            // on this provider (for example Muse Spark Contributor via a
            // gateway) and silently assign the wrong profile.
            if alias.len() > longest_match {
                longest_match = alias.len();
                best_for_surface = descriptor
                    .surfaces
                    .contains(provider_type)
                    .then_some(descriptor);
            } else if alias.len() == longest_match && descriptor.surfaces.contains(provider_type) {
                best_for_surface = Some(descriptor);
            }
        }
    }
    best_for_surface
}

fn is_version_suffix(suffix: &[u8]) -> bool {
    suffix.eq_ignore_ascii_case(b"latest")
        || (suffix.len() == 8 && suffix.iter().all(u8::is_ascii_digit))
        || (suffix.len() == 5
            && suffix[2] == b'-'
            && suffix[..2].iter().all(u8::is_ascii_digit)
            && suffix[3..].iter().all(u8::is_ascii_digit))
        || (suffix.len() == 10
            && suffix[4] == b'-'
            && suffix[7] == b'-'
            && suffix
                .iter()
                .enumerate()
                .all(|(index, byte)| matches!(index, 4 | 7) || byte.is_ascii_digit()))
        || (suffix.len() == 13
            && suffix[..8].eq_ignore_ascii_case(b"preview-")
            && suffix[10] == b'-'
            && suffix[8..10].iter().all(u8::is_ascii_digit)
            && suffix[11..].iter().all(u8::is_ascii_digit))
}

/// Get a model profile by matching provider_type and model_id.
/// Returns None if the id is not in the registry or is not offered under the
/// given provider type.
pub fn get_model_profile(provider_type: &DriverId, model_id: &str) -> Option<ModelProfile> {
    let descriptor = resolve_descriptor(provider_type, model_id)?;
    let mut profile = profile_data(descriptor.ids[0])?;
    // Native execution phases are implemented by the first-party OpenAI and
    // Meta Responses surfaces. Gateways keep the base model profile while
    // masking provider-native request options.
    if provider_type != &DriverId::OpenAI && provider_type != &DriverId::Meta {
        profile.supports_phases = false;
    }
    // Hosted tool_search is rendered by the OpenAI Responses driver and the
    // Anthropic Messages driver. Other provider types reach the same models
    // through transports that don't implement the hosted format and must fall
    // back to client-side `tool_search` (see `auto_tool_search`):
    //   - OpenRouter: stateless `/responses` shim, no tool_search extension.
    //   - Bedrock: ConverseStream; Anthropic's server-side tool search there is
    //     only on the InvokeModel API, which this driver does not use.
    //   - OpenAI Completions / Gemini: no hosted tool_search at all.
    // So mask the flag except on the first-party surfaces that implement it.
    if provider_type != &DriverId::OpenAI
        && provider_type != &DriverId::Anthropic
        && provider_type != &DriverId::Meta
    {
        profile.tool_search = false;
    }
    // Speed (service tier) is an OpenAI-platform billing feature. Azure has
    // its own capacity model and gateways (OpenRouter) do their own routing,
    // so only the first-party OpenAI surface keeps the selector.
    if provider_type != &DriverId::OpenAI {
        profile.speed = None;
    }
    // Verbosity (`text.verbosity` / `verbosity`) is an OpenAI-specific request
    // parameter. Gateways that proxy these models may reject the unknown field,
    // so only the first-party OpenAI surface keeps the selector.
    if provider_type != &DriverId::OpenAI {
        profile.verbosity = None;
    }
    Some(profile)
}

/// Estimate the USD cost of a generation from the model's static price-table
/// profile, discounting cached-read tokens at the model's `cache_read` rate.
/// Returns `None` when there is no profile or no cost data for the model —
/// callers then have no estimate to record and fall back accordingly.
///
/// This is the price-table fallback used when a provider does not report an
/// authoritative cost inline (e.g. OpenRouter's `usage.cost`). Cost figures in
/// profiles are per million tokens.
///
/// Token buckets are disjoint by convention (drivers normalize at the boundary;
/// see the `TokenUsage` event): `input_tokens` is non-cached
/// input only, with `cache_read_tokens` / `cache_creation_tokens` additive on
/// top. Cost is therefore uniform across providers — each bucket is billed at
/// its own rate (cache-creation tokens have no dedicated price and bill at the
/// input rate) with no provider-specific compensation.
pub fn estimate_cost_usd(
    provider_type: &DriverId,
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_creation_tokens: u32,
) -> Option<f64> {
    let cost = get_model_profile(provider_type, model_id)?.cost?;
    let prompt_tokens = input_tokens
        .saturating_add(cache_read_tokens)
        .saturating_add(cache_creation_tokens);
    let active_tier = cost
        .cost_tiers
        .iter()
        .filter(|tier| prompt_tokens > tier.above_tokens.max(0) as u32)
        .max_by_key(|tier| tier.above_tokens);

    let input_rate = active_tier.map_or(cost.input, |tier| tier.input);
    let output_rate = active_tier.map_or(cost.output, |tier| tier.output);
    let cache_read_rate = active_tier
        .and_then(|tier| tier.cache_read)
        .or(cost.cache_read)
        .unwrap_or(input_rate);
    let per_million = |tokens: u32, rate: f64| (tokens as f64 / 1_000_000.0) * rate;

    Some(
        per_million(input_tokens, input_rate)
            + per_million(cache_read_tokens, cache_read_rate)
            + per_million(cache_creation_tokens, input_rate)
            + per_million(output_tokens, output_rate),
    )
}

/// Get the vendor/brand for a model id, or None if it is not in the registry
/// (or not offered under the given provider type).
pub fn get_model_vendor(provider_type: &DriverId, model_id: &str) -> Option<ModelVendor> {
    resolve_descriptor(provider_type, model_id).map(|descriptor| descriptor.vendor)
}

/// Stable public profile key: `"{vendor}/{canonical_id}"` (knowledge/foundations/providers.md).
///
/// The key identifies the model's identity independent of which provider
/// serves it: `("anthropic", "claude-sonnet-4-5-20250929")` and a gateway
/// alias of the same model both map to `"anthropic/claude-sonnet-4-5"`.
pub fn get_model_profile_key(provider_type: &DriverId, model_id: &str) -> Option<String> {
    resolve_descriptor(provider_type, model_id)
        .map(|descriptor| format!("{}/{}", descriptor.vendor.slug(), descriptor.ids[0]))
}

/// Look up a profile by its stable key (`"{vendor}/{canonical_id}"`).
///
/// Key lookup is provider-independent, so the returned profile is the base
/// payload without provider-surface masking (`supports_phases`/`tool_search`
/// stay as authored). Use [`get_model_profile`] when resolving for a concrete
/// provider.
pub fn get_model_profile_by_key(key: &str) -> Option<ModelProfile> {
    let (vendor_slug, canonical) = key.split_once('/')?;
    let descriptor = REGISTRY.iter().find(|descriptor| {
        descriptor.vendor.slug().eq_ignore_ascii_case(vendor_slug)
            && descriptor.ids[0].eq_ignore_ascii_case(canonical)
    })?;
    profile_data(descriptor.ids[0])
}

/// Which provider service a model belongs to. Unknown models default to
/// [`ServiceKind::Chat`].
pub fn get_model_service_kind(provider_type: &DriverId, model_id: &str) -> ServiceKind {
    resolve_descriptor(provider_type, model_id)
        .map(|descriptor| descriptor.service)
        .unwrap_or(ServiceKind::Chat)
}

/// Profile payload keyed by canonical model id. Pure value store: provider
/// availability and vendor tagging live in `REGISTRY`, not here. The segments
/// are grouped by author for readability only — there is no provider dispatch.
fn profile_data(canonical: &str) -> Option<ModelProfile> {
    openai_profile_data(canonical)
        .or_else(|| anthropic_profile_data(canonical))
        .or_else(|| gemini_profile_data(canonical))
        .or_else(|| meta_profile_data(canonical))
        .or_else(|| third_party_profile_data(canonical))
        .or_else(|| llmsim_profile_data(canonical))
}

fn meta_profile_data(model_id: &str) -> Option<ModelProfile> {
    let (name, description, cost) = match model_id {
        "muse-spark-1.2" => (
            "Muse Spark 1.2",
            "Meta's coding-optimized Muse Spark model. Prompts and completions are not used to train Meta models.",
            ModelCost {
                input: 1.25,
                output: 4.25,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            },
        ),
        "muse-spark-1.2-contributor" => (
            "Muse Spark 1.2 Contributor",
            "Discounted Muse Spark 1.2 tier where prompts and completions may be used to train future Meta models.",
            ModelCost {
                input: 0.10,
                output: 0.20,
                cache_read: Some(0.002),
                cost_tiers: vec![],
            },
        ),
        _ => return None,
    };

    Some(ModelProfile {
        name: name.into(),
        family: "muse-spark-1.2".into(),
        description: Some(description.into()),
        release_date: Some("2026-08-05".into()),
        last_updated: Some("2026-08-05".into()),
        attachment: true,
        reasoning: true,
        temperature: true,
        knowledge: None,
        tool_call: true,
        structured_output: true,
        open_weights: false,
        cost: Some(cost),
        limits: Some(ModelLimits {
            // Meta documents one joint input + output context budget and no
            // smaller fixed output cap. Callers must leave room for input.
            context: 1_048_576,
            input: None,
            output: 1_048_576,
            max_media: None,
        }),
        modalities: Some(ModelModalities {
            input: vec![
                Modality::Text,
                Modality::Image,
                Modality::Audio,
                Modality::Video,
                Modality::Pdf,
            ],
            output: vec![Modality::Text],
        }),
        // Meta documents a model-determined default rather than a stable
        // effort value, which the current profile type cannot represent.
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        tool_search: true,
        supported_parameters: Vec::new(),
        supports_phases: true,
    })
}

fn openai_embedding_profile(name: &str, family: &str, input_cost: f64) -> ModelProfile {
    ModelProfile {
        name: name.into(),
        family: family.into(),
        description: None,
        release_date: Some("2024-01-25".into()),
        last_updated: Some("2024-01-25".into()),
        attachment: false,
        reasoning: false,
        temperature: false,
        knowledge: None,
        tool_call: false,
        structured_output: false,
        open_weights: false,
        cost: Some(ModelCost {
            input: input_cost,
            output: 0.0,
            cache_read: None,
            cost_tiers: vec![],
        }),
        limits: Some(ModelLimits {
            context: 8_191,
            input: None,
            output: 0,
            max_media: None,
        }),
        modalities: Some(ModelModalities {
            input: vec![Modality::Text],
            output: vec![],
        }),
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        tool_search: false,
        supported_parameters: Vec::new(),
        supports_phases: false,
    }
}

fn openai_profile_data(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        "text-embedding-3-small" => Some(openai_embedding_profile(
            "Text Embedding 3 Small",
            "text-embedding-3-small",
            0.02,
        )),
        "text-embedding-3-large" => Some(openai_embedding_profile(
            "Text Embedding 3 Large",
            "text-embedding-3-large",
            0.13,
        )),
        "gpt-realtime-2" => Some(ModelProfile {
            name: "GPT Realtime 2".into(),
            family: "gpt-realtime".into(),
            description: Some("OpenAI Realtime model for low-latency voice sessions".into()),
            release_date: None,
            last_updated: None,
            attachment: false,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: None,
            limits: None,
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Audio],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: Some(reasoning_effort_realtime()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "o3" => Some(ModelProfile {
            name: "o3".into(),
            family: "o3".into(),
            description: None,
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o3-pro" => Some(ModelProfile {
            name: "o3 Pro".into(),
            family: "o3-pro".into(),
            description: None,
            release_date: Some("2025-06-10".into()),
            last_updated: Some("2025-06-10".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 20.00,
                output: 80.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o4-mini" => Some(ModelProfile {
            name: "o4 mini".into(),
            family: "o4-mini".into(),
            description: None,
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-4.1 family models
        "gpt-4.1" => Some(ModelProfile {
            name: "GPT-4.1".into(),
            family: "gpt-4.1".into(),
            description: None,
            release_date: Some("2025-04-14".into()),
            last_updated: Some("2025-04-14".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-4.1-mini" => Some(ModelProfile {
            name: "GPT-4.1 mini".into(),
            family: "gpt-4.1-mini".into(),
            description: None,
            release_date: Some("2025-04-14".into()),
            last_updated: Some("2025-04-14".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.40,
                output: 1.60,
                cache_read: Some(0.20),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-4.1-nano" => Some(ModelProfile {
            name: "GPT-4.1 nano".into(),
            family: "gpt-4.1-nano".into(),
            description: None,
            release_date: Some("2025-04-14".into()),
            last_updated: Some("2025-04-14".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.10,
                output: 0.40,
                cache_read: Some(0.05),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5 family models
        // Pre-5.1 models: default medium, supports low/medium/high (no none)
        "gpt-5" => Some(ModelProfile {
            name: "GPT-5".into(),
            family: "gpt-5".into(),
            description: None,
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-mini" => Some(ModelProfile {
            name: "GPT-5 mini".into(),
            family: "gpt-5-mini".into(),
            description: None,
            release_date: Some("2025-08-13".into()),
            last_updated: Some("2025-08-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.25,
                output: 2.00,
                cache_read: Some(0.025),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-nano" => Some(ModelProfile {
            name: "GPT-5 nano".into(),
            family: "gpt-5-nano".into(),
            description: None,
            release_date: Some("2025-08-13".into()),
            last_updated: Some("2025-08-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-05-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.05,
                output: 0.40,
                cache_read: Some(0.005),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-pro" => Some(ModelProfile {
            name: "GPT-5 Pro".into(),
            family: "gpt-5-pro".into(),
            description: None,
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 15.00,
                output: 60.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-codex" => Some(ModelProfile {
            name: "GPT-5 Codex".into(),
            family: "gpt-5-codex".into(),
            description: None,
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.1 models: default none, supports none/low/medium/high
        "gpt-5.1" => Some(ModelProfile {
            name: "GPT-5.1".into(),
            family: "gpt-5.1".into(),
            description: None,
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.1-codex" => Some(ModelProfile {
            name: "GPT-5.1 Codex".into(),
            family: "gpt-5.1-codex".into(),
            description: None,
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.1-codex-mini" => Some(ModelProfile {
            name: "GPT-5.1 Codex mini".into(),
            family: "gpt-5.1-codex-mini".into(),
            description: None,
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.30,
                output: 2.40,
                cache_read: Some(0.03),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.1-codex-max and after: supports xhigh
        "gpt-5.1-codex-max" => Some(ModelProfile {
            name: "GPT-5.1 Codex max".into(),
            family: "gpt-5.1-codex-max".into(),
            description: None,
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 3.00,
                output: 24.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.2 models: supports xhigh, 400K context
        "gpt-5.2" => Some(ModelProfile {
            name: "GPT-5.2".into(),
            family: "gpt-5.2".into(),
            description: None,
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.2-pro" => Some(ModelProfile {
            name: "GPT-5.2 Pro".into(),
            family: "gpt-5.2-pro".into(),
            description: None,
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 21.00,
                output: 168.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.2-codex" => Some(ModelProfile {
            name: "GPT-5.2 Codex".into(),
            family: "gpt-5.2-codex".into(),
            description: None,
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.3 Codex: same pricing as 5.2, 25% faster inference
        "gpt-5.3-codex" => Some(ModelProfile {
            name: "GPT-5.3 Codex".into(),
            family: "gpt-5.3-codex".into(),
            description: None,
            release_date: Some("2026-02-05".into()),
            last_updated: Some("2026-02-05".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: Some(speed_priority_only()),
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.6 series: current flagship family, publicly released 2026-07-09.
        // The version (5.6) names the generation; Sol/Terra/Luna are durable
        // capability tiers (intelligence / balanced / fast-and-cheap) that can
        // advance on their own cadence. All three share a 1.05M context, 128K
        // output, a 2026-02-16 knowledge cutoff, and none/low/medium/high/xhigh
        // reasoning effort (default medium). Pricing is tiered: prompts over
        // 272K input tokens bill at 2x input and 1.5x output for the whole
        // request. Source: developers.openai.com/api/docs/models/gpt-5.6-{sol,
        // terra,luna} and openai.com/index/gpt-5-6/ (models.dev did not yet list
        // these variants at the time of addition; refresh once it catches up).
        "gpt-5.6-sol" => Some(ModelProfile {
            name: "GPT-5.6 Sol".into(),
            family: "gpt-5.6-sol".into(),
            description: Some("Flagship model of the GPT-5.6 series. Deepest reasoning for complex agentic coding, science, and multi-step analysis.".into()),
            release_date: Some("2026-07-09".into()),
            last_updated: Some("2026-07-09".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2026-02-16".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 5.00,
                output: 30.00,
                cache_read: Some(0.50),
                cost_tiers: vec![CostTier {
                    above_tokens: 272_000,
                    input: 10.00,
                    output: 45.00,
                    cache_read: Some(1.00),
                }],
            }),
            limits: Some(ModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt55()),
            speed: Some(speed_flex_priority()),
            verbosity: Some(verbosity_standard()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.6-terra" => Some(ModelProfile {
            name: "GPT-5.6 Terra".into(),
            family: "gpt-5.6-terra".into(),
            description: Some("Balanced GPT-5.6 tier for everyday work. Performance competitive with GPT-5.5 at roughly half the cost.".into()),
            release_date: Some("2026-07-09".into()),
            last_updated: Some("2026-07-09".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2026-02-16".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.50,
                output: 15.00,
                cache_read: Some(0.25),
                cost_tiers: vec![CostTier {
                    above_tokens: 272_000,
                    input: 5.00,
                    output: 22.50,
                    cache_read: Some(0.50),
                }],
            }),
            limits: Some(ModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt55()),
            speed: Some(speed_flex_priority()),
            verbosity: Some(verbosity_standard()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.6-luna" => Some(ModelProfile {
            name: "GPT-5.6 Luna".into(),
            family: "gpt-5.6-luna".into(),
            description: Some("Fastest, most cost-efficient GPT-5.6 tier. Built for high-volume, latency-sensitive work: classification, extraction, routing, and first-pass drafting.".into()),
            release_date: Some("2026-07-09".into()),
            last_updated: Some("2026-07-09".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2026-02-16".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.00,
                output: 6.00,
                cache_read: Some(0.10),
                cost_tiers: vec![CostTier {
                    above_tokens: 272_000,
                    input: 2.00,
                    output: 9.00,
                    cache_read: Some(0.20),
                }],
            }),
            limits: Some(ModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt55()),
            speed: Some(speed_flex_priority()),
            verbosity: Some(verbosity_standard()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        // GPT-5.5 family: flagship reasoning models. Released 2026-04-23.
        // Flat pricing (no 200K context tiers, unlike 5.4).
        // Source: developers.openai.com/api/docs/models/gpt-5.5 and .../gpt-5.5-pro
        // (models.dev did not yet list these variants at the time of addition;
        // refresh once the models.dev entry appears).
        "gpt-5.5" => Some(ModelProfile {
            name: "GPT-5.5".into(),
            family: "gpt-5.5".into(),
            description: Some("Flagship reasoning model. Best for complex multi-step tasks, code generation, and deep analysis.".into()),
            release_date: Some("2026-04-23".into()),
            last_updated: Some("2026-04-23".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-12-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 5.00,
                output: 30.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt55()),
            speed: Some(speed_flex_priority()),
            verbosity: Some(verbosity_standard()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.5-pro" => Some(ModelProfile {
            name: "GPT-5.5 Pro".into(),
            family: "gpt-5.5-pro".into(),
            description: Some("Extended-thinking variant for the hardest problems. Trades speed for deeper reasoning on math, science, and complex code.".into()),
            release_date: Some("2026-04-23".into()),
            last_updated: Some("2026-04-23".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-12-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 30.00,
                output: 180.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            speed: Some(speed_flex_only()),
            verbosity: None,
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        // GPT-5.4 family: reasoning models with 1.05M context, tool_search, native phases.
        // Released 2026-03-05 (5.4, 5.4-pro), 2026-03-17 (5.4-mini, 5.4-nano).
        // 5.4 and 5.4-pro have tiered pricing above 200K context tokens.
        "gpt-5.4" => Some(ModelProfile {
            name: "GPT-5.4".into(),
            family: "gpt-5.4".into(),
            description: Some("Flagship reasoning model with 1M+ context. Best for complex multi-step tasks, code generation, and deep analysis.".into()),
            release_date: Some("2026-03-05".into()),
            last_updated: Some("2026-03-05".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.50,
                output: 15.00,
                cache_read: Some(0.25),
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 5.00,
                    output: 22.50,
                    cache_read: Some(0.50),
                }],
            }),
            limits: Some(ModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: Some(speed_flex_priority()),
            verbosity: None,
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.4-mini" => Some(ModelProfile {
            name: "GPT-5.4 mini".into(),
            family: "gpt-5.4-mini".into(),
            description: Some("Fast, cost-effective reasoning model. Balances strong performance with low latency for everyday tasks.".into()),
            release_date: Some("2026-03-17".into()),
            last_updated: Some("2026-03-17".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.75,
                output: 4.50,
                cache_read: Some(0.075),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: Some(speed_flex_priority()),
            verbosity: None,
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.4-nano" => Some(ModelProfile {
            name: "GPT-5.4 nano".into(),
            family: "gpt-5.4-nano".into(),
            description: Some("Smallest and cheapest GPT-5.4 variant. Ideal for high-volume, latency-sensitive workloads.".into()),
            release_date: Some("2026-03-17".into()),
            last_updated: Some("2026-03-17".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.20,
                output: 1.25,
                cache_read: Some(0.02),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: Some(speed_flex_only()),
            verbosity: None,
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.4-pro" => Some(ModelProfile {
            name: "GPT-5.4 Pro".into(),
            family: "gpt-5.4-pro".into(),
            description: Some("Extended-thinking variant for the hardest problems. Trades speed for deeper reasoning on math, science, and complex code.".into()),
            release_date: Some("2026-03-05".into()),
            last_updated: Some("2026-03-05".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: Some(ModelCost {
                input: 30.00,
                output: 180.00,
                cache_read: None,
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 60.00,
                    output: 270.00,
                    cache_read: None,
                }],
            }),
            limits: Some(ModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            speed: Some(speed_flex_only()),
            verbosity: None,
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        // GPT-5 chat-latest models (point to latest chat-optimized versions)
        "gpt-5-chat-latest" => Some(ModelProfile {
            name: "GPT-5 Chat".into(),
            family: "gpt-5".into(),
            description: None,
            release_date: Some("2025-08-07".into()),
            last_updated: Some("2025-08-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.1-chat-latest" => Some(ModelProfile {
            name: "GPT-5.1 Chat".into(),
            family: "gpt-5.1".into(),
            description: None,
            release_date: Some("2025-11-13".into()),
            last_updated: Some("2025-11-13".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.2-chat-latest" => Some(ModelProfile {
            name: "GPT-5.2 Chat".into(),
            family: "gpt-5.2".into(),
            description: None,
            release_date: Some("2025-12-11".into()),
            last_updated: Some("2025-12-11".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Deep research models
        "o3-deep-research" => Some(ModelProfile {
            name: "o3 Deep Research".into(),
            family: "o3".into(),
            description: None,
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o4-mini-deep-research" => Some(ModelProfile {
            name: "o4 mini Deep Research".into(),
            family: "o4-mini".into(),
            description: None,
            release_date: Some("2025-04-16".into()),
            last_updated: Some("2025-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-06-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        _ => None,
    }
}

/// Profile payloads for non-OpenAI models exposed through OpenAI-compatible
/// APIs (NVIDIA NIM, Alibaba, MiniMax, Moonshot, xAI, Microsoft). Pure value
/// store keyed by canonical id; which provider surfaces each model is offered
/// under is decided by `REGISTRY`, not here. Sourced from models.dev unless
/// noted otherwise.
///
/// The id is lowercased so the cased canonical ids and aliases below resolve
/// regardless of how the caller passed them.
///
/// `structured_output` is set to `false` where the upstream models.dev entry
/// does not assert it: absence of the field is not a claim of support, so we
/// do not advertise a capability we cannot confirm.
fn third_party_profile_data(model_id: &str) -> Option<ModelProfile> {
    match model_id.to_ascii_lowercase().as_str() {
        // NVIDIA Nemotron 3 Super — flagship Nemotron reasoning model.
        // Source: models.dev (nvidia provider).
        "nemotron-3-super-120b-a12b" | "nvidia/nemotron-3-super-120b-a12b" => Some(ModelProfile {
            name: "Nemotron 3 Super".into(),
            family: "nemotron-3-super".into(),
            description: None,
            release_date: Some("2026-03-11".into()),
            last_updated: Some("2026-03-11".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-04-01".into()),
            tool_call: true,
            structured_output: false,
            open_weights: true,
            cost: Some(ModelCost {
                input: 0.20,
                output: 0.80,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 262_144,
                input: None,
                output: 262_144,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Alibaba Qwen3.7 Max — flagship Qwen model.
        // Source: models.dev (alibaba provider). Knowledge cutoff not published.
        "qwen3.7-max" | "qwen/qwen3.7-max" => Some(ModelProfile {
            name: "Qwen3.7 Max".into(),
            family: "qwen3.7-max".into(),
            description: None,
            release_date: Some("2026-05-21".into()),
            last_updated: Some("2026-05-21".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.50,
                output: 7.50,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_000_000,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Microsoft MAI-1-preview — Microsoft's first end-to-end in-house
        // foundation model. Not on models.dev; sourced from Microsoft's official
        // announcement (microsoft.ai/news/two-new-in-house-models). It is a
        // text-only instruction model (not a reasoning model); context window,
        // pricing, and knowledge cutoff were never publicly disclosed, so cost
        // and limits are left unset rather than guessed.
        "mai-1-preview" | "microsoft/mai-1-preview" => Some(ModelProfile {
            name: "MAI-1-preview".into(),
            family: "mai-1-preview".into(),
            description: None,
            release_date: Some("2025-08-28".into()),
            last_updated: None,
            attachment: false,
            reasoning: false,
            temperature: true,
            knowledge: None,
            tool_call: false,
            structured_output: false,
            open_weights: false,
            cost: None,
            limits: None,
            modalities: Some(ModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Microsoft MAI-Code-1-Flash — Microsoft's in-house, latency-optimized
        // coding model served via Azure AI Foundry (and OpenAI-compatible
        // gateways). It is a fast, tool-calling code model rather than a graded
        // reasoning model. Microsoft has not published pricing, context window,
        // or knowledge cutoff, so cost/limits are left unset rather than guessed
        // (same policy as MAI-1-preview).
        "mai-code-1-flash" | "microsoft/mai-code-1-flash" => Some(ModelProfile {
            name: "MAI-Code-1-Flash".into(),
            family: "mai-code-1".into(),
            description: Some(
                "Microsoft's latency-optimized in-house coding model (Azure AI Foundry).".into(),
            ),
            release_date: None,
            last_updated: None,
            attachment: false,
            reasoning: false,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: None,
            limits: None,
            modalities: Some(ModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // MiniMax-M3 — flagship MiniMax model. Source: models.dev (minimax
        // provider). Reasoning is a toggle upstream (no graded effort).
        "minimax-m3" | "minimax/minimax-m3" => Some(ModelProfile {
            name: "MiniMax-M3".into(),
            family: "minimax-m3".into(),
            description: None,
            release_date: Some("2026-06-01".into()),
            last_updated: Some("2026-06-01".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: true,
            cost: Some(ModelCost {
                input: 0.60,
                output: 2.40,
                cache_read: Some(0.12),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 512_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Video],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Moonshot Kimi K2 Thinking — flagship Kimi reasoning model.
        // Source: models.dev (moonshotai provider).
        "kimi-k2-thinking" | "moonshotai/kimi-k2-thinking" => Some(ModelProfile {
            name: "Kimi K2 Thinking".into(),
            family: "kimi-k2-thinking".into(),
            description: None,
            release_date: Some("2025-11-06".into()),
            last_updated: Some("2025-11-06".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2024-08-01".into()),
            tool_call: true,
            structured_output: false,
            open_weights: true,
            cost: Some(ModelCost {
                input: 0.60,
                output: 2.50,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 262_144,
                input: None,
                output: 262_144,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Moonshot Kimi K3 — flagship multimodal Kimi model with a 1M-token
        // context window. Source: models.dev (moonshotai provider). models.dev
        // lists graded reasoning effort (low/high/max), but like the other
        // OpenAI-compatible third-party models here we don't wire a graded
        // effort selector; `reasoning: true` still gates reasoning support.
        // Knowledge cutoff not published.
        "kimi-k3" | "moonshotai/kimi-k3" => Some(ModelProfile {
            name: "Kimi K3".into(),
            family: "kimi-k3".into(),
            description: None,
            release_date: Some("2026-07-16".into()),
            last_updated: Some("2026-07-16".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: true,
            cost: Some(ModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_048_576,
                input: None,
                output: 131_072,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Video],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // xAI Grok 4.3 — flagship Grok model. Source: models.dev (xai provider).
        // Pricing has a >200K-token tier. Knowledge cutoff not published.
        "grok-4.3" | "x-ai/grok-4.3" | "xai/grok-4.3" => Some(ModelProfile {
            name: "Grok 4.3".into(),
            family: "grok-4.3".into(),
            description: None,
            release_date: Some("2026-04-17".into()),
            last_updated: Some("2026-04-17".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.25,
                output: 2.50,
                cache_read: Some(0.20),
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 2.50,
                    output: 5.00,
                    cache_read: Some(0.40),
                }],
            }),
            limits: Some(ModelLimits {
                context: 1_000_000,
                input: None,
                output: 30_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        _ => None,
    }
}

/// Derive the 1M-context variant of a base Anthropic profile.
///
/// The gateway exposes `[1m]` model ids (e.g. `claude-opus-4-8[1m]`) as
/// large-context twins of the 200K base profiles. Anthropic serves the full 1M
/// window at standard per-token rates — there is no long-context premium — so
/// the variant keeps the base profile's cost verbatim (no `cost_tiers`) and
/// only raises the context limit. The display name gains a "(1M)" suffix to
/// disambiguate the two entries in the model picker; `family` is left unchanged
/// so the pair groups together. The Anthropic driver additionally sends the
/// `context-1m` beta header for these ids.
fn anthropic_1m_variant(mut profile: ModelProfile) -> ModelProfile {
    if let Some(limits) = profile.limits.as_mut() {
        limits.context = 1_000_000;
    }
    profile.name = format!("{} (1M)", profile.name);
    profile
}

/// Whether a Claude model `family` supports Anthropic's hosted tool_search
/// (the `tool_search_tool_*_20251119` server tools). Per docs.claude.com, this
/// is Sonnet 4.0+, Opus 4.0+, Haiku 4.5+, and Fable 5 — the 3.x families do not
/// support it. Centralized here (rather than per-literal) because the rule is a
/// clean family cutoff; contrast the OpenAI profiles, which set `tool_search`
/// per model literal.
///
/// Only families with a corresponding profile in `anthropic_profile_data_inner`
/// belong here — Anthropic docs also list Mythos 5, but this registry has no
/// `claude-mythos-5` descriptor, so including it would be a dead branch. Add the
/// family here when (and if) its profile lands.
fn anthropic_family_supports_tool_search(family: &str) -> bool {
    matches!(
        family,
        "claude-fable-5"
            | "claude-opus-5"
            | "claude-opus-4-8"
            | "claude-opus-4-7"
            | "claude-opus-4-6"
            | "claude-opus-4-5"
            | "claude-opus-4"
            | "claude-sonnet-5"
            | "claude-sonnet-4-6"
            | "claude-sonnet-4-5"
            | "claude-haiku-4-5"
    )
}

fn anthropic_profile_data(model_id: &str) -> Option<ModelProfile> {
    // `tool_search` is assigned centrally by family below, so the per-literal
    // `tool_search` value in the match arms is a placeholder and is overwritten.
    anthropic_profile_data_inner(model_id).map(|mut profile| {
        profile.tool_search = anthropic_family_supports_tool_search(&profile.family);
        profile
    })
}

fn anthropic_profile_data_inner(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        // Claude Fable 5 (newest — top tier above Opus)
        // Source: Anthropic model card (claude-api skill `shared/models.md`) and
        // docs.claude.com — Fable 5 is not yet in models.dev. Same API surface as
        // Opus 4.8: adaptive thinking only, sampling parameters removed (temperature
        // returns 400, hence `temperature: false`). One extra restriction vs Opus
        // 4.8: an explicit `thinking: {type: "disabled"}` also returns 400 — the
        // param must be omitted entirely (our driver already omits it when no
        // reasoning effort is set). Release/knowledge dates are not published in
        // the model card; the Models API exposes them at runtime.
        "claude-fable-5" => Some(ModelProfile {
            name: "Claude Fable 5".into(),
            family: "claude-fable-5".into(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 10.00,
                output: 50.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-fable-5[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude Opus 5 (current Opus; below Fable 5, above Opus 4.8)
        // Source: Anthropic model card (claude-api skill `shared/models.md`) and
        // docs.claude.com — Opus 5 is not yet in models.dev. A drop-in upgrade at
        // Opus 4.8's pricing ($5/$25, cache-read $0.50) with the same 200K/1M-twin
        // context and 128K output. Adaptive thinking is on by default and sampling
        // parameters are removed (temperature returns 400, hence `temperature:
        // false`), matching the Opus 4.8/4.7 surface. Release/knowledge dates are
        // not published in the model card; the Models API exposes them at runtime.
        "claude-opus-5" => Some(ModelProfile {
            name: "Claude Opus 5".into(),
            family: "claude-opus-5".into(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-opus-5[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.8 series
        // Source: Anthropic model card (claude-api skill `shared/models.md`) and
        // docs.claude.com — Opus 4.8 is not yet in models.dev. Same API surface as
        // Opus 4.7: adaptive thinking only, sampling parameters removed (temperature
        // returns 400, hence `temperature: false`). Release/knowledge dates are not
        // published in the model card; the Models API exposes them at runtime.
        "claude-opus-4-8" => Some(ModelProfile {
            name: "Claude Opus 4.8".into(),
            family: "claude-opus-4-8".into(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-opus-4-8[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.7 series
        // Sampling parameters were removed starting with Opus 4.7: the API
        // rejects `temperature` with "`temperature` is deprecated for this
        // model" (verified live), hence `temperature: false`.
        "claude-opus-4-7" => Some(ModelProfile {
            name: "Claude Opus 4.7".into(),
            family: "claude-opus-4-7".into(),
            description: None,
            release_date: Some("2026-04-16".into()),
            last_updated: Some("2026-04-16".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2026-01-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-opus-4-7[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.6 series
        "claude-opus-4-6" => Some(ModelProfile {
            name: "Claude Opus 4.6".into(),
            family: "claude-opus-4-6".into(),
            description: None,
            release_date: Some("2026-02-05".into()),
            last_updated: Some("2026-02-05".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-05-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-opus-4-6[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: Some(600),
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // 1M-context twins of the base profiles above. Same pricing and
        // capabilities; only the context limit and display name differ.
        "claude-fable-5[1m]" => anthropic_profile_data("claude-fable-5").map(anthropic_1m_variant),
        "claude-opus-5[1m]" => anthropic_profile_data("claude-opus-5").map(anthropic_1m_variant),
        "claude-opus-4-8[1m]" => {
            anthropic_profile_data("claude-opus-4-8").map(anthropic_1m_variant)
        }
        "claude-opus-4-7[1m]" => {
            anthropic_profile_data("claude-opus-4-7").map(anthropic_1m_variant)
        }
        "claude-opus-4-6[1m]" => {
            anthropic_profile_data("claude-opus-4-6").map(anthropic_1m_variant)
        }
        "claude-sonnet-5[1m]" => {
            anthropic_profile_data("claude-sonnet-5").map(anthropic_1m_variant)
        }

        // Claude Sonnet 5
        // Source: Anthropic model card and docs.claude.com — Sonnet 5 is not yet
        // in models.dev. Same API surface as Opus 4.8: adaptive thinking only
        // (budget-based thinking returns 400) and non-default sampling parameters
        // rejected, hence `temperature: false`. Pricing is the $3/$15 sticker; the
        // introductory $2/$10 through 2026-08-31 is deliberately not encoded so
        // the profile stays correct after it lapses. Release/knowledge dates are
        // not published in the model card; the Models API exposes them at runtime.
        "claude-sonnet-5" => Some(ModelProfile {
            name: "Claude Sonnet 5".into(),
            family: "claude-sonnet-5".into(),
            description: None,
            release_date: None,
            last_updated: None,
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-sonnet-5[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-sonnet-4-6" => Some(ModelProfile {
            name: "Claude Sonnet 4.6".into(),
            family: "claude-sonnet-4-6".into(),
            description: None,
            release_date: Some("2026-02-17".into()),
            last_updated: Some("2026-02-17".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.5 series
        "claude-opus-4-5" => Some(ModelProfile {
            name: "Claude Opus 4.5".into(),
            family: "claude-opus-4-5".into(),
            description: None,
            release_date: Some("2025-11-24".into()),
            last_updated: Some("2025-11-24".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-sonnet-4-5" => Some(ModelProfile {
            name: "Claude Sonnet 4.5".into(),
            family: "claude-sonnet-4-5".into(),
            description: None,
            release_date: Some("2025-09-29".into()),
            last_updated: Some("2025-09-29".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-haiku-4-5" => Some(ModelProfile {
            name: "Claude Haiku 4.5".into(),
            family: "claude-haiku-4-5".into(),
            description: None,
            release_date: Some("2025-10-15".into()),
            last_updated: Some("2025-10-15".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.00,
                output: 5.00,
                cache_read: Some(0.10),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 16_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4 series
        "claude-opus-4" => Some(ModelProfile {
            name: "Claude Opus 4".into(),
            family: "claude-opus-4".into(),
            description: None,
            release_date: Some("2025-05-14".into()),
            last_updated: Some("2025-05-14".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 15.00,
                output: 75.00,
                cache_read: Some(1.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 200_000,
                input: None,
                output: 32_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        _ => None,
    }
}

fn gemini_profile_data(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        // Gemini 3.x series (newest). Source: models.dev (google provider).
        // `gemini-3-pro-preview` is deprecated upstream; 3.1 Pro Preview is the
        // current flagship Pro. Pricing has a >200K-token tier. Reasoning effort
        // (low/medium/high) is offered upstream but, consistent with the other
        // Gemini profiles here, effort selection is left unset.
        "gemini-3.1-pro-preview" => Some(ModelProfile {
            name: "Gemini 3.1 Pro Preview".into(),
            family: "gemini-3.1-pro-preview".into(),
            description: None,
            release_date: Some("2026-02-19".into()),
            last_updated: Some("2026-02-19".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-01-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.00,
                output: 12.00,
                cache_read: Some(0.20),
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 4.00,
                    output: 18.00,
                    cache_read: Some(0.40),
                }],
            }),
            limits: Some(ModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                    Modality::Pdf,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Gemini 3.5 Flash — current-gen Flash. Source: models.dev (google
        // provider). Reasoning effort (minimal/low/medium/high) is offered
        // upstream but, consistent with the other Gemini profiles here, effort
        // selection is left unset.
        "gemini-3.5-flash" => Some(ModelProfile {
            name: "Gemini 3.5 Flash".into(),
            family: "gemini-3.5-flash".into(),
            description: None,
            release_date: Some("2026-05-19".into()),
            last_updated: Some("2026-05-19".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-01-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.50,
                output: 9.00,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                    Modality::Pdf,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Gemini 3.1 Flash Lite — low-latency, high-volume tier. Source:
        // models.dev (google provider). Reasoning effort is offered upstream but
        // left unset here, consistent with the other Gemini profiles.
        "gemini-3.1-flash-lite" => Some(ModelProfile {
            name: "Gemini 3.1 Flash Lite".into(),
            family: "gemini-3.1-flash-lite".into(),
            description: None,
            release_date: Some("2026-05-07".into()),
            last_updated: Some("2026-05-07".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-01-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.25,
                output: 1.50,
                cache_read: Some(0.025),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                    Modality::Pdf,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-2.5-pro" => Some(ModelProfile {
            name: "Gemini 2.5 Pro".into(),
            family: "gemini-2.5-pro".into(),
            description: None,
            release_date: Some("2025-03-25".into()),
            last_updated: Some("2025-06-05".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.31),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-2.5-flash" => Some(ModelProfile {
            name: "Gemini 2.5 Flash".into(),
            family: "gemini-2.5-flash".into(),
            description: None,
            release_date: Some("2025-04-17".into()),
            last_updated: Some("2025-06-12".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.15,
                output: 0.60,
                cache_read: Some(0.0375),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-2.0-flash" => Some(ModelProfile {
            name: "Gemini 2.0 Flash".into(),
            family: "gemini-2.0-flash".into(),
            description: None,
            release_date: Some("2025-02-05".into()),
            last_updated: Some("2025-02-05".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.10,
                output: 0.40,
                cache_read: Some(0.025),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_048_576,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text, Modality::Image],
            }),
            reasoning_effort: None,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        _ => None,
    }
}

/// Get LlmSim model profile (simulated LLM for testing)
/// Profile is modeled close to GPT-5.2 for realistic testing
fn llmsim_profile_data(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        "llmsim-default" | "llmsim" => Some(ModelProfile {
            name: "LlmSim Default".into(),
            family: "llmsim".into(),
            description: None,
            release_date: Some("2025-01-01".into()),
            last_updated: Some("2025-01-01".into()),
            attachment: true,
            reasoning: true,
            temperature: false, // Like gpt-5.2
            knowledge: Some("2025-08-31".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 0.00, // Free for testing
                output: 0.00,
                cache_read: Some(0.00),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()), // Same as GPT-5.2
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The per-vendor `normalize_*` functions were folded into the flat registry
    // (longest-prefix matching in `resolve_descriptor`). These shims preserve
    // the original normalization assertions as regression coverage: each maps a
    // wire id to its canonical registry id under the relevant provider surface.
    fn normalize_model_id(id: &str) -> &'static str {
        resolve_descriptor(&DriverId::OpenAI, id)
            .map(|d| d.ids[0])
            .unwrap_or("")
    }
    fn normalize_anthropic_model_id(id: &str) -> &'static str {
        resolve_descriptor(&DriverId::Anthropic, id)
            .map(|d| d.ids[0])
            .unwrap_or("")
    }
    fn normalize_gemini_model_id(id: &str) -> &'static str {
        resolve_descriptor(&DriverId::Gemini, id)
            .map(|d| d.ids[0])
            .unwrap_or("")
    }

    /// Every registered model profile, by the provider surface it is served on.
    /// The list is the single place a new model must be added — the structural
    /// invariant below then exercises it, replacing the per-model constant-mirror
    /// tests that only restated one model's name/family/cost/limits literals.
    const REGISTERED_MODELS: &[(DriverId, &str)] = &[
        // OpenAI
        (DriverId::OpenAI, "gpt-realtime-2"),
        (DriverId::OpenAI, "o3"),
        (DriverId::OpenAI, "o3-pro"),
        (DriverId::OpenAI, "o4-mini"),
        (DriverId::OpenAI, "o3-deep-research"),
        (DriverId::OpenAI, "o4-mini-deep-research"),
        (DriverId::OpenAI, "gpt-4.1"),
        (DriverId::OpenAI, "gpt-4.1-mini"),
        (DriverId::OpenAI, "gpt-4.1-nano"),
        (DriverId::OpenAI, "gpt-5"),
        (DriverId::OpenAI, "gpt-5-mini"),
        (DriverId::OpenAI, "gpt-5-nano"),
        (DriverId::OpenAI, "gpt-5-pro"),
        (DriverId::OpenAI, "gpt-5-codex"),
        (DriverId::OpenAI, "gpt-5-chat-latest"),
        (DriverId::OpenAI, "gpt-5.1"),
        (DriverId::OpenAI, "gpt-5.1-codex"),
        (DriverId::OpenAI, "gpt-5.1-codex-mini"),
        (DriverId::OpenAI, "gpt-5.1-codex-max"),
        (DriverId::OpenAI, "gpt-5.1-chat-latest"),
        (DriverId::OpenAI, "gpt-5.2"),
        (DriverId::OpenAI, "gpt-5.2-pro"),
        (DriverId::OpenAI, "gpt-5.2-codex"),
        (DriverId::OpenAI, "gpt-5.2-chat-latest"),
        (DriverId::OpenAI, "gpt-5.3-codex"),
        (DriverId::OpenAI, "gpt-5.4"),
        (DriverId::OpenAI, "gpt-5.4-mini"),
        (DriverId::OpenAI, "gpt-5.4-nano"),
        (DriverId::OpenAI, "gpt-5.4-pro"),
        (DriverId::OpenAI, "gpt-5.5"),
        (DriverId::OpenAI, "gpt-5.5-pro"),
        (DriverId::OpenAI, "gpt-5.6-sol"),
        (DriverId::OpenAI, "gpt-5.6-terra"),
        (DriverId::OpenAI, "gpt-5.6-luna"),
        // Anthropic
        (DriverId::Anthropic, "claude-fable-5"),
        (DriverId::Anthropic, "claude-opus-5"),
        (DriverId::Anthropic, "claude-opus-4-8"),
        (DriverId::Anthropic, "claude-opus-4-7"),
        (DriverId::Anthropic, "claude-opus-4-6"),
        (DriverId::Anthropic, "claude-sonnet-5"),
        (DriverId::Anthropic, "claude-sonnet-4-6"),
        (DriverId::Anthropic, "claude-opus-4-5"),
        (DriverId::Anthropic, "claude-sonnet-4-5"),
        (DriverId::Anthropic, "claude-haiku-4-5"),
        (DriverId::Anthropic, "claude-opus-4"),
        // Gemini
        (DriverId::Gemini, "gemini-3.1-pro-preview"),
        (DriverId::Gemini, "gemini-3.5-flash"),
        (DriverId::Gemini, "gemini-3.1-flash-lite"),
        (DriverId::Gemini, "gemini-2.5-pro"),
        (DriverId::Gemini, "gemini-2.5-flash"),
        (DriverId::Gemini, "gemini-2.0-flash"),
    ];

    /// Structural invariants that must hold for every registered profile. This
    /// replaces the ~40 per-model tests that each re-asserted one model's
    /// hardcoded name/family/cost/limits: instead of pinning literals, it
    /// enforces the properties that would catch a real config defect (a blank
    /// name, negative price, output exceeding context, or a reasoning-effort set
    /// whose default is not among its own values).
    #[test]
    fn registered_model_profiles_are_structurally_consistent() {
        for (provider, id) in REGISTERED_MODELS {
            let p = get_model_profile(provider, id)
                .unwrap_or_else(|| panic!("{id} should resolve under {provider:?}"));

            assert!(!p.name.trim().is_empty(), "{id}: empty name");
            assert!(!p.family.trim().is_empty(), "{id}: empty family");

            if let Some(cost) = &p.cost {
                assert!(
                    cost.input.is_finite() && cost.input >= 0.0,
                    "{id}: bad input cost {}",
                    cost.input
                );
                assert!(
                    cost.output.is_finite() && cost.output >= 0.0,
                    "{id}: bad output cost {}",
                    cost.output
                );
                if let Some(cache_read) = cost.cache_read {
                    assert!(
                        cache_read.is_finite() && cache_read >= 0.0 && cache_read <= cost.input,
                        "{id}: cache_read {cache_read} must be >=0 and <= input {}",
                        cost.input
                    );
                }
            }

            if let Some(limits) = &p.limits {
                assert!(limits.context > 0, "{id}: non-positive context");
                assert!(limits.output > 0, "{id}: non-positive output");
                assert!(
                    limits.output <= limits.context,
                    "{id}: output {} exceeds context {}",
                    limits.output,
                    limits.context
                );
            }

            if let Some(effort) = &p.reasoning_effort {
                assert!(
                    p.reasoning,
                    "{id}: has reasoning_effort but reasoning flag is false"
                );
                assert!(
                    !effort.values.is_empty(),
                    "{id}: empty reasoning_effort set"
                );
                let mut seen: Vec<&ReasoningEffort> = Vec::new();
                for v in &effort.values {
                    assert!(
                        !v.name.trim().is_empty(),
                        "{id}: reasoning_effort value with empty display name"
                    );
                    assert!(
                        !seen.contains(&&v.value),
                        "{id}: duplicate reasoning_effort value {:?}",
                        v.value
                    );
                    seen.push(&v.value);
                }
                assert!(
                    effort.values.iter().any(|v| v.value == effort.default),
                    "{id}: reasoning_effort default {:?} is not among its values",
                    effort.default
                );
            }

            if let Some(speed) = &p.speed {
                assert!(!speed.values.is_empty(), "{id}: empty speed set");
                let mut seen: Vec<&Speed> = Vec::new();
                for v in &speed.values {
                    assert!(
                        !v.name.trim().is_empty(),
                        "{id}: speed value with empty display name"
                    );
                    assert!(
                        !seen.contains(&&v.value),
                        "{id}: duplicate speed value {:?}",
                        v.value
                    );
                    seen.push(&v.value);
                }
                assert!(
                    speed.values.iter().any(|v| v.value == speed.default),
                    "{id}: speed default {:?} is not among its values",
                    speed.default
                );
            }
        }
    }

    #[test]
    fn test_profile_keys_and_service_kinds() {
        // Canonical key from a dated wire id (version-suffix normalization).
        assert_eq!(
            get_model_profile_key(&DriverId::Anthropic, "claude-sonnet-4-5-20250929").as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        // Gateway alias and bare id share one key (same model identity).
        assert_eq!(
            get_model_profile_key(&DriverId::OpenRouter, "nvidia/nemotron-3-super-120b-a12b"),
            get_model_profile_key(&DriverId::OpenAI, "nemotron-3-super-120b-a12b"),
        );
        // Unknown models have no key.
        assert_eq!(
            get_model_profile_key(&DriverId::OpenAI, "not-a-model"),
            None
        );

        // By-key lookup round-trips.
        let profile = get_model_profile_by_key("openai/gpt-5.5").unwrap();
        assert_eq!(profile.name, "GPT-5.5");
        // Both key segments are matched ASCII case-insensitively.
        assert!(get_model_profile_by_key("OpenAI/GPT-5.5").is_some());
        assert!(get_model_profile_by_key("openai/not-a-model").is_none());
        assert!(get_model_profile_by_key("no-slash").is_none());

        // Service kinds: realtime models are not chat models.
        assert_eq!(
            get_model_service_kind(&DriverId::OpenAI, "gpt-realtime-2"),
            ServiceKind::Realtime
        );
        assert_eq!(
            get_model_service_kind(&DriverId::OpenAI, "gpt-5.5"),
            ServiceKind::Chat
        );
        // Unknown models default to chat.
        assert_eq!(
            get_model_service_kind(&DriverId::OpenAI, "not-a-model"),
            ServiceKind::Chat
        );
    }

    // Per-model name/family/cost/limits constants covered by registered_model_profiles_are_structurally_consistent.

    #[test]
    fn test_get_profile_openai_versioned() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.2-2025-12-11");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "GPT-5.2");
    }

    #[test]
    fn test_get_profile_unknown_model() {
        let profile = get_model_profile(&DriverId::OpenAI, "unknown-model");
        assert!(profile.is_none());
    }

    #[test]
    fn test_retired_semantic_variants_do_not_resolve_to_parent_profiles() {
        assert!(get_model_profile(&DriverId::OpenAI, "o3-mini").is_none());
        assert!(get_model_profile(&DriverId::Anthropic, "claude-opus-4-1").is_none());
    }

    #[test]
    fn test_get_profile_wrong_provider() {
        // Try to get an OpenAI model with Anthropic provider
        let profile = get_model_profile(&DriverId::Anthropic, "gpt-5.2");
        assert!(profile.is_none());
    }

    // Per-model name/family/cost/limits constants covered by registered_model_profiles_are_structurally_consistent.

    #[test]
    fn test_normalize_openai_model_id() {
        assert_eq!(normalize_model_id("gpt-5.2"), "gpt-5.2");
        assert_eq!(normalize_model_id("gpt-5.2-2025-12-11"), "gpt-5.2");
        assert_eq!(normalize_model_id("gpt-5.4-mini"), "gpt-5.4-mini");
        assert_eq!(normalize_model_id("o3-2025-04-16"), "o3");
        assert_eq!(normalize_model_id("o4-mini"), "o4-mini");
    }

    #[test]
    fn test_normalize_anthropic_model_id() {
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-5"),
            "claude-sonnet-5"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-5-latest"),
            "claude-sonnet-5"
        );
    }

    #[test]
    fn test_openai_completions_uses_openai_profiles() {
        let profile = get_model_profile(&DriverId::OpenAICompletions, "gpt-5.2");
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "GPT-5.2");
    }

    #[test]
    fn test_azure_openai_uses_openai_profiles() {
        let profile = get_model_profile(&DriverId::AzureOpenAI, "gpt-5.2");
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "GPT-5.2");
    }

    // Speed (service tier) availability follows OpenAI's official tier tables;
    // see the speed_* helper docs.

    #[test]
    fn test_speed_config_matches_pricing_tiers() {
        let speeds = |model: &str| -> Vec<Speed> {
            get_model_profile(&DriverId::OpenAI, model)
                .unwrap()
                .speed
                .map(|s| s.values.into_iter().map(|v| v.value).collect())
                .unwrap_or_default()
        };
        use Speed::*;
        // Flex + priority pricing rows.
        for model in [
            "gpt-5.6-sol",
            "gpt-5.6-terra",
            "gpt-5.6-luna",
            "gpt-5.5",
            "gpt-5.4",
            "gpt-5.4-mini",
        ] {
            assert_eq!(speeds(model), vec![Flex, Default, Priority], "{model}");
        }
        // Flex-only pricing rows.
        for model in ["gpt-5.5-pro", "gpt-5.4-nano", "gpt-5.4-pro"] {
            assert_eq!(speeds(model), vec![Flex, Default], "{model}");
        }
        // Priority-only pricing rows.
        for model in [
            "gpt-4.1",
            "gpt-4.1-mini",
            "gpt-4.1-nano",
            "gpt-5",
            "gpt-5-mini",
            "gpt-5-codex",
            "gpt-5.1",
            "gpt-5.1-codex",
            "gpt-5.2",
            "gpt-5.3-codex",
            "o3",
            "o4-mini",
        ] {
            assert_eq!(speeds(model), vec![Default, Priority], "{model}");
        }
        // No tier rows: unlisted variants, chat-latest, and deep research.
        for model in [
            "gpt-5-nano",
            "gpt-5-pro",
            "gpt-5.1-codex-mini",
            "gpt-5.1-codex-max",
            "gpt-5.2-pro",
            "gpt-5.2-codex",
            "gpt-5-chat-latest",
            "o3-deep-research",
        ] {
            assert_eq!(speeds(model), vec![], "{model}");
        }
    }

    #[test]
    fn test_speed_masked_for_non_openai_surfaces() {
        assert!(
            get_model_profile(&DriverId::OpenAI, "gpt-5.2")
                .unwrap()
                .speed
                .is_some()
        );
        // Service tiers are OpenAI-platform billing; other surfaces reaching
        // the same model must not advertise the selector.
        for provider in [
            DriverId::AzureOpenAI,
            DriverId::OpenRouter,
            DriverId::OpenAICompletions,
        ] {
            assert!(
                get_model_profile(&provider, "gpt-5.2")
                    .unwrap()
                    .speed
                    .is_none(),
                "{provider:?}"
            );
        }
    }

    // GPT-5 model tests

    #[test]
    fn test_gpt5_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5").unwrap();
        assert_eq!(profile.name, "GPT-5");
        assert_eq!(profile.family, "gpt-5");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        // Pre-5.1 reasoning effort: default medium, supports low/medium/high
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
        assert!(
            !effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::None)
        );
    }

    #[test]
    fn test_gpt5_mini_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5-mini").unwrap();
        assert_eq!(profile.name, "GPT-5 mini");
        assert!(profile.reasoning);

        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    #[test]
    fn test_gpt5_pro_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5-pro").unwrap();
        assert_eq!(profile.name, "GPT-5 Pro");
        assert!(profile.reasoning);

        // gpt-5-pro: only supports high
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 1);
        assert_eq!(effort.values[0].value, ReasoningEffort::High);
    }

    #[test]
    fn test_gpt51_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.1").unwrap();
        assert_eq!(profile.name, "GPT-5.1");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        // gpt-5.1: default none, supports none/low/medium/high
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::None);
        assert_eq!(effort.values.len(), 4);
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::None)
        );
        assert!(
            !effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_gpt51_codex_max_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.1-codex-max").unwrap();
        assert_eq!(profile.name, "GPT-5.1 Codex max");
        assert!(profile.reasoning);

        // After gpt-5.1-codex-max: supports xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_gpt52_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.2").unwrap();
        assert_eq!(profile.name, "GPT-5.2");
        assert!(profile.reasoning);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 1.75).abs() < f64::EPSILON);
        assert!((cost.output - 14.00).abs() < f64::EPSILON);

        // gpt-5.2: default none, supports none/low/medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::None);
        assert_eq!(effort.values.len(), 5);
    }

    #[test]
    fn test_gpt52_pro_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.2-pro").unwrap();
        assert_eq!(profile.name, "GPT-5.2 Pro");
        assert!(profile.reasoning);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 21.00).abs() < f64::EPSILON);
        assert!((cost.output - 168.00).abs() < f64::EPSILON);

        // gpt-5.2-pro: default medium, supports medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
    }

    #[test]
    fn test_gpt55_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.5").unwrap();
        assert_eq!(profile.name, "GPT-5.5");
        assert_eq!(profile.family, "gpt-5.5");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        assert!(profile.tool_search);
        assert!(profile.supports_phases);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_050_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 5.00).abs() < f64::EPSILON);
        assert!((cost.output - 30.00).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.50).abs() < f64::EPSILON);
        assert!(cost.cost_tiers.is_empty()); // Flat pricing, no 200K tier

        // gpt-5.5: default medium, supports none/low/medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 5);
    }

    #[test]
    fn test_gpt_realtime_2_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-realtime-2").unwrap();
        assert_eq!(profile.name, "GPT Realtime 2");
        assert_eq!(profile.family, "gpt-realtime");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.supports_phases);

        let modalities = profile.modalities.unwrap();
        assert!(modalities.input.contains(&Modality::Audio));
        assert!(modalities.output.contains(&Modality::Audio));

        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Low);
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Minimal)
        );
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_gpt55_pro_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.5-pro").unwrap();
        assert_eq!(profile.name, "GPT-5.5 Pro");
        assert_eq!(profile.family, "gpt-5.5-pro");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        assert!(profile.tool_search);
        assert!(profile.supports_phases);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 30.00).abs() < f64::EPSILON);
        assert!((cost.output - 180.00).abs() < f64::EPSILON);
        assert!(cost.cache_read.is_none());
        assert!(cost.cost_tiers.is_empty());

        // gpt-5.5-pro: default medium, supports medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
    }

    #[test]
    fn test_gpt55_versioned() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.5-2026-04-23").unwrap();
        assert_eq!(profile.name, "GPT-5.5");

        let pro = get_model_profile(&DriverId::OpenAI, "gpt-5.5-pro-2026-04-23").unwrap();
        assert_eq!(pro.name, "GPT-5.5 Pro");
    }

    #[test]
    fn test_normalize_gpt55_model_ids() {
        assert_eq!(normalize_model_id("gpt-5.5"), "gpt-5.5");
        assert_eq!(normalize_model_id("gpt-5.5-2026-04-23"), "gpt-5.5");
        assert_eq!(normalize_model_id("gpt-5.5-pro"), "gpt-5.5-pro");
        assert_eq!(normalize_model_id("gpt-5.5-pro-2026-04-23"), "gpt-5.5-pro");
    }

    #[test]
    fn test_gpt56_profiles() {
        // Sol, Terra, Luna share the same shape: 1.05M context, 128K output,
        // 2026-02-16 knowledge cutoff, tool_search + native phases, and a
        // >272K-token pricing tier (2x input, 1.5x output).
        for (id, name, family, input, output, cache, tier_input, tier_output, tier_cache) in [
            (
                "gpt-5.6-sol",
                "GPT-5.6 Sol",
                "gpt-5.6-sol",
                5.00,
                30.00,
                0.50,
                10.00,
                45.00,
                1.00,
            ),
            (
                "gpt-5.6-terra",
                "GPT-5.6 Terra",
                "gpt-5.6-terra",
                2.50,
                15.00,
                0.25,
                5.00,
                22.50,
                0.50,
            ),
            (
                "gpt-5.6-luna",
                "GPT-5.6 Luna",
                "gpt-5.6-luna",
                1.00,
                6.00,
                0.10,
                2.00,
                9.00,
                0.20,
            ),
        ] {
            let profile = get_model_profile(&DriverId::OpenAI, id).unwrap();
            assert_eq!(profile.name, name);
            assert_eq!(profile.family, family);
            assert!(profile.reasoning);
            assert!(!profile.temperature);
            assert!(profile.tool_call);
            assert!(profile.structured_output);
            assert!(profile.tool_search);
            assert!(profile.supports_phases);
            assert_eq!(profile.knowledge.as_deref(), Some("2026-02-16"));

            let limits = profile.limits.unwrap();
            assert_eq!(limits.context, 1_050_000);
            assert_eq!(limits.output, 128_000);

            let cost = profile.cost.unwrap();
            assert!((cost.input - input).abs() < f64::EPSILON);
            assert!((cost.output - output).abs() < f64::EPSILON);
            assert!((cost.cache_read.unwrap() - cache).abs() < f64::EPSILON);
            assert_eq!(cost.cost_tiers.len(), 1);
            let tier = &cost.cost_tiers[0];
            assert_eq!(tier.above_tokens, 272_000);
            assert!((tier.input - tier_input).abs() < f64::EPSILON);
            assert!((tier.output - tier_output).abs() < f64::EPSILON);
            assert!((tier.cache_read.unwrap() - tier_cache).abs() < f64::EPSILON);

            // Series default: medium, supports none/low/medium/high/xhigh.
            let effort = profile.reasoning_effort.unwrap();
            assert_eq!(effort.default, ReasoningEffort::Medium);
            assert_eq!(effort.values.len(), 5);
        }
    }

    #[test]
    fn test_gpt56_versioned() {
        // Dated wire ids resolve to the canonical profile via the "<id>-" prefix.
        let sol = get_model_profile(&DriverId::OpenAI, "gpt-5.6-sol-2026-07-09").unwrap();
        assert_eq!(sol.name, "GPT-5.6 Sol");
        assert_eq!(normalize_model_id("gpt-5.6-sol-2026-07-09"), "gpt-5.6-sol");
        assert_eq!(normalize_model_id("gpt-5.6-luna"), "gpt-5.6-luna");
    }

    #[test]
    fn test_gpt54_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.4").unwrap();
        assert_eq!(profile.name, "GPT-5.4");
        assert_eq!(profile.family, "gpt-5.4");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_050_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 2.50).abs() < f64::EPSILON);
        assert!((cost.output - 15.00).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.25).abs() < f64::EPSILON);

        // Tiered pricing above 200K tokens
        assert_eq!(cost.cost_tiers.len(), 1);
        let tier = &cost.cost_tiers[0];
        assert_eq!(tier.above_tokens, 200_000);
        assert!((tier.input - 5.00).abs() < f64::EPSILON);
        assert!((tier.output - 22.50).abs() < f64::EPSILON);
        assert!((tier.cache_read.unwrap() - 0.50).abs() < f64::EPSILON);

        assert!(profile.description.is_some());
        assert!(profile.supports_phases);
        assert!(profile.tool_search);

        // gpt-5.4: default none, supports none/low/medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::None);
        assert_eq!(effort.values.len(), 5);
    }

    #[test]
    fn test_gpt54_mini_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.4-mini").unwrap();
        assert_eq!(profile.name, "GPT-5.4 mini");
        assert_eq!(profile.family, "gpt-5.4-mini");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        assert!(profile.tool_search);
        assert!(profile.supports_phases);
        assert!(profile.description.is_some());

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 0.75).abs() < f64::EPSILON);
        assert!((cost.output - 4.50).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.075).abs() < f64::EPSILON);
        assert!(cost.cost_tiers.is_empty()); // No tiered pricing
    }

    #[test]
    fn test_gpt54_nano_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.4-nano").unwrap();
        assert_eq!(profile.name, "GPT-5.4 nano");
        assert_eq!(profile.family, "gpt-5.4-nano");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.tool_search);
        assert!(profile.supports_phases);
        assert!(profile.description.is_some());

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 0.20).abs() < f64::EPSILON);
        assert!((cost.output - 1.25).abs() < f64::EPSILON);
        assert!((cost.cache_read.unwrap() - 0.02).abs() < f64::EPSILON);
        assert!(cost.cost_tiers.is_empty()); // No tiered pricing
    }

    #[test]
    fn test_gpt54_pro_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.4-pro").unwrap();
        assert_eq!(profile.name, "GPT-5.4 Pro");
        assert_eq!(profile.family, "gpt-5.4-pro");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(!profile.structured_output); // Not supported for pro
        assert!(profile.description.is_some());

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_050_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 30.00).abs() < f64::EPSILON);
        assert!((cost.output - 180.00).abs() < f64::EPSILON);
        assert!(cost.cache_read.is_none());

        // Tiered pricing above 200K tokens
        assert_eq!(cost.cost_tiers.len(), 1);
        let tier = &cost.cost_tiers[0];
        assert_eq!(tier.above_tokens, 200_000);
        assert!((tier.input - 60.00).abs() < f64::EPSILON);
        assert!((tier.output - 270.00).abs() < f64::EPSILON);

        assert!(profile.supports_phases);

        // gpt-5.4-pro: default medium, supports medium/high/xhigh
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
        assert_eq!(effort.values.len(), 3);
    }

    #[test]
    fn test_gpt54_versioned() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.4-2026-03-05").unwrap();
        assert_eq!(profile.name, "GPT-5.4");
    }

    #[test]
    fn test_gpt54_mini_versioned() {
        let profile = get_model_profile(&DriverId::OpenAI, "gpt-5.4-mini-2026-03-17").unwrap();
        assert_eq!(profile.name, "GPT-5.4 mini");
    }

    #[test]
    fn test_normalize_gpt5_model_ids() {
        assert_eq!(normalize_model_id("gpt-5"), "gpt-5");
        assert_eq!(normalize_model_id("gpt-5-2025-08-07"), "gpt-5");
        assert_eq!(normalize_model_id("gpt-5-mini"), "gpt-5-mini");
        assert_eq!(normalize_model_id("gpt-5-nano"), "gpt-5-nano");
        assert_eq!(normalize_model_id("gpt-5-pro"), "gpt-5-pro");
        assert_eq!(normalize_model_id("gpt-5-codex"), "gpt-5-codex");
        assert_eq!(normalize_model_id("gpt-5.1"), "gpt-5.1");
        assert_eq!(normalize_model_id("gpt-5.1-codex"), "gpt-5.1-codex");
        assert_eq!(
            normalize_model_id("gpt-5.1-codex-mini"),
            "gpt-5.1-codex-mini"
        );
        assert_eq!(normalize_model_id("gpt-5.1-codex-max"), "gpt-5.1-codex-max");
        assert_eq!(normalize_model_id("gpt-5.2"), "gpt-5.2");
        assert_eq!(normalize_model_id("gpt-5.2-pro"), "gpt-5.2-pro");
        assert_eq!(normalize_model_id("gpt-5.2-codex"), "gpt-5.2-codex");
        assert_eq!(normalize_model_id("gpt-5.3-codex"), "gpt-5.3-codex");
        assert_eq!(normalize_model_id("gpt-5.4"), "gpt-5.4");
        assert_eq!(normalize_model_id("gpt-5.4-2026-03-05"), "gpt-5.4");
        assert_eq!(normalize_model_id("gpt-5.4-pro"), "gpt-5.4-pro");
        assert_eq!(normalize_model_id("gpt-5.4-mini"), "gpt-5.4-mini");
        assert_eq!(
            normalize_model_id("gpt-5.4-mini-2026-03-17"),
            "gpt-5.4-mini"
        );
        assert_eq!(normalize_model_id("gpt-5.4-nano"), "gpt-5.4-nano");
        assert_eq!(
            normalize_model_id("gpt-5.4-nano-2026-03-17"),
            "gpt-5.4-nano"
        );
    }

    // o3/o4 reasoning model tests

    #[test]
    fn test_o3_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "o3").unwrap();
        assert_eq!(profile.name, "o3");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    #[test]
    fn test_o3_pro_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "o3-pro").unwrap();
        assert_eq!(profile.name, "o3 Pro");
        assert!(profile.reasoning);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
    }

    #[test]
    fn test_o4_mini_profile() {
        let profile = get_model_profile(&DriverId::OpenAI, "o4-mini").unwrap();
        assert_eq!(profile.name, "o4 mini");
        assert!(profile.reasoning);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    // Claude 4.7 / 4.6 model tests

    #[test]
    fn test_claude_opus_47_profile() {
        let profile = get_model_profile(&DriverId::Anthropic, "claude-opus-4-7").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.7");
        assert_eq!(profile.family, "claude-opus-4-7");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        // Sampling parameters removed from Opus 4.7 on (API rejects `temperature`).
        assert!(!profile.temperature);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        // Bare id is the 200K profile; the 1M window is `claude-opus-4-7[1m]`.
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 128_000);
        assert_eq!(limits.max_media, None);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 5.00).abs() < f64::EPSILON);
        assert!((cost.output - 25.00).abs() < f64::EPSILON);

        let modalities = profile.modalities.unwrap();
        assert_eq!(
            modalities.input,
            vec![Modality::Text, Modality::Image, Modality::Pdf]
        );

        // Adaptive thinking: default high, supports low/medium/high/max(xhigh)
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 4);
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_claude_opus_46_profile() {
        let profile = get_model_profile(&DriverId::Anthropic, "claude-opus-4-6").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.6");
        assert_eq!(profile.family, "claude-opus-4-6");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.temperature);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        // Bare id is the 200K profile; the 1M window is `claude-opus-4-6[1m]`.
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 128_000);
        assert_eq!(limits.max_media, Some(600));

        let cost = profile.cost.unwrap();
        assert!((cost.input - 5.00).abs() < f64::EPSILON);
        assert!((cost.output - 25.00).abs() < f64::EPSILON);

        let modalities = profile.modalities.unwrap();
        assert_eq!(modalities.input, vec![Modality::Text, Modality::Image]);

        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 4);
    }

    #[test]
    fn test_claude_sonnet_46_profile() {
        let profile = get_model_profile(&DriverId::Anthropic, "claude-sonnet-4-6").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4.6");
        assert_eq!(profile.family, "claude-sonnet-4-6");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 64_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 3.00).abs() < f64::EPSILON);
        assert!((cost.output - 15.00).abs() < f64::EPSILON);

        // Adaptive thinking: default high, supports low/medium/high/max(xhigh)
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
        assert_eq!(effort.values.len(), 4);
        assert!(
            effort
                .values
                .iter()
                .any(|v| v.value == ReasoningEffort::Xhigh)
        );
    }

    #[test]
    fn test_claude_opus_47_versioned() {
        let profile = get_model_profile(&DriverId::Anthropic, "claude-opus-4-7-20260416").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.7");
    }

    #[test]
    fn test_claude_sonnet_46_versioned() {
        let profile =
            get_model_profile(&DriverId::Anthropic, "claude-sonnet-4-6-20260217").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4.6");
    }

    // Per-model name/family/cost/limits constants covered by registered_model_profiles_are_structurally_consistent.

    // Normalize tests for new models

    #[test]
    fn test_normalize_gpt41_model_ids() {
        assert_eq!(normalize_model_id("gpt-4.1"), "gpt-4.1");
        assert_eq!(normalize_model_id("gpt-4.1-2025-04-14"), "gpt-4.1");
        assert_eq!(normalize_model_id("gpt-4.1-mini"), "gpt-4.1-mini");
        assert_eq!(normalize_model_id("gpt-4.1-nano"), "gpt-4.1-nano");
    }

    #[test]
    fn test_normalize_o_series_model_ids() {
        assert_eq!(normalize_model_id("o3"), "o3");
        assert_eq!(normalize_model_id("o3-2025-04-16"), "o3");
        assert_eq!(normalize_model_id("o3-pro"), "o3-pro");
        assert_eq!(normalize_model_id("o4-mini"), "o4-mini");
    }

    #[test]
    fn test_normalize_claude_47_and_46_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-7"),
            "claude-opus-4-7"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-7-20260416"),
            "claude-opus-4-7"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-6"),
            "claude-opus-4-6"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-6-20260205"),
            "claude-opus-4-6"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-6"),
            "claude-sonnet-4-6"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-6-20260217"),
            "claude-sonnet-4-6"
        );
    }

    #[test]
    fn test_normalize_claude_45_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-5-20251101"),
            "claude-opus-4-5"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-5-20250929"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-haiku-4-5-20251001"),
            "claude-haiku-4-5"
        );
    }

    // Gemini model tests

    #[test]
    fn test_normalize_gemini_model_ids() {
        assert_eq!(
            normalize_gemini_model_id("gemini-2.5-pro"),
            "gemini-2.5-pro"
        );
        assert_eq!(
            normalize_gemini_model_id("gemini-2.5-pro-preview-05-06"),
            "gemini-2.5-pro"
        );
        assert_eq!(
            normalize_gemini_model_id("gemini-2.5-flash-preview-04-17"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            normalize_gemini_model_id("gemini-2.0-flash"),
            "gemini-2.0-flash"
        );
    }

    #[test]
    fn test_gemini_unknown_model() {
        let profile = get_model_profile(&DriverId::Gemini, "unknown-model");
        assert!(profile.is_none());
    }

    // Newly added flagship model profiles

    #[test]
    fn test_claude_opus_4_8_1m_variant() {
        // `[1m]` is the large-context twin: same flat pricing, 1M context,
        // "(1M)" display suffix, shared family for grouping.
        let base = get_model_profile(&DriverId::Anthropic, "claude-opus-4-8").unwrap();
        assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

        let m1 = get_model_profile(&DriverId::Anthropic, "claude-opus-4-8[1m]").unwrap();
        assert_eq!(m1.name, "Claude Opus 4.8 (1M)");
        assert_eq!(m1.family, "claude-opus-4-8");
        assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
        assert_eq!(m1.limits.as_ref().unwrap().output, 128_000);

        // Flat standard pricing — Anthropic serves the 1M window with no
        // long-context premium, so cost matches the 200K base exactly.
        let (base_cost, m1_cost) = (base.cost.unwrap(), m1.cost.unwrap());
        assert_eq!(m1_cost.input, base_cost.input);
        assert_eq!(m1_cost.output, base_cost.output);
        assert_eq!(m1_cost.cache_read, base_cost.cache_read);
        assert!(m1_cost.cost_tiers.is_empty());
    }

    #[test]
    fn test_claude_opus_5_1m_variant() {
        let base = get_model_profile(&DriverId::Anthropic, "claude-opus-5").unwrap();
        assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

        let m1 = get_model_profile(&DriverId::Anthropic, "claude-opus-5[1m]").unwrap();
        assert_eq!(m1.name, "Claude Opus 5 (1M)");
        assert_eq!(m1.family, "claude-opus-5");
        assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
        assert_eq!(m1.limits.as_ref().unwrap().output, 128_000);

        // Flat standard pricing — the 1M window carries no long-context premium,
        // so cost matches the 200K base exactly.
        let (base_cost, m1_cost) = (base.cost.unwrap(), m1.cost.unwrap());
        assert_eq!(m1_cost.input, base_cost.input);
        assert_eq!(m1_cost.output, base_cost.output);
        assert_eq!(m1_cost.cache_read, base_cost.cache_read);
        assert!(m1_cost.cost_tiers.is_empty());
    }

    #[test]
    fn test_claude_fable_5_1m_variant() {
        let base = get_model_profile(&DriverId::Anthropic, "claude-fable-5").unwrap();
        assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

        let m1 = get_model_profile(&DriverId::Anthropic, "claude-fable-5[1m]").unwrap();
        assert_eq!(m1.name, "Claude Fable 5 (1M)");
        assert_eq!(m1.family, "claude-fable-5");
        assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
        assert_eq!(m1.cost.unwrap().input, base.cost.unwrap().input);
    }

    #[test]
    fn test_claude_opus_4_7_and_4_6_have_1m_variants() {
        for id in ["claude-opus-4-7[1m]", "claude-opus-4-6[1m]"] {
            let m1 = get_model_profile(&DriverId::Anthropic, id).unwrap();
            assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
            assert!(m1.name.ends_with("(1M)"));
        }
    }

    #[test]
    fn test_claude_sonnet_5_1m_variant() {
        let base = get_model_profile(&DriverId::Anthropic, "claude-sonnet-5").unwrap();
        assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

        let m1 = get_model_profile(&DriverId::Anthropic, "claude-sonnet-5[1m]").unwrap();
        assert_eq!(m1.name, "Claude Sonnet 5 (1M)");
        assert_eq!(m1.family, "claude-sonnet-5");
        assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
        assert_eq!(m1.cost.unwrap().input, base.cost.unwrap().input);
    }

    #[test]
    fn test_gemini_3_1_pro_preview_profile() {
        let profile = get_model_profile(&DriverId::Gemini, "gemini-3.1-pro-preview").unwrap();
        assert_eq!(profile.name, "Gemini 3.1 Pro Preview");
        assert!(profile.reasoning);
        // >200K-token pricing tier.
        let cost = profile.cost.as_ref().unwrap();
        assert_eq!(cost.cost_tiers.len(), 1);
        assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
        assert_eq!(cost.cost_tiers[0].input, 4.00);
    }

    #[test]
    fn test_gemini_3_1_pro_preview_normalizes_dated_suffix() {
        let profile = get_model_profile(&DriverId::Gemini, "gemini-3.1-pro-preview-02-19").unwrap();
        assert_eq!(profile.family, "gemini-3.1-pro-preview");
    }

    #[test]
    fn test_third_party_profiles_via_openai_completions() {
        // Bare ids and common vendor-prefixed aliases both resolve.
        let cases = [
            ("nemotron-3-super-120b-a12b", "Nemotron 3 Super"),
            ("nvidia/nemotron-3-super-120b-a12b", "Nemotron 3 Super"),
            ("qwen3.7-max", "Qwen3.7 Max"),
            ("MAI-1-preview", "MAI-1-preview"),
            ("MiniMax-M3", "MiniMax-M3"),
            ("kimi-k2-thinking", "Kimi K2 Thinking"),
            ("kimi-k3", "Kimi K3"),
            ("moonshotai/kimi-k3", "Kimi K3"),
            ("grok-4.3", "Grok 4.3"),
            ("x-ai/grok-4.3", "Grok 4.3"),
        ];
        for (id, name) in cases {
            let profile = get_model_profile(&DriverId::OpenAICompletions, id)
                .unwrap_or_else(|| panic!("missing profile for {id}"));
            assert_eq!(profile.name, name, "wrong profile for {id}");
        }
    }

    #[test]
    fn test_kimi_k3_profile() {
        let profile = get_model_profile(&DriverId::OpenAICompletions, "kimi-k3").unwrap();
        assert_eq!(profile.name, "Kimi K3");
        assert_eq!(profile.family, "kimi-k3");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.open_weights);
        assert!(!profile.temperature);
        let cost = profile.cost.as_ref().unwrap();
        assert_eq!(cost.input, 3.00);
        assert_eq!(cost.output, 15.00);
        assert_eq!(cost.cache_read, Some(0.30));
        assert!(cost.cost_tiers.is_empty());
        let limits = profile.limits.as_ref().unwrap();
        assert_eq!(limits.context, 1_048_576);
        assert_eq!(limits.output, 131_072);
        let modalities = profile.modalities.as_ref().unwrap();
        assert_eq!(
            modalities.input,
            vec![Modality::Text, Modality::Image, Modality::Video]
        );
    }

    #[test]
    fn test_grok_4_3_has_context_tier() {
        let profile = get_model_profile(&DriverId::OpenAICompletions, "grok-4.3").unwrap();
        let cost = profile.cost.as_ref().unwrap();
        assert_eq!(cost.cost_tiers.len(), 1);
        assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
    }

    #[test]
    fn test_mai_preview_has_no_cost_or_limits() {
        // Microsoft never published pricing/limits for MAI-1-preview.
        let profile = get_model_profile(&DriverId::OpenAICompletions, "MAI-1-preview").unwrap();
        assert!(profile.cost.is_none());
        assert!(profile.limits.is_none());
        assert!(!profile.reasoning);
    }

    #[test]
    fn test_third_party_unknown_still_none() {
        let profile = get_model_profile(&DriverId::OpenAICompletions, "totally-made-up");
        assert!(profile.is_none());
    }

    #[test]
    fn test_third_party_surfaces() {
        // Third-party, OpenAI-compatible models are reachable via the Chat
        // Completions path AND via Responses-capable gateways (e.g. OpenRouter
        // configured as an `openai` provider) — but never Azure OpenAI.
        for id in [
            "qwen3.7-max",
            "MiniMax-M3",
            "grok-4.3",
            "nemotron-3-super-120b-a12b",
        ] {
            assert!(
                get_model_profile(&DriverId::OpenAICompletions, id).is_some(),
                "{id} should resolve under openai_completions"
            );
            assert!(
                get_model_profile(&DriverId::OpenAI, id).is_some(),
                "{id} should resolve under openai (Open Responses gateway)"
            );
            assert!(
                get_model_profile(&DriverId::AzureOpenAI, id).is_none(),
                "{id} must not resolve under azure_openai"
            );
            assert!(
                get_model_profile(&DriverId::Anthropic, id).is_none(),
                "{id} must not resolve under anthropic"
            );
        }
        // Native phases / tool_search are advertised only on the Responses
        // surface, never on Chat Completions.
        let grok_completions = get_model_profile(&DriverId::OpenAICompletions, "grok-4.3").unwrap();
        assert!(!grok_completions.supports_phases);
        assert!(!grok_completions.tool_search);

        // Genuine OpenAI models still resolve under all OpenAI-family types.
        assert!(get_model_profile(&DriverId::OpenAI, "gpt-5.2").is_some());
        assert!(get_model_profile(&DriverId::AzureOpenAI, "gpt-5.2").is_some());
    }

    #[test]
    fn test_phases_and_tool_search_gated_to_responses_surface() {
        // GPT-5.4 advertises phases + tool_search on the Responses surface...
        let responses = get_model_profile(&DriverId::OpenAI, "gpt-5.4").unwrap();
        assert!(responses.supports_phases);
        assert!(responses.tool_search);
        // ...but not when reached via Chat Completions or Azure.
        let completions = get_model_profile(&DriverId::OpenAICompletions, "gpt-5.4").unwrap();
        assert!(!completions.supports_phases);
        assert!(!completions.tool_search);
    }

    #[test]
    fn test_anthropic_native_tool_search_by_family() {
        // Claude 4-family + Fable advertise Anthropic's hosted tool_search.
        for id in [
            "claude-fable-5",
            "claude-opus-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
            "claude-opus-4",
            "claude-sonnet-4-6",
            "claude-sonnet-4-5",
            "claude-haiku-4-5",
        ] {
            let p = get_model_profile(&DriverId::Anthropic, id)
                .unwrap_or_else(|| panic!("{id} should resolve under anthropic"));
            assert!(p.tool_search, "{id} should advertise native tool_search");
        }
        // The 1M twin inherits the flag.
        assert!(
            get_model_profile(&DriverId::Anthropic, "claude-opus-4-8[1m]")
                .unwrap()
                .tool_search
        );
        // Retired pre-4 Claude models are no longer in the registry at all.
        assert!(get_model_profile(&DriverId::Anthropic, "claude-3-5-haiku").is_none());
        // Reached via a non-first-party transport (Bedrock ConverseStream lacks
        // server-side tool search; OpenRouter's stateless shim doesn't implement
        // it), the same model must not advertise hosted tool_search — it falls
        // back to client-side search via auto_tool_search.
        if let Some(bedrock) = get_model_profile(&DriverId::Bedrock, "claude-opus-4-8") {
            assert!(!bedrock.tool_search);
        }
    }

    #[test]
    fn test_model_vendor_lookup() {
        assert_eq!(
            get_model_vendor(&DriverId::Anthropic, "claude-opus-4-8"),
            Some(ModelVendor::Anthropic)
        );
        assert_eq!(
            get_model_vendor(
                &DriverId::OpenAICompletions,
                "nvidia/nemotron-3-super-120b-a12b"
            ),
            Some(ModelVendor::Nvidia)
        );
        assert_eq!(
            get_model_vendor(&DriverId::OpenAI, "gpt-5.4"),
            Some(ModelVendor::OpenAi)
        );
        assert_eq!(get_model_vendor(&DriverId::OpenAI, "made-up"), None);
    }

    #[test]
    fn test_muse_spark_1_2_profiles_and_tiers() {
        let standard = get_model_profile(&DriverId::Meta, "muse-spark-1.2").unwrap();
        assert_eq!(standard.name, "Muse Spark 1.2");
        assert_eq!(standard.limits.as_ref().unwrap().context, 1_048_576);
        assert!(standard.tool_call);
        assert!(standard.tool_search);
        assert!(standard.structured_output);
        assert!(standard.supports_phases);
        assert_eq!(
            standard.modalities.as_ref().unwrap().input,
            vec![
                Modality::Text,
                Modality::Image,
                Modality::Audio,
                Modality::Video,
                Modality::Pdf,
            ]
        );
        let standard_cost = standard.cost.unwrap();
        assert_eq!(standard_cost.input, 1.25);
        assert_eq!(standard_cost.cache_read, Some(0.15));
        assert_eq!(standard_cost.output, 4.25);

        let contributor = get_model_profile(&DriverId::Meta, "muse-spark-1.2-contributor").unwrap();
        let contributor_cost = contributor.cost.unwrap();
        assert_eq!(contributor_cost.input, 0.10);
        assert_eq!(contributor_cost.cache_read, Some(0.002));
        assert_eq!(contributor_cost.output, 0.20);
        assert!(
            contributor
                .description
                .as_deref()
                .unwrap()
                .contains("may be used to train")
        );
        assert_eq!(
            get_model_profile_key(&DriverId::Meta, "muse-spark-1.2-contributor").as_deref(),
            Some("meta/muse-spark-1.2-contributor")
        );
    }

    #[test]
    fn test_muse_surface_capabilities_are_transport_gated() {
        let direct = get_model_profile(&DriverId::Meta, "muse-spark-1.2").unwrap();
        assert!(direct.supports_phases);
        assert!(direct.tool_search);

        let openrouter = get_model_profile(&DriverId::OpenRouter, "meta/muse-spark-1.2").unwrap();
        assert!(!openrouter.supports_phases);
        assert!(!openrouter.tool_search);

        assert!(get_model_profile(&DriverId::OpenRouter, "muse-spark-1.2-contributor").is_none());
        assert_eq!(
            get_model_vendor(&DriverId::Meta, "muse-spark-1.2"),
            Some(ModelVendor::Meta)
        );
    }

    #[test]
    fn test_registry_canonical_ids_have_profiles() {
        // Every canonical id in the registry must have a profile payload, and
        // every profile must be reachable through the registry under at least
        // one of its surfaces (no orphans on either side).
        for descriptor in REGISTRY {
            let canonical = descriptor.ids[0];
            assert!(
                profile_data(canonical).is_some(),
                "registry id {canonical} has no profile payload"
            );
            let surface = &descriptor.surfaces[0];
            assert!(
                get_model_profile(surface, canonical).is_some(),
                "registry id {canonical} does not resolve under its own surface"
            );
        }
    }

    #[test]
    fn test_third_party_alias_matching_is_case_insensitive() {
        // Lowercased and vendor-prefixed variants all resolve to the same model.
        let cases = [
            ("minimax-m3", "MiniMax-M3"),
            ("minimax/minimax-m3", "MiniMax-M3"),
            ("MiniMax-M3", "MiniMax-M3"),
            ("microsoft/mai-1-preview", "MAI-1-preview"),
            ("MAI-1-PREVIEW", "MAI-1-preview"),
            ("X-AI/Grok-4.3", "Grok 4.3"),
        ];
        for (id, name) in cases {
            let profile = get_model_profile(&DriverId::OpenAICompletions, id)
                .unwrap_or_else(|| panic!("missing profile for {id}"));
            assert_eq!(profile.name, name, "wrong profile for {id}");
        }
    }

    #[test]
    fn test_estimate_cost_usd_known_model() {
        // gpt-5.2 profile: input $1.75/M, output $14.00/M.
        let est = estimate_cost_usd(&DriverId::OpenAI, "gpt-5.2", 1_000_000, 500_000, 0, 0)
            .expect("known model should yield an estimate");
        // 1M input * 1.75 + 0.5M output * 14.00 = 1.75 + 7.00 = 8.75
        assert!((est - 8.75).abs() < 1e-9, "got {est}");
    }

    #[test]
    fn test_estimate_cost_usd_unknown_model_is_none() {
        assert!(estimate_cost_usd(&DriverId::OpenAI, "no-such-model", 100, 50, 0, 0).is_none());
    }

    #[test]
    fn test_estimate_cost_usd_openai_embedding_model() {
        let estimate = estimate_cost_usd(
            &DriverId::OpenAI,
            "text-embedding-3-small",
            1_000_000,
            0,
            0,
            0,
        );

        assert_eq!(estimate, Some(0.02));
        assert_eq!(
            get_model_service_kind(&DriverId::OpenAI, "text-embedding-3-small"),
            ServiceKind::Embeddings
        );
    }

    #[test]
    fn test_estimate_cost_usd_bills_disjoint_buckets() {
        // Disjoint convention: `input_tokens` is non-cached, `cache_read_tokens`
        // additive. gpt-5.2: input $1.75/M, cache_read $0.175/M. 200K non-cached
        // input + 800K cache reads each bill at their own rate.
        let est = estimate_cost_usd(&DriverId::OpenAI, "gpt-5.2", 200_000, 0, 800_000, 0)
            .expect("known model should yield an estimate");
        // 200K * 1.75 + 800K * 0.175 = 0.35 + 0.14 = 0.49.
        assert!((est - 0.49).abs() < 1e-9, "got {est}");
    }

    #[test]
    fn test_estimate_cost_usd_cache_heavy_run_is_cheap() {
        // Regression for EVE-599 / EVE-661: gpt-5.5 (input $5/M, output $30/M,
        // cache_read $0.50/M) on a cache-heavy run. With disjoint buckets the
        // driver reports the non-cached remainder (42K) plus 285K cache reads
        // (87% of the original 327K prompt was cached).
        let input = 42_407;
        let cache_read = 284_672;
        let output = 2_096;
        let est = estimate_cost_usd(&DriverId::OpenAI, "gpt-5.5", input, output, cache_read, 0)
            .expect("known model");
        // 42K*5 + 285K*0.5 + 2K*30 (per M) ≈ $0.42 — far below the ~$1.70 that
        // billing the whole 327K prompt at the full input rate would produce.
        assert!(est < 0.45 && est > 0.39, "est {est}");
    }

    #[test]
    fn test_estimate_cost_usd_applies_tier_to_whole_request() {
        let est = estimate_cost_usd(&DriverId::OpenAI, "gpt-5.6-sol", 300_000, 100_000, 0, 0)
            .expect("known tiered model should yield an estimate");
        // GPT-5.6 Sol charges prompts above 272K input tokens at the tiered
        // rates for the whole request: 300K*$10/M + 100K*$45/M = $7.50.
        assert!((est - 7.50).abs() < 1e-9, "got {est}");
    }

    #[test]
    fn test_estimate_cost_usd_uses_cache_tokens_for_tier_threshold() {
        let est = estimate_cost_usd(
            &DriverId::OpenAI,
            "gpt-5.6-luna",
            42_000,
            100_000,
            260_000,
            0,
        )
        .expect("known tiered model should yield an estimate");
        // The prompt exceeds the 272K tier threshold after cached reads are
        // included, so non-cached input, cached reads, and output use tier rates.
        // 42K*$2/M + 260K*$0.20/M + 100K*$9/M = $1.036.
        assert!((est - 1.036).abs() < 1e-9, "got {est}");
    }

    #[test]
    fn test_estimate_cost_usd_anthropic_cache_is_additive() {
        // Anthropic reports cached tokens separately from `input_tokens`, so a
        // cached read must add cost rather than be subtracted out of the input.
        let model = "claude-haiku-4-5";
        let base = estimate_cost_usd(&DriverId::Anthropic, model, 1_000, 0, 0, 0)
            .expect("known anthropic model");
        let with_cache = estimate_cost_usd(&DriverId::Anthropic, model, 1_000, 0, 5_000, 0)
            .expect("known anthropic model");
        // Adding 5K cache-read tokens on top of 1K input must raise the cost,
        // never lower it (which a subtraction would do).
        assert!(
            with_cache > base,
            "cache-read should be additive for Anthropic: base={base} with_cache={with_cache}"
        );
    }
}
