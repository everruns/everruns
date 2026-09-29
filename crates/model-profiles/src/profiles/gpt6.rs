// GPT-6 series profiles: Astra (flagship), Sol (balanced), Luna (fast/cheap).
//
// All three share a 1.05M context, 128K output, and a tier that bills prompts
// above 272K input tokens at 2x input/cache and 1.5x output for the whole
// request. Source: developers.openai.com/api/docs/models/gpt-6-{astra,sol,luna}
// (models.dev did not yet list them at the time of addition; refresh once it
// catches up).

use super::{effort, speed_flex_priority, verbosity_standard};
use crate::types::{
    CostTier, Modality, ModelCost, ModelLimits, ModelModalities, ModelProfile, ReasoningEffort,
    ReasoningEffortConfig,
};

pub(super) fn profile_data(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        // GPT-6 Astra: flagship, publicly released 2026-09-04 (limited preview
        // 2026-09-03). `reasoning.effort` adds a `max` tier above `xhigh` —
        // verified live against `/v1/responses` (2026-09-04).
        //
        // Live-verified quirk: unlike GPT-5.x, GPT-6 Astra's reasoning item
        // never carries readable summary content (`content: []`) even with
        // `summary: "auto"`/`"concise"` — only `encrypted_content`. It is
        // therefore excluded from the extended-thinking transcript test in
        // `crates/llm-tests`, which asserts on readable reasoning text.
        "gpt-6-astra" => Some(profile(
            "GPT-6 Astra",
            "gpt-6-astra",
            "OpenAI's most capable model, built for the hardest end-to-end work: complex reasoning, coding, computer use, research, and document creation.",
            "2026-09-04",
            "2026-04-30",
            [10.00, 50.00, 1.00],
            // Astra drops `none`.
            efforts(&ALL_EFFORTS[1..]),
        )),
        // GPT-6 Sol / Luna: faster, cheaper tiers built on Astra, publicly
        // released 2026-09-22 at half the price of their GPT-5.6 namesakes.
        // They keep `none` effort alongside Astra's `max`.
        "gpt-6-sol" => Some(profile(
            "GPT-6 Sol",
            "gpt-6-sol",
            "Balanced GPT-6 tier. Astra-level reliability for agentic coding and multi-step analysis at a fraction of the cost.",
            "2026-09-22",
            "2026-04-20",
            [2.00, 10.00, 0.20],
            efforts(&ALL_EFFORTS),
        )),
        "gpt-6-luna" => Some(profile(
            "GPT-6 Luna",
            "gpt-6-luna",
            "Fastest, most cost-efficient GPT-6 tier. Built for high-volume, latency-sensitive agent work, classification, extraction, and routing.",
            "2026-09-22",
            "2026-05-18",
            [0.10, 0.50, 0.01],
            efforts(&ALL_EFFORTS),
        )),
        _ => None,
    }
}

const ALL_EFFORTS: [(ReasoningEffort, &str); 6] = [
    (ReasoningEffort::None, "None"),
    (ReasoningEffort::Low, "Low"),
    (ReasoningEffort::Medium, "Medium"),
    (ReasoningEffort::High, "High"),
    (ReasoningEffort::Xhigh, "Extra High"),
    (ReasoningEffort::Max, "Max"),
];

/// Default effort is medium across the series.
fn efforts(values: &[(ReasoningEffort, &str)]) -> ReasoningEffortConfig {
    ReasoningEffortConfig {
        values: values
            .iter()
            .map(|&(value, name)| effort(value, name))
            .collect(),
        default: ReasoningEffort::Medium,
    }
}

