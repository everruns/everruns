// LLM-as-judge for observer scoring. See knowledge/evaluation/online-evals.md.
//
// The judge grades a trace slice against a rubric and returns a 0.0–1.0 value,
// an optional categorical label, and free-text reasoning. Decisions calls go
// through the ORG's own configured model/provider (resolved via
// ProviderResolverService) and are billed to that org.
//
// Prompt construction and response parsing are pure functions (unit-tested
// here); the LLM call sits behind the `JudgeClient` trait so the worker stays
// testable with a fake.

use anyhow::Result;
use async_trait::async_trait;
use std::sync::Arc;

use crate::kernel_imports::{
    contracts::driver_registry::DriverRegistry, contracts::driver_registry::LlmCallConfig,
    contracts::driver_registry::Message, contracts::driver_registry::MessageRole,
    contracts::driver_registry::ProviderConfig,
};
use everruns_contracts::provider::DriverId;
use everruns_contracts::typed_id::ModelId;

use crate::services::ProviderResolverService;

/// Extra output tokens a reasoning model gets for its reasoning, on top of
/// the answer's budget.
const REASONING_HEADROOM_TOKENS: u32 = 4000;
use crate::storage::StorageBackend;

pub use everruns_core::evaluation_judge::{JudgeOutput, JudgeResult, TurnEvidence};

/// Abstraction over the judge LLM call so the worker is testable with a fake.
#[async_trait]
pub trait JudgeClient: Send + Sync {
    /// Bounded system assessment. Implementations without admission support
    /// leave the check unassessed rather than falling back to an unmetered call.
    async fn judge_compaction(
        &self,
        _org_id: i64,
        _model_id: ModelId,
        _system: String,
        _user: String,
        _admission: MonitoringAdmission<'_>,
    ) -> Result<Option<OrgCompletion>> {
        Ok(None)
    }
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

pub use everruns_core::evaluation_judge::{build_judge_messages, parse_judge_output};

/// Production [`JudgeClient`] that calls the org's configured model.
pub struct LlmJudgeClient {
    db: Arc<StorageBackend>,
    driver_registry: Arc<DriverRegistry>,
    provider_resolver: Arc<ProviderResolverService>,
}

impl LlmJudgeClient {
    pub fn new(
        db: Arc<StorageBackend>,
        driver_registry: Arc<DriverRegistry>,
        provider_resolver: Arc<ProviderResolverService>,
    ) -> Self {
        Self {
            db,
            driver_registry,
            provider_resolver,
        }
    }
}

/// One completion from an org's own model: the text and what it cost.
#[derive(Debug, Clone)]
pub struct OrgCompletion {
    pub text: String,
    /// The provider model that answered.
    pub model: String,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cost_usd: Option<f64>,
}

/// Sampling and attribution for one [`LlmJudgeClient::complete`] call.
#[derive(Debug, Clone, Copy)]
pub struct CompletionOptions<'a> {
    pub monitoring: Option<MonitoringAdmission<'a>>,
    pub temperature: f32,
    pub max_tokens: u32,
    /// Request metadata naming the caller.
    pub purpose: &'a str,
}

#[derive(Debug, Clone, Copy)]
pub struct MonitoringAdmission<'a> {
    pub score_id: uuid::Uuid,
    pub attempt: i32,
    pub policy: &'a super::system::QualityMonitoringPolicy,
}

#[derive(Debug, thiserror::Error)]
#[error("Monitoring call was not admitted: consent, pricing or daily budget is unavailable")]
pub struct MonitoringNotAdmitted;

