use super::*;

pub(super) fn meta_profile_data(model_id: &str) -> Option<ModelProfile> {
    if let Some(profile) = muse_open_weight_profile(model_id) {
        return Some(profile);
    }
    // Meta prices every Muse Spark version (1.1, 1.2, 1.3) alike per tier;
    // only the data-use terms differ between Standard and Contributor.
    const MUSE_STANDARD_COST: ModelCost = ModelCost {
        input: 1.25,
        output: 4.25,
        cache_read: Some(0.15),
        cache_write: None,
        cost_tiers: vec![],
    };
    const MUSE_CONTRIBUTOR_COST: ModelCost = ModelCost {
        input: 0.10,
        output: 0.20,
        cache_read: Some(0.002),
        cache_write: None,
        cost_tiers: vec![],
    };

    // Muse Wonder has no Contributor tier on pay-as-you-go: Standard data-use
    // terms only.
    const MUSE_WONDER_COST: ModelCost = ModelCost {
        input: 4.00,
        output: 20.00,
        cache_read: Some(0.20),
        cache_write: None,
        cost_tiers: vec![],
    };

    // Muse Ember is Standard-only and has a long-context rate card: a request
    // over 200K input tokens bills entirely at the higher rates.
    let muse_ember_cost = ModelCost {
        input: 0.10,
        output: 0.50,
        cache_read: Some(0.004),
        cache_write: None,
        cost_tiers: vec![CostTier {
            above_tokens: 200_000,
            input: 0.30,
            output: 1.50,
            cache_read: Some(0.012),
            cache_write: None,
        }],
    };

    // Effort levels Meta accepts, probed against api.meta.ai on 2026-10-12
    // (Ember and Spark 1.1 on 2026-11-09). No Muse model accepts `none`
    // (HTTP 400). `max` is accepted on Ember, Wonder and 1.3, including the
    // 1.3 Contributor tier, although Meta's docs say otherwise; the API is
    // the authority here. 1.1 and 1.2 stop at `xhigh`.
    let (name, family, release_date, description, cost, max_effort) = match model_id {
        "muse-ember-1.0" => (
            "Muse Ember",
            "muse-ember",
            "2026-11-09",
            "Meta's smallest and most efficient Muse model, for bounded high-volume work: sub-agent tasks, tool calls, routing, and fanning one task out across many files or records. Prompts and completions are not used to train Meta models.",
            muse_ember_cost,
            true,
        ),
        "muse-wonder-1.0" => (
            "Muse Wonder",
            "muse-wonder",
            "2026-10-12",
            "Meta's most capable Muse model (preview), for complex software engineering, long-horizon agent runs, and sub-agent orchestration. Prompts and completions are not used to train Meta models.",
            MUSE_WONDER_COST,
            true,
        ),
        "muse-spark-1.3" => (
            "Muse Spark 1.3",
            "muse-spark-1.3",
            "2026-09-02",
            "Meta's Muse Spark model for long-horizon coding and multi-step agentic work, with native tool calling and MCP support. Prompts and completions are not used to train Meta models.",
            MUSE_STANDARD_COST,
            true,
        ),
        "muse-spark-1.3-contributor" => (
            "Muse Spark 1.3 Contributor",
            "muse-spark-1.3",
            "2026-09-02",
            "Discounted Muse Spark 1.3 tier with the same model and capabilities as Standard. Prompts and completions are used to train and improve Meta models; rate-limited by tokens.",
            MUSE_CONTRIBUTOR_COST,
            true,
        ),
        "muse-spark-1.2" => (
            "Muse Spark 1.2",
            "muse-spark-1.2",
            "2026-08-05",
            "Meta's coding-optimized Muse Spark model. Prompts and completions are not used to train Meta models.",
            MUSE_STANDARD_COST,
            false,
        ),
        "muse-spark-1.2-contributor" => (
            "Muse Spark 1.2 Contributor",
            "muse-spark-1.2",
            "2026-08-05",
            "Discounted Muse Spark 1.2 tier where prompts and completions may be used to train future Meta models.",
            MUSE_CONTRIBUTOR_COST,
            false,
        ),
        "muse-spark-1.1" => (
            "Muse Spark 1.1",
            "muse-spark-1.1",
            "2026-07-16",
            "The original Muse Spark model, superseded by 1.2 and 1.3. Standard tier only. Prompts and completions are not used to train Meta models.",
            MUSE_STANDARD_COST,
            false,
        ),
        _ => return None,
    };
    // Ember caps a single response at 512K tokens; the other API models
    // document only the joint context budget.
    let output = if model_id == "muse-ember-1.0" {
        524_288
    } else {
        1_048_576
    };
    // Meta documents `max` as Ember's default when no effort is sent.
    let default_effort = if model_id == "muse-ember-1.0" {
        ReasoningEffort::Max
    } else {
        ReasoningEffort::Medium
    };

    Some(ModelProfile {
        name: name.into(),
        family: family.into(),
        description: Some(description.into()),
        release_date: Some(release_date.into()),
        last_updated: Some(release_date.into()),
        attachment: true,
        reasoning: true,
        temperature: true,
        knowledge: None,
        tool_call: true,
        structured_output: true,
        open_weights: false,
        cost: Some(cost),
        limits: Some(ModelLimits {
            // Meta documents one joint input + output context budget. Callers
            // must leave room for input.
            context: 1_048_576,
            input: None,
            output,
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
        reasoning_effort: Some(muse_efforts(max_effort, default_effort)),
        speed: None,
        verbosity: None,
        tool_search: true,
        supported_parameters: Vec::new(),
        supports_phases: true,
        supports_server_compaction: false,
        model_card_url: None,
        decisions: None,
        images: None,
    })
}

/// An unset effort sends no `reasoning` field and Meta picks a
/// model-determined level. `default` is what pickers show for that unset
/// state; Meta drivers never send it on the caller's behalf.
fn muse_efforts(max: bool, default: ReasoningEffort) -> ReasoningEffortConfig {
    let mut values = vec![
        (ReasoningEffort::Minimal, "Minimal"),
        (ReasoningEffort::Low, "Low"),
        (ReasoningEffort::Medium, "Medium"),
        (ReasoningEffort::High, "High"),
        (ReasoningEffort::Xhigh, "Extra High"),
    ];
    if max {
        values.push((ReasoningEffort::Max, "Max"));
    }
    ReasoningEffortConfig {
        values: values
            .into_iter()
            .map(|(value, name)| effort(value, name))
            .collect(),
        default,
    }
}

/// Muse open-weight models: Meta publishes the weights but does not serve
/// them on Model API, so they are reached through gateways (OpenRouter,
/// OpenAI-compatible hosts). Prices are OpenRouter's listing, which varies
/// by host. Like the other gateway-served open models, no graded effort
/// selector is wired; `reasoning: true` still gates reasoning support.
fn muse_open_weight_profile(model_id: &str) -> Option<ModelProfile> {
    let (name, family, release_date, description, cost, context, output, input) = match model_id {
        "muse-spark-open" => (
            "Muse Spark Open",
            "muse-spark-open",
            "2026-10-15",
            "Open-weight, coding-optimized Muse Spark (816B MoE, 42B active) for agentic coding: reliable tool calling and deep codebase understanding.",
            ModelCost {
                input: 1.25,
                output: 4.25,
                cache_read: Some(0.15),
                cache_write: None,
                cost_tiers: vec![],
            },
            1_048_576,
            131_072,
            vec![
                Modality::Text,
                Modality::Image,
                Modality::Audio,
                Modality::Video,
            ],
        ),
        "muse-glimmer-30b" => (
            "Muse Glimmer 30B",
            "muse-glimmer",
            "2026-08-09",
            "Dense 30B open-weight multimodal model distilled from Muse Spark, built for autonomous agents on consumer hardware. Apache 2.0.",
            ModelCost {
                input: 0.30,
                output: 1.20,
                cache_read: Some(0.04),
                cache_write: None,
                cost_tiers: vec![],
            },
            131_072,
            16_384,
            vec![Modality::Text, Modality::Image],
        ),
        _ => return None,
    };
    Some(ModelProfile {
        name: name.into(),
        family: family.into(),
        description: Some(description.into()),
        release_date: Some(release_date.into()),
        last_updated: Some(release_date.into()),
        attachment: true,
        reasoning: true,
        temperature: true,
        knowledge: None,
        tool_call: true,
        structured_output: true,
        open_weights: true,
        cost: Some(cost),
        limits: Some(ModelLimits {
            context,
            input: None,
            output,
            max_media: None,
        }),
        modalities: Some(ModelModalities {
            input,
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
    })
}
