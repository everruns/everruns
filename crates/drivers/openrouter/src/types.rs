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
    value.parse::<f64>().ok().map(|price| price * 1_000_000.0)
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
    fn nemotron_profile_advertises_reasoning_with_effort_levels() {
        let model = parse(NEMOTRON_JSON);
        assert!(model.is_chat_model());

        let profile = model.to_discovered_profile();
        assert!(profile.reasoning, "Nemotron must be reasoning-capable");

        let effort = profile
            .reasoning_effort
            .expect("reasoning models expose an effort config");
        assert_eq!(effort.default, ReasoningEffort::Medium);
        let values: Vec<_> = effort.values.iter().map(|v| v.value).collect();
        assert_eq!(
            values,
            vec![
                ReasoningEffort::Low,
                ReasoningEffort::Medium,
                ReasoningEffort::High
            ]
        );

        // Capabilities derived from supported_parameters.
        assert!(profile.tool_call);
        assert!(profile.structured_output);
        assert!(profile.temperature);
        assert_eq!(profile.name, "NVIDIA: Nemotron 3 Super");
        assert_eq!(profile.family, "nvidia/nemotron-3-super-120b-a12b");
        assert_eq!(
            profile.description.as_deref(),
            Some("A reasoning-capable chat model.")
        );
        assert_eq!(profile.release_date, None);
        assert_eq!(profile.knowledge.as_deref(), Some("2025-01"));
        let supported: Vec<_> = profile
            .supported_parameters
            .iter()
            .map(String::as_str)
            .collect();
        assert_eq!(
            supported,
            vec![
                "include_reasoning",
                "max_tokens",
                "reasoning",
                "response_format",
                "structured_outputs",
                "temperature",
                "tool_choice",
                "tools",
                "top_p"
            ]
        );
        let cost = profile.cost.expect("cost");
        assert!((cost.input - 0.1).abs() < 1e-9);
        assert!((cost.output - 0.4).abs() < 1e-9);
        assert!((cost.cache_read.expect("cache read") - 0.02).abs() < 1e-9);
        // Prefer the routed provider's effective context window.
        let limits = profile.limits.expect("limits");
        assert_eq!(limits.context, 262144);
        // max_completion_tokens is null here, so output falls back to context
        // rather than a misleading 0.
        assert_eq!(limits.output, 262144);
    }

    #[test]
    fn oversized_token_counts_saturate_instead_of_wrapping() {
        // Values beyond i32::MAX must clamp, not wrap to a negative number.
        let model = parse(
            r#"{
                "id": "vendor/huge-context",
                "context_length": 5000000000,
                "architecture": { "output_modalities": ["text"] },
                "supported_parameters": ["max_tokens"]
            }"#,
        );
        let limits = model.to_discovered_profile().limits.expect("limits");
        assert_eq!(limits.context, i32::MAX);
        assert_eq!(limits.output, i32::MAX);
        assert!(limits.output > 0);
    }

    #[test]
    fn non_reasoning_model_has_no_effort_config() {
        let model = parse(
            r#"{
                "id": "openai/gpt-4o-mini",
                "architecture": { "output_modalities": ["text"] },
                "supported_parameters": ["max_tokens", "temperature", "tools"]
            }"#,
        );
        let profile = model.to_discovered_profile();
        assert!(!profile.reasoning);
        assert!(profile.reasoning_effort.is_none());
        assert!(profile.tool_call);
    }

    #[test]
    fn image_only_output_models_are_excluded() {
        let model = parse(
            r#"{
                "id": "some/image-generator",
                "architecture": { "output_modalities": ["image"] },
                "supported_parameters": []
            }"#,
        );
        assert!(!model.is_chat_model());
    }

    #[test]
    fn legacy_reasoning_effort_alias_is_detected() {
        let model = parse(
            r#"{
                "id": "vendor/legacy-reasoner",
                "architecture": { "output_modalities": ["text"] },
                "supported_parameters": ["reasoning_effort", "temperature"]
            }"#,
        );
        let profile = model.to_discovered_profile();
        assert!(profile.reasoning);
        assert!(profile.reasoning_effort.is_some());
    }
}
