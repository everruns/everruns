// Agent health checks (knowledge/evaluation/agent-checks.md, tier-3): generate behavioral
// smoke cases from an agent config, run them as real sessions, score with
// deterministic checks plus an LLM judge, and persist the run.

pub mod commands;
pub mod generate;
pub mod runner;
pub mod types;

pub use commands::AgentHealthCheckService;
pub use runner::HealthCheckRunContext;

/// Escape XML metacharacters so untrusted config text cannot forge the wrapper
/// tags that mark it as data in utility-LLM prompts (TM-LLM hardening).
pub(crate) fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Strip a single ```/```json fenced block, if present.
pub(crate) fn strip_code_fences(raw: &str) -> &str {
    let trimmed = raw.trim();
    let Some(inner) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let inner = inner.strip_prefix("json").unwrap_or(inner);
    inner.strip_suffix("```").unwrap_or(inner).trim()
}

#[cfg(test)]
mod error_tests {
    #[test]
    fn provider_failure_is_safe_for_persisted_health_run() {
        let error = super::super::safe_agent_check_error(
            "case generation failed: LLM error: credit_balance_exhausted; token=secret",
        );

        assert_eq!(error.code, "provider_quota_exhausted");
        let message = error.fallback_message();
        assert!(message.contains("out of credits or quota"));
        assert!(!message.contains("token"));
        assert!(!message.contains("secret"));
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use async_trait::async_trait;
    use everruns_core::{UtilityLlmRequest, UtilityLlmService};
    use everruns_provider::driver_registry::{
        LlmCompletionMetadata, LlmResponse, LlmResponseStream,
    };
    use everruns_provider::error::{AgentLoopError, Result};
    use std::sync::Arc;

    /// Utility LLM stub that returns a fixed completion text.
    pub struct FixedLlm(pub String);

    pub fn mock_llm(text: &str) -> Arc<dyn UtilityLlmService> {
        Arc::new(FixedLlm(text.to_string()))
    }

    #[async_trait]
    impl UtilityLlmService for FixedLlm {
        fn is_configured(&self) -> bool {
            true
        }
        async fn chat_completion(&self, _request: UtilityLlmRequest) -> Result<LlmResponse> {
            Ok(LlmResponse {
                text: self.0.clone(),
                tool_calls: None,
                metadata: LlmCompletionMetadata::default(),
            })
        }
        async fn chat_completion_stream(
            &self,
            _request: UtilityLlmRequest,
        ) -> Result<LlmResponseStream> {
            Err(AgentLoopError::llm("not used in tests"))
        }
    }
}
