//! Opt out of Gemini Interactions conversation storage on shared runtime hosts.
use super::Capability;
use async_trait::async_trait;
use serde_json::{Value, json};

pub struct GeminiStatelessCapability;

#[async_trait]
impl Capability for GeminiStatelessCapability {
    fn id(&self) -> &str {
        "gemini_stateless"
    }
    fn name(&self) -> &str {
        "Gemini Stateless"
    }
    fn description(&self) -> &str {
        "Replay the full Gemini conversation without storing an interaction chain."
    }
    fn category(&self) -> Option<&str> {
        Some("Model")
    }
    fn driver_options(&self, _config: &Value) -> Vec<(String, Value)> {
        vec![("gemini/interactions".into(), json!({"store":false}))]
    }
}
