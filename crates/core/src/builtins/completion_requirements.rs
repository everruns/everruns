//! Explicit completion evidence shared by framework and server agents.
use super::{Capability, SystemPromptContext};
use crate::completion_requirements::{CAPABILITY_ID, CompletionRequirements};
use async_trait::async_trait;
use serde_json::Value;

pub struct CompletionRequirementsCapability;

#[async_trait]
impl Capability for CompletionRequirementsCapability {
    fn id(&self) -> &str {
        CAPABILITY_ID
    }
    fn name(&self) -> &str {
        "Completion Requirements"
    }
    fn description(&self) -> &str {
        "Check configured tool evidence or final JSON before finishing a turn."
    }
    fn category(&self) -> Option<&str> {
        Some("Quality")
    }
    fn validate_config(&self, config: &Value) -> Result<(), String> {
        CompletionRequirements::parse(config).map(|_| ())
    }
    async fn system_prompt_contribution_with_config(
        &self,
        _ctx: &SystemPromptContext,
        config: &Value,
    ) -> Option<String> {
        Some(format!(
            "Completion requirements for each turn: {config}\nBefore finishing, satisfy each requirement using actual tool results from this turn or a final JSON answer. A tool_result check uses the latest matching call and its successful result. Do not claim completion without this evidence. Ask for required human input when blocked."
        ))
    }
}
