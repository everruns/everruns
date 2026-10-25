// GPT-6 series profiles: Astra (flagship), Sol (balanced), Luna (fast/cheap),
// and the GPT-6.1 Sol refresh.
//
// All of them share a 1.05M context, 128K output, and a tier that bills prompts
// above 272K input tokens at 2x input/cache and 1.5x output for the whole
// request. Source: developers.openai.com/api/docs/models/gpt-6-{astra,sol,luna}
// (models.dev did not yet list them at the time of addition; refresh once it
// catches up).

use super::effort;
use super::speed::priced_speed;
use super::verbosity_standard;
use crate::model_profile_data::types::{
    CostTier, INLINE_TOOL_CHANGES_PARAMETER, Modality, ModelCost, ModelLimits, ModelModalities,
    ModelProfile, ReasoningEffort, ReasoningEffortConfig, Speed, SpeedConfig,
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
        "gpt-6-astra" => Some(ModelProfile {
            // Ultrafast is GA for Astra (guides/ultrafast-mode, 2026-09-29).
            speed: Some(speeds(Speed::Priority, true)),
            ..profile(
                "GPT-6 Astra",
                "gpt-6-astra",
                "OpenAI's most capable model, built for the hardest end-to-end work: complex reasoning, coding, computer use, research, and document creation.",
                "2026-09-04",
                "2026-04-30",
                [10.00, 50.00, 1.00],
                // Astra drops `none`.
                efforts(&ALL_EFFORTS[1..]),
            )
        }),
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
        // GPT-6.1 Sol: near-Astra agentic coding and computer use at Sol's
        // price, released at DevDay 2026-09-29. Cached input drops to $0.10;
        // it drops `none` effort like Astra and caps input at 922K. OpenAI's
        // docs name the premium tier `fast` for this model; Ultrafast is
        // announced "in the coming days", so it is not offered yet.
        "gpt-6.1-sol" => {
            let mut sol = profile(
                "GPT-6.1 Sol",
                "gpt-6.1-sol",
                "Near-Astra performance at a lower cost for complex coding, computer use, and professional work.",
                "2026-09-29",
                "2026-04-30",
                [2.00, 10.00, 0.10],
                efforts(&ALL_EFFORTS[1..]),
            );
            sol.speed = Some(speeds(Speed::Fast, false));
            if let Some(limits) = sol.limits.as_mut() {
                limits.input = Some(922_000);
            }
            Some(sol)
        }
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

/// Flex, Standard and the 2x premium tier (`Priority` or its newer `Fast`
/// name), optionally with the 6x Ultrafast tier.
fn speeds(premium: Speed, ultrafast: bool) -> SpeedConfig {
    let mut values = vec![
        priced_speed(Speed::Flex, "Flex", 0.5),
        priced_speed(Speed::Default, "Standard", 1.0),
        priced_speed(premium, "Fast", 2.0),
    ];
    if ultrafast {
        values.push(priced_speed(Speed::Ultrafast, "Ultrafast", 6.0));
    }
    SpeedConfig {
        values,
        default: Speed::Default,
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
        speed: Some(speeds(Speed::Priority, false)),
        verbosity: Some(verbosity_standard()),
        tool_search: true,
        // Inline tool changes (`additional_tools`): a redefinition kept the
        // prompt cache in a live round trip on every GPT-6 tier
        // (`test_openai_tool_reveal_keeps_prompt_cache`, 2026-10-12).
        supported_parameters: vec![INLINE_TOOL_CHANGES_PARAMETER.to_string()],
        supports_phases: true,
        supports_server_compaction: false,
        model_card_url: None,
        decisions: None,
    }
}
