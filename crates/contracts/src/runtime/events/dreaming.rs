//! Metadata-only maintenance events, separate from foreground actor epochs.
use super::*;
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DreamingActivityData {
    pub operation_id: String,
    pub status: String,
    pub note_count: usize,
    pub reason: Option<String>,
    pub cost_usd: Option<f64>,
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
}

impl EventRequest {
    /// Only bounded, payload-free maintenance observations may append without
    /// a foreground epoch. Normal generation prompts and responses still need it.
    pub fn is_dreaming_metadata(&self) -> bool {
        if self.context.turn_id.is_some() {
            return false;
        }
        let allowed = match &self.data {
            EventData::DreamingActivity(_) => true,
            EventData::LlmGeneration(data) => {
                self.metadata
                    .as_ref()
                    .and_then(|m| m.get("dreaming_operation_id"))
                    .and_then(|v| v.as_str())
                    .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
                    && data.messages.is_empty()
                    && data.tools.is_empty()
                    && data.output.text.is_none()
                    && data.output.tool_calls.is_empty()
                    && data.request_body.is_none()
            }
            _ => false,
        };
        allowed && serde_json::to_vec(self).is_ok_and(|v| v.len() <= 4096)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn background_metering_cannot_carry_prompts_or_generated_memory() {
        let data = LlmGenerationData::success(
            vec![],
            vec![],
            None,
            vec![],
            "model".into(),
            None,
            None,
            None,
            None,
        );
        let request = EventRequest::new(SessionId::new(), Default::default(), data).with_metadata(
            serde_json::json!({"dreaming_operation_id":uuid::Uuid::now_v7().to_string()}),
        );
        assert!(request.is_dreaming_metadata());
        let mut output = request.clone();
        if let EventData::LlmGeneration(data) = &mut output.data {
            data.output.text = Some("private notes".into());
        }
        assert!(!output.is_dreaming_metadata());
        let mut captured = request.clone();
        if let EventData::LlmGeneration(data) = &mut captured.data {
            data.request_body = Some(serde_json::json!({"messages":"private evidence"}));
        }
        assert!(!captured.is_dreaming_metadata());
        let mut untagged = request;
        untagged.metadata = None;
        assert!(!untagged.is_dreaming_metadata());
    }
}