impl LlmJudgeClient {
    /// Ask `model_id` (the org's default model when `None`) to answer
    /// `system` and `user`. `Ok(None)` when the org has no usable model, a
    /// lasting condition. Shared with the eval user simulator, which plays a
    /// user through the same resolution and billing.
    pub async fn complete(
        &self,
        org_id: i64,
        model_id: Option<ModelId>,
        system: String,
        user: String,
        options: CompletionOptions<'_>,
    ) -> Result<Option<OrgCompletion>> {
        let input_token_bound =
            u32::try_from(system.len().saturating_add(user.len()).saturating_add(1024))
                .unwrap_or(u32::MAX);
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
        let mut provider_config = ProviderConfig::for_provider(
            everruns_contracts::runtime_provider::ProviderKey::new(&resolved.provider_id),
            driver_id.clone(),
        );
        provider_config.api_key = Some(runtime_provider.credentials.api_key);
        provider_config.base_url = runtime_provider.credentials.base_url;
        provider_config.request_options = runtime_provider.request_options;
        provider_config.api = runtime_provider.api;
        let driver = self.driver_registry.create_chat_driver(&provider_config)?;

        let messages = vec![
            Message::text(MessageRole::System, system),
            Message::text(MessageRole::User, user),
        ];
        // Reasoning models (GPT-6 Luna, Sol) reject `temperature`, and spend
        // output tokens on reasoning before the answer: send temperature only
        // where the profile accepts it, and give reasoning models low effort
        // plus headroom so the short answer is not cut off.
        let profile =
            everruns_contracts::model_profiles::get_model_profile(&driver_id, &resolved.model_id);
        let model = resolved.model_id.clone();
        let mut config = LlmCallConfig::new(resolved.model_id);
        config.max_tokens = Some(options.max_tokens);
        if profile.as_ref().is_none_or(|p| p.temperature) {
            config.temperature = Some(options.temperature);
        }
        if profile.as_ref().is_some_and(|p| p.reasoning) {
            config.reasoning_effort = Some(everruns_contracts::ReasoningEffort::Low);
            config.max_tokens = Some(options.max_tokens + REASONING_HEADROOM_TOKENS);
        }
        config.metadata = std::collections::HashMap::from([
            ("org_id".to_string(), org_id.to_string()),
            ("scorer".to_string(), options.purpose.to_string()),
        ]);

        if let Some(admission) = options.monitoring {
            config.speed = Some("default".into());
            // Include the driver's bounded retry allowance and an auth retry.
            // Holds remain conservative when provider outcomes are ambiguous.
            let cost = profile
                .as_ref()
                .and_then(|profile| profile.cost.as_ref())
                .ok_or(MonitoringNotAdmitted)?;
            let input_rate = cost
                .cost_tiers
                .iter()
                .map(|tier| tier.input)
                .fold(cost.input, f64::max);
            let output_rate = cost
                .cost_tiers
                .iter()
                .map(|tier| tier.output)
                .fold(cost.output, f64::max);
            if !input_rate.is_finite()
                || input_rate < 0.0
                || !output_rate.is_finite()
                || output_rate < 0.0
            {
                return Err(MonitoringNotAdmitted.into());
            }
            let calls =
                f64::from((everruns_contracts::LlmRetryConfig::default().max_retries + 1) * 2);
            let hold = ((f64::from(input_token_bound) * input_rate
                + f64::from(config.max_tokens.unwrap_or(0)) * output_rate)
                * calls)
                .ceil();
            if !hold.is_finite() || hold < 0.0 || hold > i64::MAX as f64 {
                return Err(MonitoringNotAdmitted.into());
            }
            let allowed = self
                .db
                .reserve_monitoring_budget(
                    org_id,
                    admission.score_id,
                    admission.attempt,
                    &serde_json::to_value(admission.policy)?,
                    (hold as i64).max(1),
                )
                .await?;
            if !allowed {
                return Err(MonitoringNotAdmitted.into());
            }
        }

        let endpoint = everruns_contracts::runtime_provider::ProviderEndpoint::default();
        let completion = driver.chat_completion(&endpoint, messages, &config);
        // Finish before the scoring lease can be reclaimed. A timed-out call
        // keeps its conservative reservation because its billing is uncertain.
        let response = if options.monitoring.is_some() {
            tokio::time::timeout(std::time::Duration::from_secs(90), completion).await??
        } else {
            completion.await?
        };
        let meta = response.metadata;
        if let Some(admission) = options.monitoring {
            let actual = meta
                .provider_cost_usd
                .filter(|cost| cost.is_finite() && *cost >= 0.0)
                .map(|cost| (cost * 1_000_000.0).ceil() as i64);
            self.db
                .settle_monitoring_budget(org_id, admission.score_id, admission.attempt, actual)
                .await?;
            let usage_id =
                uuid::Uuid::new_v5(&admission.score_id, &admission.attempt.to_be_bytes());
            // Usage is attributed per admitted attempt, including responses
            // whose judgment cannot subsequently be parsed or accepted.
            if let Some(session) = self
                .db
                .get_trace_score_session(admission.score_id, org_id)
                .await?
            {
                crate::domains::budgets::BudgetService::new(self.db.clone())
                    .meter_observer_grading(
                        session,
                        usage_id,
                        meta.prompt_tokens.unwrap_or(0) as u64,
                        meta.completion_tokens.unwrap_or(0) as u64,
                        meta.provider_cost_usd,
                    )
                    .await;
            }
        }
        Ok(Some(OrgCompletion {
            text: response.text,
            model,
            input_tokens: meta.prompt_tokens.map(|t| t as u64),
            output_tokens: meta.completion_tokens.map(|t| t as u64),
            cost_usd: meta.provider_cost_usd,
        }))
    }
}

#[async_trait]
impl JudgeClient for LlmJudgeClient {
    async fn judge_compaction(
        &self,
        org_id: i64,
        model_id: ModelId,
        system: String,
        user: String,
        admission: MonitoringAdmission<'_>,
    ) -> Result<Option<OrgCompletion>> {
        self.complete(
            org_id,
            Some(model_id),
            system,
            user,
            CompletionOptions {
                monitoring: Some(admission),
                temperature: 0.0,
                max_tokens: 700,
                purpose: "compaction_quality_monitoring",
            },
        )
        .await
    }
    async fn judge(
        &self,
        org_id: i64,
        model_id: Option<ModelId>,
        rubric: &str,
        evidence: &TurnEvidence,
    ) -> Result<Option<JudgeResult>> {
        let (system, user) = build_judge_messages(rubric, evidence);
        let Some(response) = self
            .complete(
                org_id,
                model_id,
                system,
                user,
                CompletionOptions {
                    monitoring: None,
                    temperature: 0.0,
                    max_tokens: 700,
                    purpose: "observer_llm_judge",
                },
            )
            .await?
        else {
            return Ok(None);
        };
        let parsed = parse_judge_output(&response.text)?;
        Ok(Some(JudgeResult {
            value: parsed.value,
            label: parsed.label,
            reasoning: parsed.reasoning,
            input_tokens: response.input_tokens,
            output_tokens: response.output_tokens,
            cost_usd: response.cost_usd,
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
