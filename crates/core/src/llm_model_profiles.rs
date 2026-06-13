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

use crate::llm_driver_registry::ServiceKind;
use crate::llm_models::{
    CostTier, LlmModelCost, LlmModelLimits, LlmModelModalities, LlmModelProfile, LlmProviderType,
    Modality, ModelVendor, ReasoningEffort, ReasoningEffortConfig, ReasoningEffortValue,
};

// Helper functions for creating reasoning effort configurations

fn effort(value: ReasoningEffort, name: &str) -> ReasoningEffortValue {
    ReasoningEffortValue {
        value,
        name: name.into(),
    }
}

/// Standard reasoning efforts for pre-gpt-5.1 models (o1, o1-mini, o3-mini)
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

/// Reasoning effort for o1-pro (only high)
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

/// Reasoning effort for gpt-5.5
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
/// (Fable 5, Opus 4.8, Opus 4.7, Opus 4.6, Sonnet 4.6)
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
    surfaces: &'static [LlmProviderType],
    /// Which provider service this model belongs to (specs/providers.md).
    /// Pickers filter on it: chat pickers never list realtime models.
    service: ServiceKind,
}

const fn md(
    ids: &'static [&'static str],
    vendor: ModelVendor,
    surfaces: &'static [LlmProviderType],
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
    surfaces: &'static [LlmProviderType],
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
const OPENAI: &[LlmProviderType] = &[
    LlmProviderType::Openai,
    LlmProviderType::Openrouter,
    LlmProviderType::AzureOpenai,
    LlmProviderType::OpenaiCompletions,
];
const OPENAI_COMPAT: &[LlmProviderType] = &[
    LlmProviderType::Openai,
    LlmProviderType::Openrouter,
    LlmProviderType::OpenaiCompletions,
];
const ANTHROPIC: &[LlmProviderType] = &[LlmProviderType::Anthropic];
const GEMINI: &[LlmProviderType] = &[LlmProviderType::Gemini];
const LLMSIM: &[LlmProviderType] = &[LlmProviderType::LlmSim];

