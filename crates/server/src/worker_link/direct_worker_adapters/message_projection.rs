use super::{Event, RuntimeMessage};
use crate::kernel_imports::EventData;
use everruns_contracts::typed_id::MessageId;

/// Convert an event to a message
pub(super) fn event_to_message(event: Event) -> Option<RuntimeMessage> {
    match &event.data {
        EventData::InputMessage(data) => Some(data.message.clone()),
        EventData::OutputMessageCompleted(data) => Some(data.message.clone()),
        EventData::ToolCompleted(data) => {
            let mut message =
                crate::storage::runtime::message::tool_completed_to_message(data.clone());
            message.id = MessageId::from_uuid(event.id.uuid());
            message.created_at = event.ts;
            Some(message)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel_imports::ContentPart;
    use serde_json::json;

    /// A screenshot a tool returned reaches the worker as an image part, not as
    /// base64 inside the result text.
    #[test]
    fn tool_result_images_stay_images() {
        let event: Event = serde_json::from_value(json!({
            "id": "event_01a175cc692270d89a533ec978ac4908",
            "type": "tool.completed",
            "ts": "2026-10-26T01:21:32.956025Z",
            "session_id": "session_01a175cc34c3743fb9e70396ffe6b7a6",
            "context": {},
            "data": {
                "tool_call_id": "call_1",
                "tool_name": "computer",
                "success": true,
                "status": "success",
                "result": [
                    {"type": "text", "text": "{\"status\":\"ok\"}"},
                    {"type": "image", "base64": "iVBORw0KGgo=", "media_type": "image/png"}
                ]
            },
            "sequence": 12
        }))
        .unwrap();

        for message in [
            event_to_message(event.clone()).unwrap(),
            crate::worker_link::grpc_service::event_to_message(&event).unwrap(),
        ] {
            let images: Vec<_> = message
                .content
                .iter()
                .filter_map(|part| match part {
                    ContentPart::Image(image) => image.base64.as_deref(),
                    _ => None,
                })
                .collect();
            assert_eq!(images, vec!["iVBORw0KGgo="]);
            let text = message.content_to_llm_string();
            assert!(!text.contains("iVBORw0KGgo="), "{text}");
        }
    }
}
