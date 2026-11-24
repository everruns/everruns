//! Gemini Interactions per-call storage policy. This is conversation storage,
//! not a provider logging or training policy.
use everruns_contracts::driver_registry::LlmCallConfig;
use everruns_contracts::error::{AgentLoopError, Result};

pub const GEMINI_INTERACTIONS_OPTION_KEY: &str = "gemini/interactions";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GeminiInteractionOptions {
    /// Store the conversation for subsequent `previous_interaction_id` calls.
    pub store: bool,
}

impl Default for GeminiInteractionOptions {
    fn default() -> Self {
        Self { store: true }
    }
}

impl GeminiInteractionOptions {
    pub(crate) fn for_call(config: &LlmCallConfig) -> Result<Self> {
        config
            .driver_options
            .get(GEMINI_INTERACTIONS_OPTION_KEY)
            .map(|value| {
                serde_json::from_value(value.clone()).map_err(|e| {
                    AgentLoopError::config(format!("Invalid Gemini interactions options: {e}"))
                })
            })
            .unwrap_or_else(|| Ok(Self::default()))
    }
}
