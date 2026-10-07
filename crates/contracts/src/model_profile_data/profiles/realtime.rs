// OpenAI Realtime voice-session profiles (GPT Realtime 2 and 2.1). Split out of
// `openai_profiles.rs` to keep that file under its size ratchet.

use super::reasoning_effort_realtime;
use crate::model_profile_data::types::{Modality, ModelLimits, ModelModalities, ModelProfile};

pub(super) fn profile_data(model_id: &str) -> Option<ModelProfile> {
    match model_id {
        "gpt-realtime-2" => Some(ModelProfile {
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
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Audio],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: Some(reasoning_effort_realtime()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: true,
            supports_server_compaction: false,
            decisions: None,
        }),

        // GPT-Realtime-2.1. Source: models.dev (openai provider). Cost is left
        // unset like GPT Realtime 2: audio tokens bill separately and the
        // profile cost model is text-only.
        "gpt-realtime-2.1" => Some(ModelProfile {
            name: "GPT Realtime 2.1".into(),
            family: "gpt-realtime".into(),
            description: Some("OpenAI Realtime model for low-latency voice sessions".into()),
            release_date: Some("2026-07-06".into()),
            last_updated: Some("2026-07-06".into()),
            attachment: true,
            reasoning: true,
            temperature: false,
            knowledge: Some("2024-09-30".into()),
            tool_call: true,
            structured_output: false,
            open_weights: false,
            cost: None,
            limits: Some(ModelLimits {
                context: 128_000,
                input: Some(96_000),
                output: 32_000,
                max_media: None,
            }),
            modalities: Some(ModelModalities {
                input: vec![Modality::Text, Modality::Audio, Modality::Image],
                output: vec![Modality::Text, Modality::Audio],
            }),
            reasoning_effort: Some(reasoning_effort_realtime()),
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: Vec::new(),
            supports_phases: true,
            supports_server_compaction: false,
            decisions: None,
        }),
        _ => None,
    }
}
