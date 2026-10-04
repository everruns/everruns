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

    let (name, family, release_date, description, cost) = match model_id {
        "muse-spark-1.3" => (
            "Muse Spark 1.3",
            "muse-spark-1.3",
            "2026-09-02",
            "Meta's Muse Spark model for long-horizon coding and multi-step agentic work, with native tool calling and MCP support. Prompts and completions are not used to train Meta models.",
            MUSE_STANDARD_COST,
        ),
        "muse-spark-1.3-contributor" => (
            "Muse Spark 1.3 Contributor",
            "muse-spark-1.3",
            "2026-09-02",
            "Discounted Muse Spark 1.3 tier with the same model and capabilities as Standard. Prompts and completions are used to train and improve Meta models; rate-limited by tokens.",
            MUSE_CONTRIBUTOR_COST,
        ),
        "muse-spark-1.2" => (
            "Muse Spark 1.2",
            "muse-spark-1.2",
            "2026-08-05",
            "Meta's coding-optimized Muse Spark model. Prompts and completions are not used to train Meta models.",
            MUSE_STANDARD_COST,
        ),
        "muse-spark-1.2-contributor" => (
            "Muse Spark 1.2 Contributor",
            "muse-spark-1.2",
            "2026-08-05",
            "Discounted Muse Spark 1.2 tier where prompts and completions may be used to train future Meta models.",
            MUSE_CONTRIBUTOR_COST,
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
        // Meta documents a model-determined default rather than a stable
        // effort value, which the current profile type cannot represent.
        reasoning_effort: None,
        speed: None,
        verbosity: None,
        tool_search: true,
        supported_parameters: Vec::new(),
        supports_phases: true,
        supports_server_compaction: false,
        decisions: None,
    })
}
