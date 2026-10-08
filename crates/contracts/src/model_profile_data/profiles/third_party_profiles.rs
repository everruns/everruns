use super::*;

/// Profile payloads for non-OpenAI models exposed through OpenAI-compatible
/// APIs (NVIDIA NIM, Alibaba, MiniMax, Mistral, Moonshot, xAI, Microsoft). Pure value
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
pub(super) fn third_party_profile_data(model_id: &str) -> Option<ModelProfile> {
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
                cache_write: None,
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
            supports_server_compaction: false,
            decisions: None,
        }),

        // Alibaba Qwen3.8 Max: 2.4T-parameter MoE flagship. Source: models.dev
        // (alibaba provider). Knowledge cutoff not published.
        "qwen3.8-max" | "qwen/qwen3.8-max" => Some(ModelProfile {
            name: "Qwen3.8 Max".into(),
            family: "qwen3.8-max".into(),
            description: None,
            release_date: Some("2026-08-03".into()),
            last_updated: Some("2026-08-03".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.00,
                output: 6.00,
                cache_read: Some(0.25),
                cache_write: Some(2.50),
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 1_000_000,
                input: None,
                output: 131_072,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![
                    Modality::Text,
                    Modality::Image,
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
            supports_server_compaction: false,
            decisions: None,
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
                cache_write: None,
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
            supports_server_compaction: false,
            decisions: None,
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
            supports_server_compaction: false,
            decisions: None,
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
            supports_server_compaction: false,
            decisions: None,
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
                cache_write: None,
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
            supports_server_compaction: false,
            decisions: None,
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
                cache_write: None,
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
            supports_server_compaction: false,
            decisions: None,
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
                cache_write: None,
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
            supports_server_compaction: false,
            decisions: None,
        }),

        // Mistral Large 4: Mistral's 1T-parameter (49B active) multimodal MoE,
        // a hybrid instruct-and-reasoning model, public preview 2026-10-06.
        // Source: models.dev (mistral provider) for limits and capabilities,
        // checked against Mistral's own `/v1/models` (524,288 context, vision,
        // function calling, reasoning) and the launch post
        // (mistral.ai/news/mistral-large-4) for list price: $1.36 / $4.18.
        // models.dev's mistral entry carries $0.68 / $2.09, which is the
        // launch discount (`/v1/models` bills it as
        // `mistral-large-4-0-launch-discount`); the table keeps list price so
        // estimates do not silently halve when the promotion ends. Cache read
        // is models.dev's 10% of input at list price.
        //
        // Reasoning is a toggle, not a grade: the API accepts only
        // `reasoning_effort` "none" and "high" (anything else is a 400), and
        // omitting it answers without thinking. Weights are promised for the
        // end of October 2026 under an unannounced license, so `open_weights`
        // stays false until they ship. Knowledge cutoff not published.
        "mistral-large-4" => Some(ModelProfile {
            name: "Mistral Large 4".into(),
            family: "mistral-large".into(),
            description: None,
            release_date: Some("2026-10-06".into()),
            last_updated: Some("2026-10-06".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 1.36,
                output: 4.18,
                cache_read: Some(0.14),
                cache_write: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 524_288,
                input: None,
                output: 262_144,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_toggle()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
            supports_server_compaction: false,
            decisions: None,
        }),

        // Mistral Medium 3.5 (`mistral-medium-2604`) and Mistral Small 4
        // (`mistral-small-2603`). Source: models.dev (mistral provider), list
        // price. Open weights. Reasoning is the same none/high toggle as Large 4.
        // models.dev does not assert structured output for Small 4. Knowledge
        // cutoff not published for Medium 3.5.
        "mistral-medium-2604" => Some(ModelProfile {
            name: "Mistral Medium 3.5".into(),
            family: "mistral-medium".into(),
            description: None,
            release_date: Some("2026-04-29".into()),
            last_updated: Some("2026-04-29".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: None,
            tool_call: true,
            structured_output: true,
            open_weights: true,
            cost: Some(ModelCost {
                input: 1.50,
                output: 7.50,
                cache_read: Some(0.15),
                cache_write: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 262_144,
                input: None,
                output: 262_144,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_toggle()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
            supports_server_compaction: false,
            decisions: None,
        }),

        "mistral-small-2603" => Some(ModelProfile {
            name: "Mistral Small 4".into(),
            family: "mistral-small".into(),
            description: None,
            release_date: Some("2026-03-16".into()),
            last_updated: Some("2026-03-16".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2025-06".into()),
            tool_call: true,
            structured_output: false,
            open_weights: true,
            cost: Some(ModelCost {
                input: 0.15,
                output: 0.60,
                cache_read: Some(0.015),
                cache_write: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                context: 262_144,
                input: None,
                output: 256_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Image],
                output: vec![Modality::Text],
            }),
            reasoning_effort: Some(reasoning_effort_toggle()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: false,
            supports_server_compaction: false,
            decisions: None,
        }),

        // xAI Grok 4.7: frontier Grok for long-running agents. Source:
        // models.dev (xai provider). Pricing has a >200K-token tier.
        "grok-4.7" | "x-ai/grok-4.7" | "xai/grok-4.7" => Some(ModelProfile {
            name: "Grok 4.7".into(),
            family: "grok-4.7".into(),
            description: None,
            release_date: Some("2026-09-21".into()),
            last_updated: Some("2026-09-21".into()),
            attachment: true,
            reasoning: true,
            temperature: true,
            knowledge: Some("2026-05".into()),
            tool_call: true,
            structured_output: true,
            open_weights: false,
            cost: Some(ModelCost {
                input: 2.00,
                output: 6.00,
                cache_read: Some(0.50),
                cache_write: None,
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 4.00,
                    output: 12.00,
                    cache_read: Some(1.00),
                    cache_write: None,
                }],
            }),
            limits: Some(ModelLimits {
                context: 500_000,
                input: None,
                output: 500_000,
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
            supports_server_compaction: false,
            decisions: None,
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
                cache_write: None,
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 2.50,
                    output: 5.00,
                    cache_read: Some(0.40),
                    cache_write: None,
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
            supports_server_compaction: false,
            decisions: None,
        }),

        _ => None,
    }
}
