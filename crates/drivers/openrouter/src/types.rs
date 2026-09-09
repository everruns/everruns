// OpenRouter Models API Types
//
// OpenRouter exposes the same OpenAI-compatible surface (`/responses`,
// `/chat/completions`) but its `/models` endpoint returns much richer metadata
// than OpenAI's bare `{id, created, owned_by}`. In particular it advertises a
// `supported_parameters` array (e.g. `reasoning`, `tools`, `response_format`)
// that lets us derive a capability profile at discovery time — most OpenRouter
// models (NVIDIA Nemotron, DeepSeek, etc.) have no hardcoded profile, so without
// this the UI cannot tell they support reasoning and hides the effort selector.

use serde::Deserialize;

/// Response from OpenRouter's `/api/v1/models` endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterModelsResponse {
    pub data: Vec<OpenRouterModelInfo>,
}

/// Individual model entry from OpenRouter's models API.
///
/// Only the fields we map are modeled; unknown fields are ignored so the parse
/// is resilient to OpenRouter adding metadata over time.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterModelInfo {
    /// Fully qualified model id (e.g. `nvidia/nemotron-3-super-120b-a12b`).
    pub id: String,
    /// Canonical model slug, when OpenRouter aliases the public id.
    #[serde(default)]
    pub canonical_slug: Option<String>,
    /// Human-readable display name (e.g. "NVIDIA: Nemotron 3 Super").
    #[serde(default)]
    pub name: Option<String>,
    /// Human-readable model description.
    #[serde(default)]
    pub description: Option<String>,
    /// Unix timestamp of when the model was added.
    #[serde(default)]
    pub created: Option<i64>,
    /// Maximum context window in tokens.
    #[serde(default)]
    pub context_length: Option<i64>,
    /// Modality metadata (input/output modalities).
    #[serde(default)]
    pub architecture: Option<OpenRouterArchitecture>,
    /// Per-token pricing reported by OpenRouter.
    #[serde(default)]
    pub pricing: Option<OpenRouterPricing>,
    /// Top-provider limits (max completion tokens, effective context).
    #[serde(default)]
    pub top_provider: Option<OpenRouterTopProvider>,
    /// Parameters the model/router accepts. The presence of `reasoning`
    /// (or `reasoning_effort`) is what tells us reasoning is supported.
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    /// Provider-reported knowledge cutoff, when available.
    #[serde(default)]
    pub knowledge_cutoff: Option<String>,
}

/// Modality metadata from OpenRouter's `architecture` object.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterArchitecture {
    #[serde(default)]
    pub input_modalities: Vec<String>,
    #[serde(default)]
    pub output_modalities: Vec<String>,
}

/// Per-token pricing reported by OpenRouter's `pricing` object.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterPricing {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub completion: Option<String>,
    #[serde(default)]
    pub input_cache_read: Option<String>,
    #[serde(default)]
    pub cache_read: Option<String>,
}

/// Per-request limits reported by the routed upstream provider.
#[derive(Debug, Clone, Deserialize)]
pub struct OpenRouterTopProvider {
    #[serde(default)]
    pub context_length: Option<i64>,
    #[serde(default)]
    pub max_completion_tokens: Option<i64>,
}

impl OpenRouterModelInfo {
    /// Whether the model supports a given parameter (case-insensitive).
    fn supports(&self, param: &str) -> bool {
        self.supported_parameters
            .iter()
            .any(|p| p.eq_ignore_ascii_case(param))
    }

    /// Whether this is a text-producing chat model. OpenRouter also lists
    /// image-only / non-text-output models, which we exclude from discovery.
    pub fn is_chat_model(&self) -> bool {
        match self.architecture.as_ref() {
            Some(arch) if !arch.output_modalities.is_empty() => arch
                .output_modalities
                .iter()
                .any(|m| m.eq_ignore_ascii_case("text")),
            // No modality metadata — assume chat (OpenRouter's catalog is
            // overwhelmingly text-output chat models).
            _ => true,
        }
    }

