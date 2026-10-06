mod anthropic_profiles;
// Hardcoded LLM Model Profiles
//
// This module provides model profiles based on models.dev structure.
//
// IMPORTANT: Never guess or extrapolate profile data (pricing, limits, capabilities).
// Always source from https://github.com/sst/models.dev/tree/dev/providers
// and cross-reference with official provider documentation. If a model is not
// yet listed on models.dev, wait until the data is available before adding it.
//
// The registry itself is the curated selection; there is no separate preset list.
// Data source: https://github.com/sst/models.dev/tree/dev/providers
// Cross-referenced with official Anthropic and OpenAI documentation

mod anthropic_capabilities;
mod enumeration;
mod gpt6;
mod model_id_match;
mod speed;

pub use enumeration::*;
pub use speed::estimate_cost_usd_for_speed;
use speed::{speed_flex_only, speed_flex_priority, speed_priority_only};

use crate::model_profile_data::types::{
    CostTier, Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile, ModelVendor,
    ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue, ServiceKind, Verbosity,
    VerbosityConfig, VerbosityValue,
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

/// On/off reasoning for models whose API takes only `none` and `high`
/// (Mistral Large 4). Default: none, matching the API, which answers without
/// thinking when the field is omitted.
fn reasoning_effort_toggle() -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: vec![
            effort(ReasoningEffort::None, "Off"),
            effort(ReasoningEffort::High, "On"),
        ],
        default: ReasoningEffort::None,
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
/// (Fable 5.1, Fable 5, Opus 5.5, Opus 5, Opus 4.8, Opus 4.7, Opus 4.6, Sonnet 5, Sonnet 4.6)
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
    surfaces: &'static [&'static str],
    /// Which provider service this model belongs to.
    /// Pickers filter on it: chat pickers never list realtime models.
    service: ServiceKind,
}

const fn md(
    ids: &'static [&'static str],
    vendor: ModelVendor,
    surfaces: &'static [&'static str],
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
    surfaces: &'static [&'static str],
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
const OPENAI: &[&str] = &["openai", "openrouter", "azure_openai", "openai_completions"];
const OPENAI_COMPAT: &[&str] = &["openai", "openrouter", "openai_completions"];
const ANTHROPIC: &[&str] = &["anthropic"];
const GEMINI: &[&str] = &["gemini"];
const LLMSIM: &[&str] = &["llmsim"];
// Microsoft MAI models are served first-party by the dedicated `Mai` driver
// (Azure AI Foundry) and are also reachable through OpenAI-compatible gateways
// (e.g. OpenRouter). They are never offered through the Azure OpenAI driver,
// which targets OpenAI deployments rather than MAI deployments.
const MICROSOFT_MAI: &[&str] = &["mai", "openai", "openrouter", "openai_completions"];
// Muse is served first-party by Meta Model API and through OpenAI-compatible
// gateways, both tiers alike. The Contributor tier was listed first-party only
// on the assumption that its data-use bargain was a Meta Model API product
// term; it is not. OpenRouter serves `meta/muse-spark-1.3-contributor` and
// passes Meta's tier pricing through unchanged ($0.10/$0.002/$0.20 against the
// standard tier's $1.25/$0.15/$4.25), so the id means the same thing and buys
// the same discount on either route. Surface still gates *capabilities*: a
// gateway route loses phases and tool search, which `profile_data` handles.
const META_MUSE: &[&str] = &["meta", "openai", "openrouter", "openai_completions"];
// Mistral models are served first-party by the `mistral` driver (La Plateforme,
// Chat Completions) and through gateways. Never through the OpenAI Responses
// surface: Mistral does not implement it.
const MISTRAL: &[&str] = &["mistral", "openrouter", "openai_completions"];

