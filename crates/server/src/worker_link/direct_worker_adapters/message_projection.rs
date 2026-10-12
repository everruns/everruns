use super::{Event, RuntimeMessage};
use crate::kernel_imports::{ContentPart, EventData, RuntimeMessageRole, ToolResultContentPart};
use everruns_contracts::typed_id::MessageId;

/// Convert an event to a message
pub(super) fn event_to_message(event: Event) -> Option<RuntimeMessage> {
    match &event.data {
        EventData::InputMessage(data) => Some(data.message.clone()),
        EventData::OutputMessageCompleted(data) => Some(data.message.clone()),
        EventData::ToolCompleted(data) => {
            let result_json = data
                .result
                .as_ref()
                .and_then(|r| serde_json::to_value(r).ok());
            let content = vec![ContentPart::ToolResult(ToolResultContentPart {
                tool_call_id: data.tool_call_id.clone(),
                result: result_json,
                error: data.error.clone(),
            })];
            Some(RuntimeMessage {
                id: MessageId::from_uuid(event.id.uuid()),
                role: RuntimeMessageRole::ToolResult,
                content,
                phase: None,
                phase_source: None,
                controls: None,
                metadata: None,
                external_actor: None,
                created_at: event.ts,
            })
        }
        _ => None,
    }
}