static REGISTRY: &[ModelDescriptor] = &[
    // OpenAI
    md_service(
        &["gpt-realtime-2"],
        ModelVendor::OpenAi,
        OPENAI,
        ServiceKind::Realtime,
    ),
    md(&["gpt-4o"], ModelVendor::OpenAi, OPENAI),
    md(&["gpt-4o-mini"], ModelVendor::OpenAi, OPENAI),
    md(&["o1"], ModelVendor::OpenAi, OPENAI),
    md(&["o1-mini"], ModelVendor::OpenAi, OPENAI),
    md(&["o1-pro"], ModelVendor::OpenAi, OPENAI),
    md(&["o1-preview"], ModelVendor::OpenAi, OPENAI),
    md(&["o3"], ModelVendor::OpenAi, OPENAI),
    md(&["o3-mini"], ModelVendor::OpenAi, OPENAI),
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
    // Anthropic
    md(&["claude-fable-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-8"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-7"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-6"], ModelVendor::Anthropic, ANTHROPIC),
    // 1M-context twins. The gateway exposes these `[1m]` ids alongside the
    // 200K base models (e.g. "Opus 4.8" vs "Opus 4.8 (1M)" in the picker); the
    // driver sends the `context-1m` beta header for them. See
    // `anthropic_1m_variant` for how their profiles are derived.
    md(&["claude-fable-5[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-8[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-7[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-6[1m]"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-4-6"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-4-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-haiku-4-5"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4-1"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-opus-4"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-sonnet-4"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-3-7-sonnet"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-3-5-sonnet"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-3-5-haiku"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-3-opus"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-3-sonnet"], ModelVendor::Anthropic, ANTHROPIC),
    md(&["claude-3-haiku"], ModelVendor::Anthropic, ANTHROPIC),
    // Google Gemini
    md(&["gemini-3.1-pro-preview"], ModelVendor::Google, GEMINI),
    md(&["gemini-2.5-pro"], ModelVendor::Google, GEMINI),
    md(&["gemini-2.5-flash"], ModelVendor::Google, GEMINI),
    md(&["gemini-2.0-flash"], ModelVendor::Google, GEMINI),
    md(&["gemini-1.5-pro"], ModelVendor::Google, GEMINI),
    md(&["gemini-1.5-flash"], ModelVendor::Google, GEMINI),
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
        OPENAI_COMPAT,
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
/// `gpt-4o-mini` over `gpt-4o`).
fn resolve_descriptor(
    provider_type: &LlmProviderType,
    model_id: &str,
) -> Option<&'static ModelDescriptor> {
    // Match without allocating: compare bytes case-insensitively. Ids are ASCII.
    let id = model_id.as_bytes();
    let mut best: Option<(usize, &'static ModelDescriptor)> = None;
    for descriptor in REGISTRY {
        if !descriptor.surfaces.contains(provider_type) {
            continue;
        }
        for alias in descriptor.ids {
            let alias = alias.as_bytes();
            // Exact (case-insensitive) match, or an `"<alias>-"` prefix (which
            // covers dated and `-latest` suffixes).
            let id_matches = if id.len() == alias.len() {
                id.eq_ignore_ascii_case(alias)
            } else {
                id.len() > alias.len()
                    && id[alias.len()] == b'-'
                    && id[..alias.len()].eq_ignore_ascii_case(alias)
            };
            if id_matches && best.is_none_or(|(len, _)| alias.len() > len) {
                best = Some((alias.len(), descriptor));
            }
        }
    }
    best.map(|(_, descriptor)| descriptor)
}

/// Get a model profile by matching provider_type and model_id.
/// Returns None if the id is not in the registry or is not offered under the
/// given provider type.
pub fn get_model_profile(
    provider_type: &LlmProviderType,
    model_id: &str,
) -> Option<LlmModelProfile> {
    let descriptor = resolve_descriptor(provider_type, model_id)?;
    let mut profile = profile_data(descriptor.ids[0])?;
    // Native execution phases are an OpenAI Responses-only feature. OpenRouter
    // uses a compatible but stateless Responses endpoint, so it keeps the base
    // model profile while masking native OpenAI-only request options.
    if !matches!(provider_type, LlmProviderType::Openai) {
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
    // So mask the flag for everything except the two first-party providers.
    if !matches!(
        provider_type,
        LlmProviderType::Openai | LlmProviderType::Anthropic
    ) {
        profile.tool_search = false;
    }
    Some(profile)
}

/// Estimate the USD cost of a generation from the model's static price-table
/// profile. Returns `None` when there is no profile or no cost data for the
/// model — callers then have no estimate to record and fall back accordingly.
///
/// This is the price-table fallback used when a provider does not report an
/// authoritative cost inline (e.g. OpenRouter's `usage.cost`). Cost figures in
/// profiles are per million tokens. Cache tokens are not discounted here,
/// matching existing budget metering behavior.
pub fn estimate_cost_usd(
    provider_type: &LlmProviderType,
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
) -> Option<f64> {
    let cost = get_model_profile(provider_type, model_id)?.cost?;
    let input_cost = (input_tokens as f64 / 1_000_000.0) * cost.input;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * cost.output;
    Some(input_cost + output_cost)
}

/// Get the vendor/brand for a model id, or None if it is not in the registry
/// (or not offered under the given provider type).
pub fn get_model_vendor(provider_type: &LlmProviderType, model_id: &str) -> Option<ModelVendor> {
    resolve_descriptor(provider_type, model_id).map(|descriptor| descriptor.vendor)
}

/// Stable public profile key: `"{vendor}/{canonical_id}"` (specs/providers.md).
///
/// The key identifies the model's identity independent of which provider
/// serves it: `("anthropic", "claude-sonnet-4-5-20250929")` and a gateway
/// alias of the same model both map to `"anthropic/claude-sonnet-4-5"`.
pub fn get_model_profile_key(provider_type: &LlmProviderType, model_id: &str) -> Option<String> {
    resolve_descriptor(provider_type, model_id)
        .map(|descriptor| format!("{}/{}", descriptor.vendor.slug(), descriptor.ids[0]))
}

/// Look up a profile by its stable key (`"{vendor}/{canonical_id}"`).
///
/// Key lookup is provider-independent, so the returned profile is the base
/// payload without provider-surface masking (`supports_phases`/`tool_search`
/// stay as authored). Use [`get_model_profile`] when resolving for a concrete
/// provider.
pub fn get_model_profile_by_key(key: &str) -> Option<LlmModelProfile> {
    let (vendor_slug, canonical) = key.split_once('/')?;
    let descriptor = REGISTRY.iter().find(|descriptor| {
        descriptor.vendor.slug().eq_ignore_ascii_case(vendor_slug)
            && descriptor.ids[0].eq_ignore_ascii_case(canonical)
    })?;
    profile_data(descriptor.ids[0])
}

/// Which provider service a model belongs to. Unknown models default to
/// [`ServiceKind::Chat`].
pub fn get_model_service_kind(provider_type: &LlmProviderType, model_id: &str) -> ServiceKind {
    resolve_descriptor(provider_type, model_id)
        .map(|descriptor| descriptor.service)
        .unwrap_or(ServiceKind::Chat)
}

/// Profile payload keyed by canonical model id. Pure value store: provider
/// availability and vendor tagging live in `REGISTRY`, not here. The segments
/// are grouped by author for readability only — there is no provider dispatch.
fn profile_data(canonical: &str) -> Option<LlmModelProfile> {
    openai_profile_data(canonical)
        .or_else(|| anthropic_profile_data(canonical))
        .or_else(|| gemini_profile_data(canonical))
        .or_else(|| third_party_profile_data(canonical))
        .or_else(|| llmsim_profile_data(canonical))
}

fn openai_profile_data(model_id: &str) -> Option<LlmModelProfile> {
    match model_id {
        "gpt-realtime-2" => Some(LlmModelProfile {
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
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Audio],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: Some(reasoning_effort_realtime()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-4o" => Some(LlmModelProfile {
            name: "GPT-4o".into(),
            family: "gpt-4o".into(),
            description: None,
            release_date: Some("2024-05-13".into()),
            last_updated: Some("2024-11-20".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 2.50,
                output: 10.00,
                cache_read: Some(1.25),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Audio],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-4o-mini" => Some(LlmModelProfile {
            name: "GPT-4o mini".into(),
            family: "gpt-4o-mini".into(),
            description: None,
            release_date: Some("2024-07-18".into()),
            last_updated: Some("2024-07-18".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.15,
                output: 0.60,
                cache_read: Some(0.075),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o1" => Some(LlmModelProfile {
            name: "o1".into(),
            family: "o1".into(),
            description: None,
            release_date: Some("2024-12-17".into()),
            last_updated: Some("2024-12-17".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 60.00,
                cache_read: Some(7.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o1-mini" => Some(LlmModelProfile {
            name: "o1-mini".into(),
            family: "o1-mini".into(),
            description: None,
            release_date: Some("2024-09-12".into()),
            last_updated: Some("2024-09-12".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: false,
            structured_output: false,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 12.00,
                cache_read: Some(1.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o1-pro" => Some(LlmModelProfile {
            name: "o1-pro".into(),
            family: "o1-pro".into(),
            description: None,
            release_date: Some("2025-03-19".into()),
            last_updated: Some("2025-03-19".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 150.00,
                output: 600.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o3-mini" => Some(LlmModelProfile {
            name: "o3-mini".into(),
            family: "o3-mini".into(),
            description: None,
            release_date: Some("2025-01-31".into()),
            last_updated: Some("2025-01-31".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o3" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o3-pro" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 20.00,
                output: 80.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o4-mini" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-4.1 family models
        "gpt-4.1" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-4.1-mini" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.40,
                output: 1.60,
                cache_read: Some(0.20),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-4.1-nano" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.10,
                output: 0.40,
                cache_read: Some(0.05),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5 family models
        // Pre-5.1 models: default medium, supports low/medium/high (no none)
        "gpt-5" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-mini" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.25,
                output: 2.00,
                cache_read: Some(0.025),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-nano" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.05,
                output: 0.40,
                cache_read: Some(0.005),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-pro" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 60.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_high_only()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5-codex" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.1 models: default none, supports none/low/medium/high
        "gpt-5.1" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.1-codex" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.1-codex-mini" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.30,
                output: 2.40,
                cache_read: Some(0.03),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.1-codex-max and after: supports xhigh
        "gpt-5.1-codex-max" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 24.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.2 models: supports xhigh, 400K context
        "gpt-5.2" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.2-pro" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 21.00,
                output: 168.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.2-codex" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.3 Codex: same pricing as 5.2, 25% faster inference
        "gpt-5.3-codex" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // GPT-5.5 family: latest flagship reasoning models. Released 2026-04-23.
        // Flat pricing (no 200K context tiers, unlike 5.4).
        // Source: developers.openai.com/api/docs/models/gpt-5.5 and .../gpt-5.5-pro
        // (models.dev did not yet list these variants at the time of addition;
        // refresh once the models.dev entry appears).
        "gpt-5.5" => Some(LlmModelProfile {
            name: "GPT-5.5".into(),
            family: "gpt-5.5".into(),
            description: Some("Latest flagship reasoning model. Best for complex multi-step tasks, code generation, and deep analysis.".into()),
            release_date: Some("2026-04-23".into()),
            last_updated: Some("2026-04-23".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2025-12-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 30.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt55()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.5-pro" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 30.00,
                output: 180.00,
                cache_read: None,
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        // GPT-5.4 family: reasoning models with 1.05M context, tool_search, native phases.
        // Released 2026-03-05 (5.4, 5.4-pro), 2026-03-17 (5.4-mini, 5.4-nano).
        // 5.4 and 5.4-pro have tiered pricing above 200K context tokens.
        "gpt-5.4" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
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
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.4-mini" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.75,
                output: 4.50,
                cache_read: Some(0.075),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.4-nano" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.20,
                output: 1.25,
                cache_read: Some(0.02),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 400_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        "gpt-5.4-pro" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
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
            limits: Some(LlmModelLimits {
                context: 1_050_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52_pro()),
            tool_search: true,
            supported_parameters: Vec::new(),
            supports_phases: true,
        }),

        // GPT-5 chat-latest models (point to latest chat-optimized versions)
        "gpt-5-chat-latest" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.125),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt5_pre51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.1-chat-latest" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.50,
                output: 12.00,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt51()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gpt-5.2-chat-latest" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.75,
                output: 14.00,
                cache_read: Some(0.175),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 16_384,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Deep research models
        "o3-deep-research" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 2.00,
                output: 8.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o4-mini-deep-research" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.10,
                output: 4.40,
                cache_read: Some(0.55),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 100_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "o1-preview" => Some(LlmModelProfile {
            name: "o1 Preview".into(),
            family: "o1".into(),
            description: None,
            release_date: Some("2024-09-12".into()),
            last_updated: Some("2024-09-12".into()),
            attachment: false,
            reasoning: true,
            temperature: true,
            knowledge: Some("2023-10-01".into()),
            tool_call: false,
            structured_output: false,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 60.00,
                cache_read: Some(7.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 32_768,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_standard()),
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
fn third_party_profile_data(model_id: &str) -> Option<LlmModelProfile> {
    match model_id.to_ascii_lowercase().as_str() {
        // NVIDIA Nemotron 3 Super — flagship Nemotron reasoning model.
        // Source: models.dev (nvidia provider).
        "nemotron-3-super-120b-a12b" | "nvidia/nemotron-3-super-120b-a12b" => {
            Some(LlmModelProfile {
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
                cost: Some(LlmModelCost {
                    input: 0.20,
                    output: 0.80,
                    cache_read: None,
                    cost_tiers: vec![],
                }),
                limits: Some(LlmModelLimits {
                    context: 262_144,
                    input: None,
                    output: 262_144,
                    max_media: None,
                }),
                modalities: Some(LlmModelModalities {
                    input: vec![Modality::Text],
                    output: vec![Modality::Text],
                }),
                reasoning_effort: None,
                tool_search: false,
                supported_parameters: Vec::new(),
                supports_phases: false,
            })
        }

        // Alibaba Qwen3.7 Max — flagship Qwen model.
        // Source: models.dev (alibaba provider). Knowledge cutoff not published.
        "qwen3.7-max" | "qwen/qwen3.7-max" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 2.50,
                output: 7.50,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_000_000,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
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
        "mai-1-preview" | "microsoft/mai-1-preview" => Some(LlmModelProfile {
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
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // MiniMax-M3 — flagship MiniMax model. Source: models.dev (minimax
        // provider). Reasoning is a toggle upstream (no graded effort).
        "minimax-m3" | "minimax/minimax-m3" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.60,
                output: 2.40,
                cache_read: Some(0.12),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 512_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Video],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Moonshot Kimi K2 Thinking — flagship Kimi reasoning model.
        // Source: models.dev (moonshotai provider).
        "kimi-k2-thinking" | "moonshotai/kimi-k2-thinking" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.60,
                output: 2.50,
                cache_read: Some(0.15),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 262_144,
                input: None,
                output: 262_144,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // xAI Grok 4.3 — flagship Grok model. Source: models.dev (xai provider).
        // Pricing has a >200K-token tier. Knowledge cutoff not published.
        "grok-4.3" | "x-ai/grok-4.3" | "xai/grok-4.3" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
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
            limits: Some(LlmModelLimits {
                context: 1_000_000,
                input: None,
                output: 30_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
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
fn anthropic_1m_variant(mut profile: LlmModelProfile) -> LlmModelProfile {
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
            | "claude-opus-4-8"
            | "claude-opus-4-7"
            | "claude-opus-4-6"
            | "claude-opus-4-5"
            | "claude-opus-4-1"
            | "claude-opus-4"
            | "claude-sonnet-4-6"
            | "claude-sonnet-4-5"
            | "claude-sonnet-4"
            | "claude-haiku-4-5"
    )
}

fn anthropic_profile_data(model_id: &str) -> Option<LlmModelProfile> {
    // `tool_search` is assigned centrally by family below, so the per-literal
    // `tool_search` value in the match arms is a placeholder and is overwritten.
    anthropic_profile_data_inner(model_id).map(|mut profile| {
        profile.tool_search = anthropic_family_supports_tool_search(&profile.family);
        profile
    })
}

fn anthropic_profile_data_inner(model_id: &str) -> Option<LlmModelProfile> {
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
        "claude-fable-5" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 10.00,
                output: 50.00,
                cache_read: Some(1.00),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                // Bare id is the 200K profile; `claude-fable-5[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
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
        "claude-opus-4-8" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                // Bare id is the 200K profile; `claude-opus-4-8[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.7 series
        // Sampling parameters were removed starting with Opus 4.7: the API
        // rejects `temperature` with "`temperature` is deprecated for this
        // model" (verified live), hence `temperature: false`.
        "claude-opus-4-7" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                // Bare id is the 200K profile; `claude-opus-4-7[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image, Modality::Pdf],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.6 series
        "claude-opus-4-6" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                // Bare id is the 200K profile; `claude-opus-4-6[1m]` is the 1M twin.
                context: 200_000,
                input: None,
                output: 128_000,
                max_media: Some(600),
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // 1M-context twins of the base profiles above. Same pricing and
        // capabilities; only the context limit and display name differ.
        "claude-fable-5[1m]" => anthropic_profile_data("claude-fable-5").map(anthropic_1m_variant),
        "claude-opus-4-8[1m]" => {
            anthropic_profile_data("claude-opus-4-8").map(anthropic_1m_variant)
        }
        "claude-opus-4-7[1m]" => {
            anthropic_profile_data("claude-opus-4-7").map(anthropic_1m_variant)
        }
        "claude-opus-4-6[1m]" => {
            anthropic_profile_data("claude-opus-4-6").map(anthropic_1m_variant)
        }

        "claude-sonnet-4-6" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_adaptive_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.5 series
        "claude-opus-4-5" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 5.00,
                output: 25.00,
                cache_read: Some(0.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-sonnet-4-5" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-haiku-4-5" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.00,
                output: 5.00,
                cache_read: Some(0.10),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 16_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4.1 series
        "claude-opus-4-1" => Some(LlmModelProfile {
            name: "Claude Opus 4.1".into(),
            family: "claude-opus-4-1".into(),
            description: None,
            release_date: Some("2025-08-05".into()),
            last_updated: Some("2025-08-05".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-03-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 75.00,
                cache_read: Some(1.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 32_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 4 series
        "claude-sonnet-4" => Some(LlmModelProfile {
            name: "Claude Sonnet 4".into(),
            family: "claude-sonnet-4".into(),
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
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-opus-4" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 75.00,
                cache_read: Some(1.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 32_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 3.7 series
        "claude-3-7-sonnet" => Some(LlmModelProfile {
            name: "Claude 3.7 Sonnet".into(),
            family: "claude-3-7-sonnet".into(),
            description: None,
            release_date: Some("2025-02-19".into()),
            last_updated: Some("2025-02-19".into()),
            attachment: true,
            reasoning: true, // Extended thinking mode
            temperature: true,
            knowledge: Some("2024-11-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 64_000, // Extended output with thinking
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_anthropic_extended_thinking()),
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        // Claude 3.5 series
        "claude-3-5-sonnet" => Some(LlmModelProfile {
            name: "Claude 3.5 Sonnet".into(),
            family: "claude-3-5-sonnet".into(),
            description: None,
            release_date: Some("2024-06-20".into()),
            last_updated: Some("2024-10-22".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-3-5-haiku" => Some(LlmModelProfile {
            name: "Claude 3.5 Haiku".into(),
            family: "claude-3-5-haiku".into(),
            description: None,
            release_date: Some("2024-10-22".into()),
            last_updated: Some("2024-10-22".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-07-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.00,
                output: 5.00,
                cache_read: Some(0.10),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-3-opus" => Some(LlmModelProfile {
            name: "Claude 3 Opus".into(),
            family: "claude-3-opus".into(),
            description: None,
            release_date: Some("2024-02-29".into()),
            last_updated: Some("2024-02-29".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 15.00,
                output: 75.00,
                cache_read: Some(1.50),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 4_096,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-3-sonnet" => Some(LlmModelProfile {
            name: "Claude 3 Sonnet".into(),
            family: "claude-3-sonnet".into(),
            description: None,
            release_date: Some("2024-02-29".into()),
            last_updated: Some("2024-02-29".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 3.00,
                output: 15.00,
                cache_read: Some(0.30),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 4_096,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "claude-3-haiku" => Some(LlmModelProfile {
            name: "Claude 3 Haiku".into(),
            family: "claude-3-haiku".into(),
            description: None,
            release_date: Some("2024-03-07".into()),
            last_updated: Some("2024-03-07".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2023-08-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.25,
                output: 1.25,
                cache_read: Some(0.03),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 200_000,
                input: None,
                output: 4_096,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        _ => None,
    }
}

fn gemini_profile_data(model_id: &str) -> Option<LlmModelProfile> {
    match model_id {
        // Gemini 3.x series (newest). Source: models.dev (google provider).
        // `gemini-3-pro-preview` is deprecated upstream; 3.1 Pro Preview is the
        // current flagship Pro. Pricing has a >200K-token tier. Reasoning effort
        // (low/medium/high) is offered upstream but, consistent with the other
        // Gemini profiles here, effort selection is left unset.
        "gemini-3.1-pro-preview" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
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
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
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
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-2.5-pro" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 10.00,
                cache_read: Some(0.31),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-2.5-flash" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.15,
                output: 0.60,
                cache_read: Some(0.0375),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 65_536,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-2.0-flash" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.10,
                output: 0.40,
                cache_read: Some(0.025),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text, Modality::Image],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-1.5-pro" => Some(LlmModelProfile {
            name: "Gemini 1.5 Pro".into(),
            family: "gemini-1.5-pro".into(),
            description: None,
            release_date: Some("2024-02-15".into()),
            last_updated: Some("2024-09-24".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 1.25,
                output: 5.00,
                cache_read: Some(0.31),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 2_097_152,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        "gemini-1.5-flash" => Some(LlmModelProfile {
            name: "Gemini 1.5 Flash".into(),
            family: "gemini-1.5-flash".into(),
            description: None,
            release_date: Some("2024-05-24".into()),
            last_updated: Some("2024-09-24".into()),
            attachment: true,
            reasoning: false,
            temperature: true,
            knowledge: Some("2024-04-01".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(LlmModelCost {
                input: 0.075,
                output: 0.30,
                cache_read: Some(0.01875),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 1_048_576,
                input: None,
                output: 8_192,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
                    Modality::Audio,
                    Modality::Video,
                ],
                output: vec![Modality::Text],
            }),
            reasoning_effort: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
        }),

        _ => None,
    }
}

/// Get LlmSim model profile (simulated LLM for testing)
/// Profile is modeled close to GPT-5.2 for realistic testing
fn llmsim_profile_data(model_id: &str) -> Option<LlmModelProfile> {
    match model_id {
        "llmsim-default" | "llmsim" => Some(LlmModelProfile {
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
            cost: Some(LlmModelCost {
                input: 0.00, // Free for testing
                output: 0.00,
                cache_read: Some(0.00),
                cost_tiers: vec![],
            }),
            limits: Some(LlmModelLimits {
                context: 128_000,
                input: None,
                output: 64_000,
                max_media: None,
            }),
            modalities: Some(LlmModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_gpt52()), // Same as GPT-5.2
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
        resolve_descriptor(&LlmProviderType::Openai, id)
            .map(|d| d.ids[0])
            .unwrap_or("")
    }
    fn normalize_anthropic_model_id(id: &str) -> &'static str {
        resolve_descriptor(&LlmProviderType::Anthropic, id)
            .map(|d| d.ids[0])
            .unwrap_or("")
    }
    fn normalize_gemini_model_id(id: &str) -> &'static str {
        resolve_descriptor(&LlmProviderType::Gemini, id)
            .map(|d| d.ids[0])
            .unwrap_or("")
    }

    #[test]
    fn test_profile_keys_and_service_kinds() {
        // Canonical key from a dated wire id (version-suffix normalization).
        assert_eq!(
            get_model_profile_key(&LlmProviderType::Anthropic, "claude-sonnet-4-5-20250929")
                .as_deref(),
            Some("anthropic/claude-sonnet-4-5")
        );
        // Gateway alias and bare id share one key (same model identity).
        assert_eq!(
            get_model_profile_key(
                &LlmProviderType::Openrouter,
                "nvidia/nemotron-3-super-120b-a12b"
            ),
            get_model_profile_key(&LlmProviderType::Openai, "nemotron-3-super-120b-a12b"),
        );
        // Unknown models have no key.
        assert_eq!(
            get_model_profile_key(&LlmProviderType::Openai, "not-a-model"),
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
            get_model_service_kind(&LlmProviderType::Openai, "gpt-realtime-2"),
            ServiceKind::Realtime
        );
        assert_eq!(
            get_model_service_kind(&LlmProviderType::Openai, "gpt-5.5"),
            ServiceKind::Chat
        );
        // Unknown models default to chat.
        assert_eq!(
            get_model_service_kind(&LlmProviderType::Openai, "not-a-model"),
            ServiceKind::Chat
        );
    }

    #[test]
    fn test_get_profile_openai_gpt4o() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4o");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "GPT-4o");
        assert_eq!(profile.family, "gpt-4o");
        assert!(profile.tool_call);
        assert!(profile.structured_output);
    }

    #[test]
    fn test_get_profile_openai_gpt4o_versioned() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4o-2024-11-20");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "GPT-4o");
    }

    #[test]
    fn test_get_profile_anthropic_claude35_sonnet() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-3-5-sonnet-20241022");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "Claude 3.5 Sonnet");
        assert!(profile.tool_call);
    }

    #[test]
    fn test_get_profile_anthropic_claude_sonnet4() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-20250514");
        assert!(profile.is_some());
        let profile = profile.unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4");
    }

    #[test]
    fn test_get_profile_unknown_model() {
        let profile = get_model_profile(&LlmProviderType::Openai, "unknown-model");
        assert!(profile.is_none());
    }

    #[test]
    fn test_get_profile_wrong_provider() {
        // Try to get an OpenAI model with Anthropic provider
        let profile = get_model_profile(&LlmProviderType::Anthropic, "gpt-4o");
        assert!(profile.is_none());
    }

    #[test]
    fn test_profile_has_cost_and_limits() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4o").unwrap();
        assert!(profile.cost.is_some());
        assert!(profile.limits.is_some());

        let cost = profile.cost.unwrap();
        assert!(cost.input > 0.0);
        assert!(cost.output > 0.0);

        let limits = profile.limits.unwrap();
        assert!(limits.context > 0);
        assert!(limits.output > 0);
    }

    #[test]
    fn test_o1_has_reasoning() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o1").unwrap();
        assert!(profile.reasoning);
    }

    #[test]
    fn test_claude_opus_4_has_reasoning() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4").unwrap();
        assert!(profile.reasoning);
    }

    #[test]
    fn test_normalize_openai_model_id() {
        assert_eq!(normalize_model_id("gpt-4o"), "gpt-4o");
        assert_eq!(normalize_model_id("gpt-4o-2024-11-20"), "gpt-4o");
        assert_eq!(normalize_model_id("gpt-4o-mini"), "gpt-4o-mini");
        assert_eq!(normalize_model_id("o1-2024-12-17"), "o1");
        assert_eq!(normalize_model_id("o1-mini"), "o1-mini");
    }

    #[test]
    fn test_normalize_anthropic_model_id() {
        assert_eq!(
            normalize_anthropic_model_id("claude-3-5-sonnet"),
            "claude-3-5-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-3-5-sonnet-20241022"),
            "claude-3-5-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-3-5-sonnet-latest"),
            "claude-3-5-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-sonnet-4-20250514"),
            "claude-sonnet-4"
        );
    }

    #[test]
    fn test_openai_completions_uses_openai_profiles() {
        let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, "gpt-4o");
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "GPT-4o");
    }

    #[test]
    fn test_azure_openai_uses_openai_profiles() {
        let profile = get_model_profile(&LlmProviderType::AzureOpenai, "gpt-4o");
        assert!(profile.is_some());
        assert_eq!(profile.unwrap().name, "GPT-4o");
    }

    // GPT-5 model tests

    #[test]
    fn test_gpt5_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5-mini").unwrap();
        assert_eq!(profile.name, "GPT-5 mini");
        assert!(profile.reasoning);

        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    #[test]
    fn test_gpt5_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5-pro").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.1").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.1-codex-max").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.2").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.2-pro").unwrap();
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
    fn test_gpt53_codex_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.3-codex").unwrap();
        assert_eq!(profile.name, "GPT-5.3 Codex");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 400_000);
        assert_eq!(limits.output, 128_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 1.75).abs() < f64::EPSILON);
        assert!((cost.output - 14.00).abs() < f64::EPSILON);
    }

    #[test]
    fn test_gpt55_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.5").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-realtime-2").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.5-pro").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.5-2026-04-23").unwrap();
        assert_eq!(profile.name, "GPT-5.5");

        let pro = get_model_profile(&LlmProviderType::Openai, "gpt-5.5-pro-2026-04-23").unwrap();
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
    fn test_gpt54_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-mini").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-nano").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-pro").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-5.4-2026-03-05").unwrap();
        assert_eq!(profile.name, "GPT-5.4");
    }

    #[test]
    fn test_gpt54_mini_versioned() {
        let profile =
            get_model_profile(&LlmProviderType::Openai, "gpt-5.4-mini-2026-03-17").unwrap();
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

    // GPT-4.1 model tests

    #[test]
    fn test_gpt41_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4.1").unwrap();
        assert_eq!(profile.name, "GPT-4.1");
        assert_eq!(profile.family, "gpt-4.1");
        assert!(!profile.reasoning);
        assert!(profile.tool_call);
    }

    #[test]
    fn test_gpt41_mini_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4.1-mini").unwrap();
        assert_eq!(profile.name, "GPT-4.1 mini");
        assert!(!profile.reasoning);
    }

    #[test]
    fn test_gpt41_nano_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "gpt-4.1-nano").unwrap();
        assert_eq!(profile.name, "GPT-4.1 nano");
        assert!(!profile.reasoning);
    }

    // o3/o4 reasoning model tests

    #[test]
    fn test_o3_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o3").unwrap();
        assert_eq!(profile.name, "o3");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    #[test]
    fn test_o3_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o3-pro").unwrap();
        assert_eq!(profile.name, "o3 Pro");
        assert!(profile.reasoning);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::High);
    }

    #[test]
    fn test_o4_mini_profile() {
        let profile = get_model_profile(&LlmProviderType::Openai, "o4-mini").unwrap();
        assert_eq!(profile.name, "o4 mini");
        assert!(profile.reasoning);
        let effort = profile.reasoning_effort.unwrap();
        assert_eq!(effort.default, ReasoningEffort::Medium);
    }

    // Claude 4.7 / 4.6 model tests

    #[test]
    fn test_claude_opus_47_profile() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-7").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-6").unwrap();
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
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-6").unwrap();
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
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-7-20260416").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.7");
    }

    #[test]
    fn test_claude_sonnet_46_versioned() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-6-20260217").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4.6");
    }

    // Claude 4.5 model tests

    #[test]
    fn test_claude_opus_45_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-5-20251101").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.5");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
    }

    #[test]
    fn test_claude_sonnet_45_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-5-20250929").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4.5");
        assert!(profile.reasoning);
    }

    #[test]
    fn test_claude_haiku_45_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-haiku-4-5-20251001").unwrap();
        assert_eq!(profile.name, "Claude Haiku 4.5");
        assert!(profile.reasoning);
    }

    // Claude 4.1 model tests

    #[test]
    fn test_claude_opus_41_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-1-20250805").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.1");
        assert_eq!(profile.family, "claude-opus-4-1");
        assert!(profile.reasoning);
        assert!(profile.tool_call);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 32_000);

        let cost = profile.cost.unwrap();
        assert!((cost.input - 15.00).abs() < f64::EPSILON);
        assert!((cost.output - 75.00).abs() < f64::EPSILON);
    }

    // Claude Sonnet 4 updated output limit

    #[test]
    fn test_claude_sonnet_4_output_limit() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-sonnet-4-20250514").unwrap();
        assert_eq!(profile.name, "Claude Sonnet 4");
        let limits = profile.limits.unwrap();
        assert_eq!(limits.output, 64_000);
        assert!(profile.reasoning);
    }

    #[test]
    fn test_claude_37_sonnet_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Anthropic, "claude-3-7-sonnet-20250219").unwrap();
        assert_eq!(profile.name, "Claude 3.7 Sonnet");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
    }

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
    fn test_normalize_claude_41_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-1"),
            "claude-opus-4-1"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-opus-4-1-20250805"),
            "claude-opus-4-1"
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

    #[test]
    fn test_normalize_claude_37_model_ids() {
        assert_eq!(
            normalize_anthropic_model_id("claude-3-7-sonnet"),
            "claude-3-7-sonnet"
        );
        assert_eq!(
            normalize_anthropic_model_id("claude-3-7-sonnet-20250219"),
            "claude-3-7-sonnet"
        );
    }

    // Gemini model tests

    #[test]
    fn test_gemini_25_pro_profile() {
        let profile = get_model_profile(&LlmProviderType::Gemini, "gemini-2.5-pro").unwrap();
        assert_eq!(profile.name, "Gemini 2.5 Pro");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
        assert!(profile.structured_output);

        let limits = profile.limits.unwrap();
        assert_eq!(limits.context, 1_048_576);
        assert!(limits.input.is_none());
    }

    #[test]
    fn test_gemini_25_flash_profile() {
        let profile = get_model_profile(&LlmProviderType::Gemini, "gemini-2.5-flash").unwrap();
        assert_eq!(profile.name, "Gemini 2.5 Flash");
        assert!(profile.reasoning);
        assert!(profile.tool_call);
    }

    #[test]
    fn test_gemini_20_flash_profile() {
        let profile = get_model_profile(&LlmProviderType::Gemini, "gemini-2.0-flash").unwrap();
        assert_eq!(profile.name, "Gemini 2.0 Flash");
        assert!(!profile.reasoning);
        assert!(profile.tool_call);
    }

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
        let profile = get_model_profile(&LlmProviderType::Gemini, "unknown-model");
        assert!(profile.is_none());
    }

    // Newly added flagship model profiles

    #[test]
    fn test_claude_fable_5_profile() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-fable-5").unwrap();
        assert_eq!(profile.name, "Claude Fable 5");
        assert_eq!(profile.family, "claude-fable-5");
        assert!(profile.reasoning);
        // Fable 5 removed sampling parameters (temperature returns 400).
        assert!(!profile.temperature);
        let cost = profile.cost.as_ref().unwrap();
        assert_eq!(cost.input, 10.00);
        assert_eq!(cost.output, 50.00);
        let limits = profile.limits.as_ref().unwrap();
        // Bare id is the 200K profile; the 1M window is `claude-fable-5[1m]`
        // (see test_claude_fable_5_1m_variant).
        assert_eq!(limits.context, 200_000);
        assert_eq!(limits.output, 128_000);
        assert!(profile.reasoning_effort.is_some());
    }

    #[test]
    fn test_claude_opus_4_8_profile() {
        let profile = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-8").unwrap();
        assert_eq!(profile.name, "Claude Opus 4.8");
        assert_eq!(profile.family, "claude-opus-4-8");
        assert!(profile.reasoning);
        // Opus 4.8 removed sampling parameters (temperature returns 400).
        assert!(!profile.temperature);
        // Bare id is the 200K profile; the 1M window is `claude-opus-4-8[1m]`.
        assert_eq!(profile.limits.as_ref().unwrap().context, 200_000);
        assert!(profile.reasoning_effort.is_some());
    }

    #[test]
    fn test_claude_opus_4_8_1m_variant() {
        // `[1m]` is the large-context twin: same flat pricing, 1M context,
        // "(1M)" display suffix, shared family for grouping.
        let base = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-8").unwrap();
        assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

        let m1 = get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-8[1m]").unwrap();
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
    fn test_claude_fable_5_1m_variant() {
        let base = get_model_profile(&LlmProviderType::Anthropic, "claude-fable-5").unwrap();
        assert_eq!(base.limits.as_ref().unwrap().context, 200_000);

        let m1 = get_model_profile(&LlmProviderType::Anthropic, "claude-fable-5[1m]").unwrap();
        assert_eq!(m1.name, "Claude Fable 5 (1M)");
        assert_eq!(m1.family, "claude-fable-5");
        assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
        assert_eq!(m1.cost.unwrap().input, base.cost.unwrap().input);
    }

    #[test]
    fn test_claude_opus_4_7_and_4_6_have_1m_variants() {
        for id in ["claude-opus-4-7[1m]", "claude-opus-4-6[1m]"] {
            let m1 = get_model_profile(&LlmProviderType::Anthropic, id).unwrap();
            assert_eq!(m1.limits.as_ref().unwrap().context, 1_000_000);
            assert!(m1.name.ends_with("(1M)"));
        }
    }

    #[test]
    fn test_gemini_3_1_pro_preview_profile() {
        let profile =
            get_model_profile(&LlmProviderType::Gemini, "gemini-3.1-pro-preview").unwrap();
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
        let profile =
            get_model_profile(&LlmProviderType::Gemini, "gemini-3.1-pro-preview-02-19").unwrap();
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
            ("grok-4.3", "Grok 4.3"),
            ("x-ai/grok-4.3", "Grok 4.3"),
        ];
        for (id, name) in cases {
            let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, id)
                .unwrap_or_else(|| panic!("missing profile for {id}"));
            assert_eq!(profile.name, name, "wrong profile for {id}");
        }
    }

    #[test]
    fn test_grok_4_3_has_context_tier() {
        let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, "grok-4.3").unwrap();
        let cost = profile.cost.as_ref().unwrap();
        assert_eq!(cost.cost_tiers.len(), 1);
        assert_eq!(cost.cost_tiers[0].above_tokens, 200_000);
    }

    #[test]
    fn test_mai_preview_has_no_cost_or_limits() {
        // Microsoft never published pricing/limits for MAI-1-preview.
        let profile =
            get_model_profile(&LlmProviderType::OpenaiCompletions, "MAI-1-preview").unwrap();
        assert!(profile.cost.is_none());
        assert!(profile.limits.is_none());
        assert!(!profile.reasoning);
    }

    #[test]
    fn test_third_party_unknown_still_none() {
        let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, "totally-made-up");
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
                get_model_profile(&LlmProviderType::OpenaiCompletions, id).is_some(),
                "{id} should resolve under openai_completions"
            );
            assert!(
                get_model_profile(&LlmProviderType::Openai, id).is_some(),
                "{id} should resolve under openai (Open Responses gateway)"
            );
            assert!(
                get_model_profile(&LlmProviderType::AzureOpenai, id).is_none(),
                "{id} must not resolve under azure_openai"
            );
            assert!(
                get_model_profile(&LlmProviderType::Anthropic, id).is_none(),
                "{id} must not resolve under anthropic"
            );
        }
        // Native phases / tool_search are advertised only on the Responses
        // surface, never on Chat Completions.
        let grok_completions =
            get_model_profile(&LlmProviderType::OpenaiCompletions, "grok-4.3").unwrap();
        assert!(!grok_completions.supports_phases);
        assert!(!grok_completions.tool_search);

        // Genuine OpenAI models still resolve under all OpenAI-family types.
        assert!(get_model_profile(&LlmProviderType::Openai, "gpt-4o").is_some());
        assert!(get_model_profile(&LlmProviderType::AzureOpenai, "gpt-4o").is_some());
    }

    #[test]
    fn test_phases_and_tool_search_gated_to_responses_surface() {
        // GPT-5.4 advertises phases + tool_search on the Responses surface...
        let responses = get_model_profile(&LlmProviderType::Openai, "gpt-5.4").unwrap();
        assert!(responses.supports_phases);
        assert!(responses.tool_search);
        // ...but not when reached via Chat Completions or Azure.
        let completions =
            get_model_profile(&LlmProviderType::OpenaiCompletions, "gpt-5.4").unwrap();
        assert!(!completions.supports_phases);
        assert!(!completions.tool_search);
    }

    #[test]
    fn test_anthropic_native_tool_search_by_family() {
        // Claude 4-family + Fable advertise Anthropic's hosted tool_search.
        for id in [
            "claude-fable-5",
            "claude-opus-4-8",
            "claude-opus-4-7",
            "claude-opus-4-6",
            "claude-opus-4-5",
            "claude-opus-4-1",
            "claude-opus-4",
            "claude-sonnet-4-6",
            "claude-sonnet-4-5",
            "claude-sonnet-4",
            "claude-haiku-4-5",
        ] {
            let p = get_model_profile(&LlmProviderType::Anthropic, id)
                .unwrap_or_else(|| panic!("{id} should resolve under anthropic"));
            assert!(p.tool_search, "{id} should advertise native tool_search");
        }
        // The 1M twin inherits the flag.
        assert!(
            get_model_profile(&LlmProviderType::Anthropic, "claude-opus-4-8[1m]")
                .unwrap()
                .tool_search
        );
        // Pre-4 Claude does not support it.
        assert!(
            !get_model_profile(&LlmProviderType::Anthropic, "claude-3-5-haiku")
                .map(|p| p.tool_search)
                .unwrap_or(false)
        );
        // Reached via a non-first-party transport (Bedrock ConverseStream lacks
        // server-side tool search; OpenRouter's stateless shim doesn't implement
        // it), the same model must not advertise hosted tool_search — it falls
        // back to client-side search via auto_tool_search.
        if let Some(bedrock) = get_model_profile(&LlmProviderType::Bedrock, "claude-opus-4-8") {
            assert!(!bedrock.tool_search);
        }
    }

    #[test]
    fn test_model_vendor_lookup() {
        assert_eq!(
            get_model_vendor(&LlmProviderType::Anthropic, "claude-opus-4-8"),
            Some(ModelVendor::Anthropic)
        );
        assert_eq!(
            get_model_vendor(
                &LlmProviderType::OpenaiCompletions,
                "nvidia/nemotron-3-super-120b-a12b"
            ),
            Some(ModelVendor::Nvidia)
        );
        assert_eq!(
            get_model_vendor(&LlmProviderType::Openai, "gpt-5.4"),
            Some(ModelVendor::OpenAi)
        );
        assert_eq!(get_model_vendor(&LlmProviderType::Openai, "made-up"), None);
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
            let profile = get_model_profile(&LlmProviderType::OpenaiCompletions, id)
                .unwrap_or_else(|| panic!("missing profile for {id}"));
            assert_eq!(profile.name, name, "wrong profile for {id}");
        }
    }

    #[test]
    fn test_estimate_cost_usd_known_model() {
        // gpt-4o profile: input $2.50/M, output $10.00/M.
        let est = estimate_cost_usd(&LlmProviderType::Openai, "gpt-4o", 1_000_000, 500_000)
            .expect("known model should yield an estimate");
        // 1M input * 2.50 + 0.5M output * 10.00 = 2.50 + 5.00 = 7.50
        assert!((est - 7.50).abs() < 1e-9, "got {est}");
    }

    #[test]
    fn test_estimate_cost_usd_unknown_model_is_none() {
        assert!(estimate_cost_usd(&LlmProviderType::Openai, "no-such-model", 100, 50).is_none());
    }
}
