use super::*;

pub(super) fn gemini_profile_data(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        // Gemini 4 family. Source: Google's model pages and pricing page,
        // re-checked against the live `models` listing on 2026-11-19. Every
        // member shares the 1M unified window, long decode past the 64K
        // synchronous default, thinking levels instead of budgets, and
        // implicit-only caching. Carbon and Argon reject `xhigh` (probed).
        // Argon and Carbon: introductory pricing through 2026-11-30; the
        // scheduled rates thereafter are $4/$20/$0.20.
        "gemini-4-argon" => Some(gemini_4(
            ("Gemini 4 Argon", "gemini-4-argon", "2026-10-13"),
            (2.0, 10.0, 0.10),
            reasoning_effort_standard(),
        )),
        // Google's strongest reasoning model of the family.
        "gemini-4-carbon" => Some(gemini_4(
            ("Gemini 4 Carbon", "gemini-4-carbon", "2026-11-10"),
            (2.0, 10.0, 0.10),
            reasoning_effort_standard(),
        )),
        // Fast, cheapest member; Google's suggested starting point.
        "gemini-4-hydrogen" => Some(gemini_4(
            ("Gemini 4 Hydrogen", "gemini-4-hydrogen", "2026-11-12"),
            (1.5, 7.5, 0.075),
            reasoning_effort_gemini_xhigh(),
        )),
        // Adapted from Carbon for complex software engineering, between
        // Hydrogen and Carbon.
        "gemini-4-neon" => Some(gemini_4(
            ("Gemini 4 Neon", "gemini-4-neon", "2026-11-18"),
            (3.0, 15.0, 0.15),
            reasoning_effort_gemini_xhigh(),
        )),
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
                cache_write: None,
                cost_tiers: vec![CostTier {
                    above_tokens: 200_000,
                    input: 4.00,
                    output: 18.00,
                    cache_read: Some(0.40),
                    cache_write: None,
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
            supports_server_compaction: false,
            model_card_url: None,
            decisions: None,
            images: None,
        }),

        // Gemini 3.6/3.7/3.8 Flash and 3.5 Flash Lite. Source: models.dev
        // (google provider). All four share the 1M context, 64K output, the
        // full multimodal input set, and no long-context price tier. Reasoning
        // effort is offered upstream but left unset, as for the other Gemini
        // profiles here. 3.8 Flash publishes no knowledge cutoff.
        "gemini-3.8-flash" => Some(gemini_flash(
            "Gemini 3.8 Flash",
            "gemini-3.8-flash",
            "2026-09-02",
            None,
            (0.75, 3.75, 0.075),
        )),
        "gemini-3.7-flash" => Some(gemini_flash(
            "Gemini 3.7 Flash",
            "gemini-3.7-flash",
            "2026-08-13",
            Some("2026-03"),
            (0.75, 3.75, 0.075),
        )),
        "gemini-3.6-flash" => Some(gemini_flash(
            "Gemini 3.6 Flash",
            "gemini-3.6-flash",
            "2026-07-21",
            Some("2026-03"),
            (0.75, 3.75, 0.075),
        )),
        "gemini-3.5-flash-lite" => Some(gemini_flash(
            "Gemini 3.5 Flash Lite",
            "gemini-3.5-flash-lite",
            "2026-07-21",
            Some("2026-03"),
            (0.30, 2.50, 0.03),
        )),

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
                cache_write: None,
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
            supports_server_compaction: false,
            model_card_url: None,
            decisions: None,
            images: None,
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
                cache_write: None,
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
            supports_server_compaction: false,
            model_card_url: None,
            decisions: None,
            images: None,
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
                cache_write: None,
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
            supports_server_compaction: false,
            model_card_url: None,
            decisions: None,
            images: None,
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
                cache_write: None,
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
            supports_server_compaction: false,
            model_card_url: None,
            decisions: None,
            images: None,
        }),

        _ => None,
    }
}

/// Current-gen Gemini Flash profile: reasoning, tools, structured output, 1M
/// context, 64K output, text/image/audio/video/PDF in, flat pricing.
/// `cost` is (input, output, cache_read) per million tokens.
fn gemini_flash(
    name: &str,
    family: &str,
    release_date: &str,
    knowledge: Option<&str>,
    cost: (f64, f64, f64),
) -> ModelProfile {
    let (input, output, cache_read) = cost;
    ModelProfile {
        name: name.into(),
        family: family.into(),
        description: None,
        release_date: Some(release_date.into()),
        last_updated: Some(release_date.into()),
        attachment: true,
        reasoning: true,
        temperature: true,
        knowledge: knowledge.map(Into::into),
        tool_call: true,
        structured_output: true,
        open_weights: false,
        cost: Some(ModelCost {
            input,
            output,
            cache_read: Some(cache_read),
            cache_write: None,
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
        supports_server_compaction: false,
        model_card_url: None,
        decisions: None,
        images: None,
    }
}

/// Gemini 4 profile. The 1M output ceiling requires continuation; the driver
/// defaults to 64K unless the caller explicitly raises the total output
/// budget. `id` is (name, family, release date); `cost` is (input, output,
/// cache_read) per million tokens.
fn gemini_4(
    id: (&str, &str, &str),
    cost: (f64, f64, f64),
    reasoning_effort: ReasoningEffortConfig,
) -> ModelProfile {
    let (name, family, release_date) = id;
    let mut profile = gemini_flash(name, family, release_date, None, cost);
    profile.temperature = false;
    profile.limits = Some(ModelLimits {
        context: 1_048_576,
        input: None,
        output: 1_048_576,
        max_media: None,
    });
    profile.reasoning_effort = Some(reasoning_effort);
    profile.model_card_url = Some(format!(
        "https://ai.google.dev/gemini-api/docs/models/{family}"
    ));
    profile
}