/// `[input, output, cache_read]` base rates per million tokens. Cache writes
/// bill at 1.25x input; the >272K tier doubles input/cache and adds 50% output.
fn profile(
    name: &str,
    family: &str,
    description: &str,
    released: &str,
    knowledge: &str,
    [input, output, cache_read]: [f64; 3],
    reasoning_effort: ReasoningEffortConfig,
) -> ModelProfile {
    ModelProfile {
        name: name.into(),
        family: family.into(),
        description: Some(description.into()),
        release_date: Some(released.into()),
        last_updated: Some(released.into()),
        attachment: true,
        reasoning: true,
        temperature: false,
        knowledge: Some(knowledge.into()),
        tool_call: true,
        structured_output: true,
        open_weights: false,
        cost: Some(ModelCost {
            input,
            output,
            cache_read: Some(cache_read),
            cache_write: Some(input * 1.25),
            cost_tiers: vec![CostTier {
                above_tokens: 272_000,
                input: input * 2.0,
                output: output * 1.5,
                cache_read: Some(cache_read * 2.0),
                cache_write: Some(input * 2.5),
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
        reasoning_effort: Some(reasoning_effort),
        speed: Some(speed_flex_priority()),
        verbosity: Some(verbosity_standard()),
        tool_search: true,
        supported_parameters: Vec::new(),
        supports_phases: true,
        supports_server_compaction: false,
    }
}

#[cfg(test)]
mod tests {
    use super::super::get_model_profile;
    use super::*;

    #[test]
    fn test_gpt6_profiles() {
        for (id, name, knowledge, released, rates, tier_rates, effort_values) in [
            (
                "gpt-6-astra",
                "GPT-6 Astra",
                "2026-04-30",
                "2026-09-04",
                [10.00, 50.00, 1.00],
                [20.00, 75.00, 2.00],
                &ALL_EFFORTS[1..],
            ),
            (
                "gpt-6-sol",
                "GPT-6 Sol",
                "2026-04-20",
                "2026-09-22",
                [2.00, 10.00, 0.20],
                [4.00, 15.00, 0.40],
                &ALL_EFFORTS[..],
            ),
            (
                "gpt-6-luna",
                "GPT-6 Luna",
                "2026-05-18",
                "2026-09-22",
                [0.10, 0.50, 0.01],
                [0.20, 0.75, 0.02],
                &ALL_EFFORTS[..],
            ),
        ] {
            let profile = get_model_profile("openai", id).unwrap();
            assert_eq!(profile.name, name);
            assert_eq!(profile.family, id);
            assert!(profile.reasoning);
            assert!(!profile.temperature);
            assert!(profile.tool_call);
            assert!(profile.structured_output);
            assert!(profile.tool_search);
            assert!(profile.supports_phases);
            assert!(profile.verbosity.is_some());
            assert!(profile.speed.is_some());
            assert_eq!(profile.knowledge.as_deref(), Some(knowledge));
            assert_eq!(profile.release_date.as_deref(), Some(released));

            let limits = profile.limits.unwrap();
            assert_eq!(limits.context, 1_050_000);
            assert_eq!(limits.output, 128_000);

            let close = |a: f64, b: f64| (a - b).abs() < 1e-9;
            let cost = profile.cost.unwrap();
            assert!(close(cost.input, rates[0]), "{id}");
            assert!(close(cost.output, rates[1]), "{id}");
            assert!(close(cost.cache_read.unwrap(), rates[2]), "{id}");
            assert!(close(cost.cache_write.unwrap(), rates[0] * 1.25), "{id}");
            assert_eq!(cost.cost_tiers.len(), 1);
            let tier = &cost.cost_tiers[0];
            assert_eq!(tier.above_tokens, 272_000);
            assert!(close(tier.input, tier_rates[0]), "{id}");
            assert!(close(tier.output, tier_rates[1]), "{id}");
            assert!(close(tier.cache_read.unwrap(), tier_rates[2]), "{id}");

            let effort = profile.reasoning_effort.unwrap();
            assert_eq!(effort.default, ReasoningEffort::Medium);
            assert_eq!(
                effort.values.iter().map(|v| v.value).collect::<Vec<_>>(),
                effort_values.iter().map(|&(v, _)| v).collect::<Vec<_>>(),
                "{id}"
            );
        }
    }
}
