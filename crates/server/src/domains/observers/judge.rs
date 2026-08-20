// LLM-as-judge for observer scoring. See knowledge/evaluation/online-evals.md.
//
// The judge grades a trace slice against a rubric and returns a 0.0–1.0 value,
// an optional categorical label, and free-text reasoning. Judge calls go
// through the ORG's own configured model/provider (resolved via
// ProviderResolverService) and are billed to that org.
//
// Prompt construction and response parsing are pure functions (unit-tested
// here); the LLM call sits behind the `JudgeClient` trait so the worker stays
// testable with a fake.

use anyhow::{Context, Result};
use async_trait::async_trait;
use std::sync::Arc;

use crate::kernel_imports::{
    everruns_provider::driver_registry::DriverRegistry,
    everruns_provider::driver_registry::LlmCallConfig,
    everruns_provider::driver_registry::LlmMessage,
    everruns_provider::driver_registry::LlmMessageRole,
    everruns_provider::driver_registry::ProviderConfig,
};
use everruns_provider::provider::DriverId;
use everruns_provider::typed_id::ModelId;

use crate::services::ProviderResolverService;
use crate::storage::StorageBackend;

/// Parsed structured output from the judge model.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct JudgeOutput {
    /// Score 0.0–1.0.
    pub value: f64,
    /// Optional categorical label.
    #[serde(default)]
    pub label: Option<String>,
    /// Reasoning for the score.
    #[serde(default)]
    pub reasoning: String,
}

