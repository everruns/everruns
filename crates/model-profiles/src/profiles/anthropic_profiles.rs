use super::*;

pub(super) fn anthropic_profile_data_inner(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        // Claude Fable 5.1 (newest — top tier above Opus; successor to Fable 5)
        // Source: Anthropic model card (claude-api skill `shared/models.md`) and
        // docs.claude.com — Fable 5.1 is not yet in models.dev. Same tier, limits
        // and per-token price as Fable 5 ($10/$50); cache reads drop to $0.25/MTok
        // (a quarter of Fable 5's $1.00). Same request surface as Fable 5:
        // adaptive thinking only (an explicit `thinking: {type: "disabled"}`
        // returns 400, so the param is omitted when no effort is set), sampling
        // parameters removed (`temperature: false`). Fable 5.1 additionally
        // rejects forced tool use (`tool_choice` `any`/`tool` return 400); the
        // Anthropic driver only ever sends `auto`, so no driver change is needed.
        // Release/knowledge dates are not published in the model card; the
        // Models API exposes them at runtime.
        "claude-fable-5-1" => Some(ModelProfile {
            name: "Claude Fable 5.1".into(),
            family: "claude-fable-5-1".into(),
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
                cache_read: Some(0.25),
                cache_write: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-fable-5-1[1m]` is the 1M twin.
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
            supports_server_compaction: false,
        }),

        // Claude Fable 5 (previous Fable release; still served, below Fable 5.1)
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
                cache_write: None,
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
            supports_server_compaction: false,
        }),

        // Claude Opus 5.5 (current Opus; successor to Opus 5 at a lower price)
        // Source: Anthropic model card (claude-api skill). Same 200K/1M-twin context,
        // 128K output, and tokenizer as Opus 5 at $4/$20 (cache-read $0.20).
        // Thinking cannot be disabled (adaptive only; omitting `thinking` still
        // runs adaptive), sampling parameters are removed (`temperature: false`),
        // and forced `tool_choice` any/tool returns 400 — the driver only sends
        // `auto`. The API's default effort is `medium`, one below Opus 5's `high`;
        // the driver sends this profile's `high` when the caller picks no effort.
        "claude-opus-5-5" => Some(ModelProfile {
            name: "Claude Opus 5.5".into(),
            family: "claude-opus-5-5".into(),
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
                input: 4.00,
                output: 20.00,
                cache_read: Some(0.20),
                cache_write: None,
                cost_tiers: vec![],
            }),
            limits: Some(ModelLimits {
                // Bare id is the 200K profile; `claude-opus-5-5[1m]` is the 1M twin.
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
            supports_server_compaction: false,
        }),

        // Claude Opus 5 (previous Opus; below Opus 5.5, above Opus 4.8)
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
        }),

        // 1M-context twins of the base profiles above. Same pricing and
        // capabilities; only the context limit and display name differ.
        "claude-fable-5-1[1m]" => {
            anthropic_profile_data("claude-fable-5-1").map(anthropic_1m_variant)
        }
        "claude-fable-5[1m]" => anthropic_profile_data("claude-fable-5").map(anthropic_1m_variant),
        "claude-opus-5-5[1m]" => {
            anthropic_profile_data("claude-opus-5-5").map(anthropic_1m_variant)
        }
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
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
                cache_write: None,
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
            supports_server_compaction: false,
        }),

        _ => None,
    }
}
