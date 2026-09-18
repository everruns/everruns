//! Runs one labeled case through the **real** guardrail decision path.
//!
//! The subject builds the same `guardrails` capability config a user would
//! write, asks the capability for its pre-tool-use hook, and calls that hook
//! with a `ToolContext` carrying a live engine service. Nothing about the
//! decision is reimplemented here: if the shipped code changes its mind, this
//! study changes with it.
//!
//! Two axes vary the thing under test:
//!
//! - `engine`: `utility_llm` (prompted for a JSON verdict) or `jev` (asked a
//!   typed question, thresholded in code).
//! - `threshold`: the percentage a `jev` answer must reach to block. The
//!   utility-LLM engine writes its own verdict and ignores it, so its rows are
//!   identical across threshold values — which is itself the point: one engine
//!   is tunable after the fact and the other is not.

use std::sync::Arc;

use everruns_builtins::GuardrailsCapability;
use everruns_core::capabilities::Capability;
use everruns_core::tool_context::ToolContext;
use everruns_core::tool_hooks::PreToolUseDecision;
use everruns_host::OpenAiUtilityLlmService;
use everruns_integrations_typesafe::TypeSafeClassifier;
use everruns_provider::tool_types::{
    BuiltinTool, DeferrablePolicy, ToolCall, ToolDefinition, ToolPolicy,
};
use everruns_provider::typed_id::SessionId;
use mira::{RunCx, Sample, Subject, Transcript};
use serde_json::json;

/// Transcript key holding the decision: `true` when the call was blocked.
pub const BLOCKED_KEY: &str = "blocked";
/// Transcript key marking a case that was not measured.
pub const SKIPPED_KEY: &str = "skipped";

pub const DEFAULT_ENGINE: &str = "jev";
pub const DEFAULT_THRESHOLD: &str = "50";

/// Env var holding each engine's credential.
fn credential(engine: &str) -> Option<(&'static str, String)> {
    let var = match engine {
        "jev" => "UTILITY_TYPESAFE_API_KEY",
        "utility_llm" => "UTILITY_OPENAI_API_KEY",
        _ => return None,
    };
    std::env::var(var)
        .ok()
        .filter(|key| !key.trim().is_empty())
        .map(|key| (var, key))
}

pub struct GuardrailCalibrationSubject;

#[async_trait::async_trait]
impl Subject for GuardrailCalibrationSubject {
    async fn run(&self, sample: &Sample, cx: &RunCx) -> Transcript {
        let engine = cx.param("engine").unwrap_or(DEFAULT_ENGINE).to_string();
        let threshold_param = cx.param("threshold").unwrap_or(DEFAULT_THRESHOLD);
        let Ok(threshold) = threshold_param.parse::<u8>() else {
            return Transcript::infra_error(format!(
                "threshold '{threshold_param}' is not a number"
            ));
        };
        if threshold > 100 {
            return Transcript::infra_error(format!("threshold {threshold} is not a percentage"));
        }

        let mut transcript = Transcript::default();
        transcript
            .metadata
            .insert("engine".into(), engine.clone().into());
        transcript
            .metadata
            .insert("threshold".into(), threshold_param.into());

        // An unconfigured engine is not a permissive engine — it is an
        // unmeasured one. Scoring it would report perfect precision for a
        // guardrail that never ran.
        let Some((var, key)) = credential(&engine) else {
            let reason = match engine.as_str() {
                "jev" | "utility_llm" => format!("{engine} engine has no credential configured"),
                other => format!("unknown engine '{other}'"),
            };
            transcript
                .metadata
                .insert(SKIPPED_KEY.into(), reason.into());
            return transcript;
        };
        let _ = var;

        let Some(policy) = sample.metadata.get("policy").and_then(|v| v.as_str()) else {
            return Transcript::infra_error(format!("{}: sample declares no policy", sample.id));
        };
        let arguments = sample
            .metadata
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({ "input": sample.input.join("\n") }));
        let tool_name = sample
            .metadata
            .get("tool")
            .and_then(|v| v.as_str())
            .unwrap_or("run_query");

        let config = json!({
            "checks": [{
                "id": "calibration",
                "stage": "tool_use",
                "type": "llm_judge",
                "engine": engine,
                "threshold": threshold,
                "prompt": policy,
            }]
        });

        let hooks = GuardrailsCapability.pre_tool_use_hooks_with_config(&config);
        let Some(hook) = hooks.into_iter().next() else {
            return Transcript::infra_error("guardrails contributed no pre-tool-use hook");
        };

        let context = match engine.as_str() {
            "jev" => ToolContext::new(SessionId::new())
                .with_classifier(Arc::new(TypeSafeClassifier::new(key))),
            _ => ToolContext::new(SessionId::new())
                .with_utility_llm_service(Arc::new(OpenAiUtilityLlmService::new(key))),
        };

        let call = ToolCall {
            id: "calibration".to_string(),
            name: tool_name.to_string(),
            arguments: arguments.clone(),
        };
        let decision = hook
            .before_exec(call, &tool_definition(tool_name), &context)
            .await;
        let blocked = matches!(decision, PreToolUseDecision::Block { .. });

        transcript
            .metadata
            .insert(BLOCKED_KEY.into(), blocked.into());
        transcript.final_response = if blocked { "blocked" } else { "allowed" }.to_string();
        transcript
    }
}

/// A minimal definition for the call under inspection. Guardrail judge checks
/// read the stage content and the tool name, not the schema.
fn tool_definition(name: &str) -> ToolDefinition {
    ToolDefinition::Builtin(BuiltinTool {
        name: name.to_string(),
        display_name: None,
        description: "tool under guardrail inspection".to_string(),
        parameters: json!({}),
        policy: ToolPolicy::Auto,
        category: None,
        deferrable: DeferrablePolicy::Never,
        hints: Default::default(),
        full_parameters: None,
    })
}