/// Result of a judge call: the parsed grade plus token/cost accounting.
#[derive(Debug, Clone)]
pub struct JudgeResult {
    pub value: f64,
    pub label: Option<String>,
    pub reasoning: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// What the judge grades: the user's question, the agent's answer, and a
/// summary of tools used in the turn.
#[derive(Debug, Clone, Default)]
pub struct TurnEvidence {
    pub input_message: String,
    pub final_answer: String,
    pub tool_names: Vec<String>,
}

/// Abstraction over the judge LLM call so the worker is testable with a fake.
#[async_trait]
pub trait JudgeClient: Send + Sync {
    /// Grade `evidence` against `rubric` for `org_id`, using `model_id` (or the
    /// org's default model when `None`).
    ///
    /// Returns `Ok(None)` when the org has no judge model configured — a
    /// permanent, org-specific condition the worker should treat as `skipped`
    /// rather than a retryable error. `Err` is reserved for transient/real
    /// failures (provider errors, unparseable responses) that warrant a retry.
    async fn judge(
        &self,
        org_id: i64,
        model_id: Option<ModelId>,
        rubric: &str,
        evidence: &TurnEvidence,
    ) -> Result<Option<JudgeResult>>;
}

const JUDGE_SYSTEM_PREAMBLE: &str = "You are an evaluation judge scoring an AI agent's response. \
Apply the rubric and respond with ONLY a JSON object of the form \
{\"value\": <number 0.0-1.0>, \"label\": <short string or null>, \"reasoning\": <string>}. \
`value` is the score where 1.0 fully satisfies the rubric and 0.0 fails it. \
`label` is an optional short category for the outcome. Do not include any text outside the JSON.";

/// Build (system, user) messages for the judge from the rubric and evidence.
pub fn build_judge_messages(rubric: &str, evidence: &TurnEvidence) -> (String, String) {
    let system = format!("{JUDGE_SYSTEM_PREAMBLE}\n\nRubric:\n{rubric}");

    let tools = if evidence.tool_names.is_empty() {
        "(none)".to_string()
    } else {
        evidence.tool_names.join(", ")
    };
    let user = format!(
        "User message:\n{}\n\nAgent's final answer:\n{}\n\nTools used: {}",
        truncate(&evidence.input_message, 4000),
        truncate(&evidence.final_answer, 8000),
        tools,
    );
    (system, user)
}

/// Parse the judge model's response into a [`JudgeOutput`]. Tolerates code
/// fences and surrounding prose by extracting the first balanced JSON object.
pub fn parse_judge_output(text: &str) -> Result<JudgeOutput> {
    let json_slice = extract_json_object(text)
        .ok_or_else(|| anyhow::anyhow!("judge response contained no JSON object"))?;
    let mut out: JudgeOutput = serde_json::from_str(json_slice)
        .context("judge response was not valid JudgeOutput JSON")?;
    // Clamp to the contract range so a misbehaving judge can't store nonsense.
    out.value = out.value.clamp(0.0, 1.0);
    out.label = out.label.filter(|l| !l.trim().is_empty());
    Ok(out)
}

/// Find the first balanced `{...}` object in `text` (handles ```json fences
/// and chatty models). Naive brace matching is sufficient for judge outputs.
fn extract_json_object(text: &str) -> Option<&str> {
    let start = text.find('{')?;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (i, ch) in text[start..].char_indices() {
        match ch {
            '"' if !escaped => in_string = !in_string,
            '\\' if in_string => {
                escaped = !escaped;
                continue;
            }
            '{' if !in_string => depth += 1,
            '}' if !in_string => {
                depth -= 1;
                if depth == 0 {
                    return Some(&text[start..=start + i]);
                }
            }
            _ => {}
        }
        escaped = false;
    }
    None
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push_str("…[truncated]");
    out
}

/// Production [`JudgeClient`] that calls the org's configured model.
pub struct LlmJudgeClient {
    driver_registry: Arc<DriverRegistry>,
    provider_resolver: Arc<ProviderResolverService>,
}

impl LlmJudgeClient {
    pub fn new(
        _db: Arc<StorageBackend>,
        driver_registry: Arc<DriverRegistry>,
        provider_resolver: Arc<ProviderResolverService>,
    ) -> Self {
        Self {
            driver_registry,
            provider_resolver,
        }
    }
}

#[async_trait]
impl JudgeClient for LlmJudgeClient {
    async fn judge(
        &self,
        org_id: i64,
        model_id: Option<ModelId>,
        rubric: &str,
        evidence: &TurnEvidence,
    ) -> Result<Option<JudgeResult>> {
        // Resolve the org's model + decrypted provider credentials. A missing
        // model is a permanent config condition → Ok(None) so the worker skips
        // (rather than retrying and finally erroring).
        let resolved = match model_id {
            Some(id) => {
                self.provider_resolver
                    .resolve_model(org_id, id.uuid())
                    .await?
            }
            None => self.provider_resolver.resolve_default_model(org_id).await?,
        };
        let Some(resolved) = resolved else {
            return Ok(None);
        };

        // DriverId::from_str is infallible (unknown ids become External).
        let driver_id: DriverId = resolved.provider_type.parse().unwrap();
        let Some(runtime_provider) = self
            .provider_resolver
            .resolve_runtime_provider(org_id, &resolved.provider_id)
            .await?
        else {
            return Ok(None);
        };
        let provider_config = ProviderConfig {
            provider: everruns_provider::runtime_provider::ProviderKey::new(&resolved.provider_id),
            provider_type: driver_id,
            api_key: Some(runtime_provider.credentials.api_key),
            base_url: runtime_provider.credentials.base_url,
            metadata: Default::default(),
            request_options: runtime_provider.request_options,
        };
        let driver = self.driver_registry.create_chat_driver(&provider_config)?;

        let (system, user) = build_judge_messages(rubric, evidence);
        let messages = vec![
            LlmMessage::text(LlmMessageRole::System, system),
            LlmMessage::text(LlmMessageRole::User, user),
        ];
        let config = LlmCallConfig {
            speed: None,
            verbosity: None,
            model: resolved.model_id,
            temperature: Some(0.0),
            max_tokens: Some(700),
            tools: Vec::new(),
            reasoning_effort: None,
            metadata: std::collections::HashMap::from([
                ("org_id".to_string(), org_id.to_string()),
                ("scorer".to_string(), "observer_llm_judge".to_string()),
            ]),
            previous_response_id: None,
            provider_opaque_context: None,
            tool_search: None,
            prompt_cache: None,
            openrouter_routing: None,
            parallel_tool_calls: None,
            volatile_suffix_len: 0,
            extra_headers: Vec::new(),
            cache_diagnostics: None,
        };

        let response = driver
            .chat_completion(
                &everruns_provider::runtime_provider::ProviderEndpoint::default(),
                messages,
                &config,
            )
            .await?;
        let parsed = parse_judge_output(&response.text)?;
        let meta = response.metadata;
        Ok(Some(JudgeResult {
            value: parsed.value,
            label: parsed.label,
            reasoning: parsed.reasoning,
            input_tokens: meta.prompt_tokens.map(|t| t as u64),
            output_tokens: meta.completion_tokens.map(|t| t as u64),
            cost_usd: meta.provider_cost_usd,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_json() {
        let out =
            parse_judge_output(r#"{"value": 0.8, "label": "ok", "reasoning": "good"}"#).unwrap();
        assert_eq!(out.value, 0.8);
        assert_eq!(out.label.as_deref(), Some("ok"));
        assert_eq!(out.reasoning, "good");
    }

    #[test]
    fn parses_fenced_json_with_prose() {
        let text = "Here is my assessment:\n```json\n{\"value\": 1.0, \"reasoning\": \"complete\"}\n```\nDone.";
        let out = parse_judge_output(text).unwrap();
        assert_eq!(out.value, 1.0);
        assert!(out.label.is_none());
        assert_eq!(out.reasoning, "complete");
    }

    #[test]
    fn clamps_out_of_range_value() {
        let out = parse_judge_output(r#"{"value": 1.7, "reasoning": "x"}"#).unwrap();
        assert_eq!(out.value, 1.0);
        let out = parse_judge_output(r#"{"value": -0.3, "reasoning": "x"}"#).unwrap();
        assert_eq!(out.value, 0.0);
    }

    #[test]
    fn empty_label_becomes_none() {
        let out = parse_judge_output(r#"{"value": 0.5, "label": "  ", "reasoning": "x"}"#).unwrap();
        assert!(out.label.is_none());
    }

    #[test]
    fn errors_on_no_json() {
        assert!(parse_judge_output("the answer was great").is_err());
    }

    #[test]
    fn handles_braces_inside_strings() {
        let out =
            parse_judge_output(r#"{"value": 0.5, "reasoning": "uses {curly} braces"}"#).unwrap();
        assert_eq!(out.reasoning, "uses {curly} braces");
    }

    #[test]
    fn build_messages_includes_rubric_and_evidence() {
        let evidence = TurnEvidence {
            input_message: "What is 2+2?".into(),
            final_answer: "4".into(),
            tool_names: vec!["calc".into()],
        };
        let (system, user) = build_judge_messages("Score correctness", &evidence);
        assert!(system.contains("Score correctness"));
        assert!(system.contains("JSON"));
        assert!(user.contains("What is 2+2?"));
        assert!(user.contains("4"));
        assert!(user.contains("calc"));
    }

    #[test]
    fn build_messages_handles_no_tools() {
        let evidence = TurnEvidence {
            input_message: "hi".into(),
            final_answer: "hello".into(),
            tool_names: vec![],
        };
        let (_system, user) = build_judge_messages("r", &evidence);
        assert!(user.contains("Tools used: (none)"));
    }
}