    /// Build a [`ModelProfile`](everruns_provider::model::ModelProfile) from
    /// OpenRouter's advertised metadata.
    pub fn to_discovered_profile(&self) -> everruns_provider::model::ModelProfile {
        use everruns_provider::model::*;

        // Reasoning is advertised via the `reasoning` parameter (modern unified
        // control) or the legacy `reasoning_effort` alias.
        let reasoning = self.supports("reasoning") || self.supports("reasoning_effort");
        let reasoning_effort = reasoning.then(openrouter_effort_config);

        // Modalities → attachment support.
        let input_modalities = self
            .architecture
            .as_ref()
            .map(|a| a.input_modalities.as_slice())
            .unwrap_or(&[]);
        let has_modality = |name: &str| {
            input_modalities
                .iter()
                .any(|m| m.eq_ignore_ascii_case(name))
        };
        let image_input = has_modality("image");
        let pdf_input = has_modality("file") || has_modality("pdf");

        let modalities = {
            let mut input = vec![Modality::Text];
            if image_input {
                input.push(Modality::Image);
            }
            if pdf_input {
                input.push(Modality::Pdf);
            }
            Some(ModelModalities {
                input,
                output: vec![Modality::Text],
            })
        };

        // Prefer the routed provider's effective context window when present.
        let context = self
            .top_provider
            .as_ref()
            .and_then(|t| t.context_length)
            .or(self.context_length);
        let max_output = self
            .top_provider
            .as_ref()
            .and_then(|t| t.max_completion_tokens);
        let limits = context.map(|ctx| {
            let context = clamp_i64_to_i32(ctx);
            ModelLimits {
                context,
                input: None,
                // OpenRouter commonly omits a separate output cap
                // (`max_completion_tokens: null`). Fall back to the context
                // window — the model's theoretical max output — rather than
                // emitting a misleading `0` that renders as "Output: 0".
                output: max_output.map(clamp_i64_to_i32).unwrap_or(context),
                max_media: None,
            }
        });

        ModelProfile {
            name: self.name.clone().unwrap_or_else(|| self.id.clone()),
            family: self
                .canonical_slug
                .clone()
                .unwrap_or_else(|| self.id.clone()),
            description: self.description.clone(),
            release_date: None,
            last_updated: None,
            attachment: image_input || pdf_input,
            reasoning,
            temperature: self.supports("temperature"),
            knowledge: self.knowledge_cutoff.clone(),
            tool_call: self.supports("tools") || self.supports("tool_choice"),
            structured_output: self.supports("structured_outputs")
                || self.supports("response_format"),
            open_weights: false,
            cost: self.pricing.as_ref().and_then(OpenRouterPricing::to_cost),
            limits,
            modalities,
            reasoning_effort,
            speed: None,
            verbosity: None,
            tool_search: false,
            supported_parameters: self.supported_parameters.clone(),
            supports_phases: false,
        }
    }
}

impl OpenRouterPricing {
    fn to_cost(&self) -> Option<everruns_provider::model::ModelCost> {
        let input = openrouter_price_per_million(self.prompt.as_deref()?)?;
        let output = openrouter_price_per_million(self.completion.as_deref()?)?;
        let cache_read = self
            .input_cache_read
            .as_deref()
            .or(self.cache_read.as_deref())
            .and_then(openrouter_price_per_million);

        Some(everruns_provider::model::ModelCost {
            input,
            output,
            cache_read,
            cost_tiers: Vec::new(),
        })
    }
}

/// Saturating `i64` → `i32` conversion.
///
/// OpenRouter reports token counts as `i64`, while `ModelLimits` stores
/// `i32`. A raw `as` cast wraps on overflow (producing negative limits); this
/// clamps into range instead. Negative inputs clamp to `0`.
fn clamp_i64_to_i32(value: i64) -> i32 {
    value.clamp(0, i32::MAX as i64) as i32
}

fn openrouter_price_per_million(value: &str) -> Option<f64> {
    value
        .parse::<f64>()
        .ok()
        .map(|price| price * 1_000_000.0)
        // Check after scaling too: a finite per-token value can overflow.
        .filter(|price| price.is_finite() && *price >= 0.0)
}

