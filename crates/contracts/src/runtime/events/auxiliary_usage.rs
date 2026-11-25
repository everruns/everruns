//! Bounded background billing observations that cannot write conversation content.

use super::*;

impl EventRequest {
    /// A bounded, payload-free usage observation stamped by the completion host.
    pub fn is_session_completion_usage(&self) -> bool {
        let EventData::LlmGeneration(data) = &self.data else {
            return false;
        };
        self.metadata
            .as_ref()
            .and_then(|metadata| metadata.get("completion_operation_id"))
            .and_then(serde_json::Value::as_str)
            .is_some_and(|id| uuid::Uuid::parse_str(id).is_ok())
            && data.messages.is_empty()
            && data.tools.is_empty()
            && data.output.text.is_none()
            && data.output.tool_calls.is_empty()
            && data.request_body.is_none()
            && serde_json::to_vec(self).is_ok_and(|bytes| bytes.len() <= 4096)
    }

    /// The narrow exceptions to foreground ownership, each with a bounded payload.
    pub fn is_background_metadata(&self) -> bool {
        // THREAT[TM-DURABLE-012]: an unowned billing observation cannot
        // impersonate execution. Turn-scoped reviews still require their epoch.
        self.is_dreaming_metadata()
            || (self.is_session_completion_usage()
                && self.context.turn_id.is_none()
                && self.context.exec_id.is_none()
                && self.context.input_message_id.is_none())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> EventRequest {
        EventRequest::new(
            SessionId::new(),
            EventContext::empty(),
            LlmGenerationData::success(
                vec![],
                vec![],
                None,
                vec![],
                "model".into(),
                None,
                Some(TokenUsage::new(900, 100)),
                None,
                None,
            ),
        )
        .with_metadata(serde_json::json!({"completion_operation_id":Uuid::now_v7().to_string()}))
    }

    #[test]
    fn background_usage_cannot_append_content_or_impersonate_execution() {
        let original = request();
        assert!(original.is_background_metadata());
        for field in [
            "messages",
            "tools",
            "text",
            "tool_calls",
            "request_body",
            "size",
        ] {
            let mut candidate = original.clone();
            let EventData::LlmGeneration(data) = &mut candidate.data else {
                unreachable!()
            };
            match field {
                "messages" => data
                    .messages
                    .push(crate::runtime::message::RuntimeMessage::user("private")),
                "tools" => data.tools.push(ToolDefinitionSummary {
                    name: "tool".into(),
                    description: String::new(),
                    display_name: None,
                    category: None,
                    capability_id: None,
                    capability_name: None,
                }),
                "text" => data.output.text = Some("private".into()),
                "tool_calls" => data.output.tool_calls.push(crate::tool_types::ToolCall {
                    id: "id".into(),
                    name: "tool".into(),
                    arguments: serde_json::json!({}),
                }),
                "request_body" => {
                    data.request_body = Some(serde_json::json!({"private":"payload"}))
                }
                "size" => data.metadata.model = "x".repeat(4096),
                _ => unreachable!(),
            }
            assert!(!candidate.is_background_metadata(), "{field}");
        }
        let mut scoped = original.clone();
        scoped.context.turn_id = Some(TurnId::new());
        assert!(!scoped.is_background_metadata());
        scoped.context.turn_id = None;
        scoped.context.exec_id = Some(ExecId::new());
        assert!(!scoped.is_background_metadata());
        scoped.context.exec_id = None;
        scoped.context.input_message_id = Some(MessageId::new());
        assert!(!scoped.is_background_metadata());
        let mut untagged = original;
        untagged.metadata = Some(serde_json::json!({"completion_operation_id":"not-an-operation"}));
        assert!(!untagged.is_background_metadata());
    }
}