static REGISTRY: &[ModelDescriptor] = &[
    md_service(
        &["jev-1.13.0", "jev-1.13", "typesafe/jev-1.13"],
        ModelVendor::TypeSafe,
        &["typesafe", "openrouter"],
        ServiceKind::Decisions,
    ),
    md_service(
        &["jev-latest", "typesafe/jev-latest"],
        ModelVendor::TypeSafe,
        &["typesafe", "openrouter"],
        ServiceKind::Decisions,
    ),
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
    md(&["gpt-6-astra"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-6-sol"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-6-luna"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-6.1-sol"], ModelVendor::OpenAi, OPENAI),
    // Anthropic
    md(&["claude-fable-5-1"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-fable-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-5-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-8"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-7"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-6"], ModelVendor::Anthropic, ANTHROPIC),
    // 1M-context twins. The gateway exposes these `[1m]` ids alongside the
    // 200K base models (e.g. "Opus 4.8" vs "Opus 4.8 (1M)" in the picker); the
    // driver sends the `context-1m` beta header for them. See
    // `anthropic_1m_variant` for how their profiles are derived.
    md(&["claude-fable-5-1[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-fable-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-5-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-8[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-7[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-6[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-5-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(
        &["claude-sonnet-5-5[1m]"],
        ModelVendor::Anthropic,
        ANTHROPIC,
    ),
    md(&["claude-sonnet-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-4-6"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-5"], ModelVendor::Anthropic, ANTHROPIC),
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
        &["muse-spark-1.3", "meta/muse-spark-1.3"],
        ModelVendor::Meta,
        META_MUSE,
    ),
    md(
        &[
            "muse-spark-1.3-contributor",
            "meta/muse-spark-1.3-contributor",
        ],
        ModelVendor::Meta,
        META_MUSE,
    ),
    md(
        &["muse-spark-1.2", "meta/muse-spark-1.2"],
        ModelVendor::Meta,
        META_MUSE,
    ),
    md(
        &[
            "muse-spark-1.2-contributor",
            "meta/muse-spark-1.2-contributor",
        ],
        ModelVendor::Meta,
        META_MUSE,
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
        &[
            "mistral-large-4",
            "mistral-large-4-0",
            "mistralai/mistral-large-4-0",
            "mistralai/mistral-large-4",
            "mistral/mistral-large-4",
        ],
        ModelVendor::Mistral,
        MISTRAL,
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
fn resolve_descriptor(provider_type: &str, model_id: &str) -> Option<&'static ModelDescriptor> {
    // Match without allocating: compare bytes case-insensitively. Ids are ASCII.
    let id = model_id.as_bytes();
    let mut longest_match = 0;
    let mut best_for_surface: Option<&'static ModelDescriptor> = None;
    for descriptor in REGISTRY {
        for alias in descriptor.ids {
            let alias = alias.as_bytes();
            if !model_id_match::matches_alias(id, alias) {
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
                    .contains(&provider_type)
                    .then_some(descriptor);
            } else if alias.len() == longest_match && descriptor.surfaces.contains(&provider_type) {
                best_for_surface = Some(descriptor);
            }
        }
    }
    best_for_surface
}

/// Get a model profile by matching provider_type and model_id.
/// Returns None if the id is not in the registry or is not offered under the
/// given provider type.
pub fn get_model_profile(provider_type: &str, model_id: &str) -> Option<ModelProfile> {
    let descriptor = resolve_descriptor(provider_type, model_id)?;
    let mut profile = profile_data(descriptor.ids[0])?;
    // Native execution phases are implemented by the first-party OpenAI and
    // Meta Responses surfaces. Gateways keep the base model profile while
    // masking provider-native request options.
    if provider_type != "openai" && provider_type != "meta" {
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
    if provider_type != "openai" && provider_type != "anthropic" && provider_type != "meta" {
        profile.tool_search = false;
    }
    if provider_type != "anthropic" {
        profile.supports_server_compaction = false;
    }
    // Speed (service tier) is an OpenAI-platform billing feature. Azure has
    // its own capacity model and gateways (OpenRouter) do their own routing,
    // so only the first-party OpenAI surface keeps the selector.
    if provider_type != "openai" {
        profile.speed = None;
    }
    // Verbosity (`text.verbosity` / `verbosity`) is an OpenAI-specific request
    // parameter. Gateways that proxy these models may reject the unknown field,
    // so only the first-party OpenAI surface keeps the selector.
    if provider_type != "openai" {
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
/// its own rate, falling back to the applicable input rate when no cache-write
/// price is known.
pub fn estimate_cost_usd(
    provider_type: &str,
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
    let cache_write_rate = match active_tier {
        Some(tier) => tier.cache_write.unwrap_or(input_rate),
        None => cost.cache_write.unwrap_or(input_rate),
    };
    let per_million = |tokens: u32, rate: f64| (tokens as f64 / 1_000_000.0) * rate;

    Some(
        per_million(input_tokens, input_rate)
            + per_million(cache_read_tokens, cache_read_rate)
            + per_million(cache_creation_tokens, cache_write_rate)
            + per_million(output_tokens, output_rate),
    )
}

/// Get the vendor/brand for a model id, or None if it is not in the registry
/// (or not offered under the given provider type).
pub fn get_model_vendor(provider_type: &str, model_id: &str) -> Option<ModelVendor> {
    resolve_descriptor(provider_type, model_id).map(|descriptor| descriptor.vendor)
}

/// Stable public profile key: `"{vendor}/{canonical_id}"`.
///
/// The key identifies the model's identity independent of which provider
/// serves it: `("anthropic", "claude-sonnet-4-6-20260217")` and a gateway
/// alias of the same model both map to `"anthropic/claude-sonnet-4-6"`.
pub fn get_model_profile_key(provider_type: &str, model_id: &str) -> Option<String> {
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
pub fn get_model_service_kind(provider_type: &str, model_id: &str) -> ServiceKind {
    resolve_descriptor(provider_type, model_id)
        .map(|descriptor| descriptor.service)
        .unwrap_or(ServiceKind::Chat)
}

/// Profile payload keyed by canonical model id. Pure value store: provider
/// availability and vendor tagging live in `REGISTRY`, not here. The segments
/// are grouped by author for readability only — there is no provider dispatch.
fn profile_data(canonical: &str) -> Option<ModelProfile> {
    jev_profile_data(canonical)
        .or_else(|| openai_profile_data(canonical))
        .or_else(|| anthropic_profile_data(canonical))
        .or_else(|| gemini_profile_data(canonical))
        .or_else(|| meta_profile_data(canonical))
        .or_else(|| third_party_profile_data(canonical))
        .or_else(|| llmsim_profile_data(canonical))
}

mod meta_profiles;
use meta_profiles::meta_profile_data;

mod openai_profiles;
use openai_profiles::openai_profile_data;

mod third_party_profiles;
use third_party_profiles::third_party_profile_data;

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

fn anthropic_profile_data(model_id: &str) -> Option<ModelProfile> {
    // Assign family capabilities centrally; match arms hold model-specific data.
    anthropic_profiles::anthropic_profile_data_inner(model_id).map(|mut profile| {
        profile.tool_search = anthropic_capabilities::supports_tool_search(&profile.family);
        anthropic_capabilities::apply(&mut profile);
        profile
    })
}

mod gemini_profiles;
use gemini_profiles::gemini_profile_data;

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
                cache_write: None,
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
            supports_server_compaction: false,
            decisions: None,
        }),
        _ => None,
    }
}

#[cfg(test)]
#[path = "profiles_claude_tests.rs"]
mod claude_tests;

#[cfg(test)]
mod tests;

fn jev_profile_data(canonical: &str) -> Option<ModelProfile> {
    if !matches!(canonical, "jev-1.13.0" | "jev-latest") {
        return None;
    }
    Some(ModelProfile {
        name: if canonical == "jev-latest" {
            "Jev Latest"
        } else {
            "Jev 1.13"
        }
        .into(),
        family: "jev".into(),
        description: Some("Typed calibrated decisions over text state.".into()),
        release_date: None,
        last_updated: None,
        attachment: false,
        reasoning: false,
        temperature: false,
        knowledge: None,
        tool_call: false,
        structured_output: true,
        open_weights: false,
        cost: Some(crate::model_profile_data::ModelCost::new(0.042, 0.0)),
        limits: None,
        modalities: None,
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        tool_search: false,
        supported_parameters: Vec::new(),
        supports_phases: false,
        supports_server_compaction: false,
        decisions: Some(crate::model_profile_data::DecisionModelProfile {
            primitives: vec!["noul".into(), "choice".into(), "score".into()],
            calibrated: true,
            max_choice_options: Some(255),
            max_score_levels: Some(10),
            state_tokens: Some(32_000),
            request_tokens: Some(64_000),
        }),
    })
}
