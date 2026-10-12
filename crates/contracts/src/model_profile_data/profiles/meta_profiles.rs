use super::*;

pub(super) fn meta_profile_data(model_id: &str) -> Option<ModelProfile> {
    // Meta prices the Standard and Contributor tiers identically across Muse
    // Spark 1.2 and 1.3; only the data-use terms differ between tiers.
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

    // Effort levels Meta accepts, probed against api.meta.ai on 2026-10-12.
    // No Muse model accepts `none` (HTTP 400). `max` is accepted from 1.3 on,
    // including the 1.3 Contributor tier, although Meta's docs say otherwise;
    // the API is the authority here. 1.2 stops at `xhigh`.
    let (name, family, release_date, description, cost, max_effort) = match model_id {
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
        reasoning_effort: Some(muse_efforts(max_effort)),
        speed: None,
        verbosity: None,
        tool_search: true,
        supported_parameters: Vec::new(),
        supports_phases: true,
        supports_server_compaction: false,
        decisions: None,
    })
}

/// An unset effort sends no `reasoning` field and Meta picks a
/// model-determined level. `default` is what pickers show for that unset
/// state; Meta drivers never send it on the caller's behalf.
fn muse_efforts(max: bool) -> ReasoningEffortConfig {
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
        default: ReasoningEffort::Medium,
    }
}