/// Standard low/medium/high effort config for OpenRouter reasoning models.
///
/// OpenRouter normalizes `reasoning.effort` ∈ {low, medium, high} across upstream
/// providers, so we expose those three with `medium` as the default.
fn openrouter_effort_config() -> everruns_provider::model::ReasoningEffortConfig {
    use everruns_provider::model::*;
    ReasoningEffortConfig {
        values: vec![
            ReasoningEffortValue {
                value: ReasoningEffort::Low,
                name: "Low".into(),
            },
            ReasoningEffortValue {
                value: ReasoningEffort::Medium,
                name: "Medium".into(),
            },
            ReasoningEffortValue {
                value: ReasoningEffort::High,
                name: "High".into(),
            },
        ],
        default: ReasoningEffort::Medium,
    }
}

#[cfg(test)]
mod openrouter_tests {
    use super::*;
    use everruns_provider::model::ReasoningEffort;

    // Trimmed copy of the live `nvidia/nemotron-3-super-120b-a12b` entry from
    // OpenRouter's /api/v1/models response.
    const NEMOTRON_JSON: &str = r#"{
        "id": "nvidia/nemotron-3-super-120b-a12b",
        "canonical_slug": "nvidia/nemotron-3-super-120b-a12b",
        "name": "NVIDIA: Nemotron 3 Super",
        "description": "A reasoning-capable chat model.",
        "created": 1773245239,
        "context_length": 1000000,
        "architecture": {
            "input_modalities": ["text"],
            "output_modalities": ["text"]
        },
        "pricing": {
            "prompt": "0.00000010",
            "completion": "0.00000040",
            "input_cache_read": "0.00000002"
        },
        "top_provider": { "context_length": 262144, "max_completion_tokens": null },
        "supported_parameters": [
            "include_reasoning", "max_tokens", "reasoning", "response_format",
            "structured_outputs", "temperature", "tool_choice", "tools", "top_p"
        ],
        "knowledge_cutoff": "2025-01"
    }"#;

    fn parse(json: &str) -> OpenRouterModelInfo {
        serde_json::from_str(json).expect("parse model")
    }

    #[test]
    fn catalog_metadata_maps_to_a_complete_profile() {
        let model = parse(NEMOTRON_JSON);
        assert!(model.is_chat_model());
        let mut profile = model.to_discovered_profile();
        let cost = profile.cost.take().unwrap();
        assert!((cost.input - 0.1).abs() < 1e-9);
        assert!((cost.output - 0.4).abs() < 1e-9);
        assert!((cost.cache_read.unwrap() - 0.02).abs() < 1e-9);
        assert!(cost.cost_tiers.is_empty());
        assert_eq!(
            serde_json::to_value(profile).unwrap(),
            serde_json::json!({
                "name":"NVIDIA: Nemotron 3 Super", "family":"nvidia/nemotron-3-super-120b-a12b",
                "description":"A reasoning-capable chat model.", "attachment":false,
                "reasoning":true, "temperature":true, "knowledge":"2025-01",
                "tool_call":true,"structured_output":true,"open_weights":false,
                "limits":{"context":262144,"output":262144},
                "modalities":{"input":["text"],"output":["text"]},
                "reasoning_effort":{"values":[{"value":"low","name":"Low"},{"value":"medium","name":"Medium"},{"value":"high","name":"High"}],"default":"medium"},
                "tool_search":false,"supports_phases":false,
                "supported_parameters":["include_reasoning","max_tokens","reasoning","response_format","structured_outputs","temperature","tool_choice","tools","top_p"]
            })
        );
    }

    #[test]
    fn capability_aliases_and_modalities_do_not_invent_missing_features() {
        for (metadata, chat, expected) in [
            (
                serde_json::json!({}),
                true,
                serde_json::json!([false, false, false, false, false, ["text"]]),
            ),
            (
                serde_json::json!({"architecture":{"output_modalities":["image"]}}),
                false,
                serde_json::json!([false, false, false, false, false, ["text"]]),
            ),
            (
                serde_json::json!({"architecture":{"output_modalities":[]},"supported_parameters":["max_tokens","temperature","tools"]}),
                true,
                serde_json::json!([false, true, true, false, false, ["text"]]),
            ),
            (
                serde_json::json!({"architecture":{"input_modalities":["IMAGE","file","pdf"],"output_modalities":["image","TEXT"]},"supported_parameters":["REASONING_EFFORT","TOOL_CHOICE","RESPONSE_FORMAT"]}),
                true,
                serde_json::json!([true, false, true, true, true, ["text", "image", "pdf"]]),
            ),
        ] {
            let mut input = metadata;
            input["id"] = serde_json::json!("vendor/model");
            let model: OpenRouterModelInfo = serde_json::from_value(input).unwrap();
            assert_eq!(model.is_chat_model(), chat);
            let profile = model.to_discovered_profile();
            assert_eq!(profile.name, "vendor/model");
            assert_eq!(profile.family, "vendor/model");
            assert_eq!(
                serde_json::json!([
                    profile.reasoning,
                    profile.temperature,
                    profile.tool_call,
                    profile.structured_output,
                    profile.attachment,
                    profile.modalities.unwrap().input
                ]),
                expected
            );
            assert_eq!(
                profile.reasoning_effort.map(|effort| effort.default),
                profile.reasoning.then_some(ReasoningEffort::Medium)
            );
            assert!(profile.cost.is_none());
            assert!(profile.limits.is_none());
        }
    }

    #[test]
    fn effective_token_limits_saturate_and_keep_explicit_output_caps() {
        for (context, top, expected) in [
            (None, serde_json::json!({}), None),
            (Some(-1_i64), serde_json::json!({}), Some((0, 0))),
            (
                Some(5_000_000_000),
                serde_json::json!({}),
                Some((i32::MAX, i32::MAX)),
            ),
            (
                Some(100),
                serde_json::json!({"context_length":80,"max_completion_tokens":20}),
                Some((80, 20)),
            ),
            (
                Some(100),
                serde_json::json!({"context_length":80,"max_completion_tokens":-1}),
                Some((80, 0)),
            ),
            (
                Some(100),
                serde_json::json!({"max_completion_tokens":5_000_000_000_i64}),
                Some((100, i32::MAX)),
            ),
            (
                Some(i32::MAX as i64),
                serde_json::json!({}),
                Some((i32::MAX, i32::MAX)),
            ),
        ] {
            let model: OpenRouterModelInfo = serde_json::from_value(serde_json::json!({"id":"vendor/model","context_length":context,"top_provider":top})).unwrap();
            assert_eq!(
                model.to_discovered_profile().limits.map(|l| {
                    assert!(l.input.is_none());
                    assert!(l.max_media.is_none());
                    (l.context, l.output)
                }),
                expected
            );
        }
    }

    #[test]
    fn catalog_prices_are_finite_nonnegative_and_optional_cache_prices_stay_optional() {
        let profile = |pricing| {
            serde_json::from_value::<OpenRouterModelInfo>(
                serde_json::json!({"id":"vendor/model","pricing":pricing}),
            )
            .unwrap()
            .to_discovered_profile()
        };
        for invalid in ["NaN", "inf", "-inf", "-0.01", "1e308", "not-a-price"] {
            for field in ["prompt", "completion"] {
                let mut pricing = serde_json::json!({"prompt":"0.000001","completion":"0.000002"});
                pricing[field] = serde_json::json!(invalid);
                assert!(
                    profile(pricing).cost.is_none(),
                    "invalid {field}: {invalid}"
                );
            }
            let pricing = serde_json::json!({"prompt":"0.000001","completion":"0.000002","input_cache_read":invalid});
            let cost = profile(pricing).cost.unwrap();
            assert_eq!((cost.input, cost.output, cost.cache_read), (1.0, 2.0, None));
        }
        for pricing in [
            serde_json::json!({}),
            serde_json::json!({"prompt":"0"}),
            serde_json::json!({"completion":"0"}),
        ] {
            assert!(profile(pricing).cost.is_none());
        }
        for (pricing, expected) in [
            (
                serde_json::json!({"prompt":"0","completion":"0"}),
                (0.0, 0.0, None),
            ),
            (
                serde_json::json!({"prompt":"1e-6","completion":"2e-6","cache_read":"0.0000005"}),
                (1.0, 2.0, Some(0.5)),
            ),
            (
                serde_json::json!({"prompt":"1e-6","completion":"2e-6","cache_read":"0.0000005","input_cache_read":"0.00000025"}),
                (1.0, 2.0, Some(0.25)),
            ),
        ] {
            let cost = profile(pricing).cost.unwrap();
            assert_eq!((cost.input, cost.output, cost.cache_read), expected);
            assert!(cost.cost_tiers.is_empty());
        }
    }
}
