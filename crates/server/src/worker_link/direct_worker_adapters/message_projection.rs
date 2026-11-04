use super::{Event, RuntimeMessage};
use crate::kernel_imports::EventData;
use everruns_contracts::typed_id::MessageId;

pub(super) async fn get_message(
    db: &crate::storage::StorageBackend,
    session_id: uuid::Uuid,
    message_id: uuid::Uuid,
) -> super::Result<Option<RuntimeMessage>> {
    use everruns_contracts::error::StoreResultExt;

    let result: super::Result<_> = async {
        let message = crate::storage::runtime::message::retrieve_message(
            db,
            session_id.into(),
            message_id.into(),
        )
        .await?;
        if message.is_some() {
            return Ok(message);
        }
        // Keep the stable event-derived IDs exposed by load_messages for tool
        // results, without falling back to scanning the conversation.
        let event = db
            .tool_message_event_by_id(session_id.into(), message_id.into())
            .await
            .store_err()?;
        Ok(event.and_then(|row| event_to_message(crate::EventService::row_to_event(row))))
    }
    .await;
    result.map_err(|error| {
        tracing::error!(error = %error, "Failed to load message");
        super::store_error("Failed to load message")
    })
}

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

    #[tokio::test]
    async fn indexed_get_preserves_event_derived_tool_result_ids() {
        use everruns_worker::WorkerAdapters;

        let adapters = super::super::tests::test_adapters();
        let session = adapters.db.create_test_session().await;
        let other = adapters.db.create_test_session().await;
        let event = adapters
            .db
            .create_event(crate::storage::CreateEventRow {
                session_id: session,
                event_type: "tool.completed".into(),
                ts: chrono::Utc::now(),
                context: json!({}),
                data: json!({
                    "tool_call_id": "call_1", "tool_name": "test",
                    "success": true, "status": "success",
                    "result": [{"type": "text", "text": "result"}]
                }),
                metadata: None,
                tags: None,
            })
            .await
            .unwrap();
        let loaded = adapters.load_messages(session.uuid()).await.unwrap();
        let message = adapters
            .get_message(session.uuid(), event.id.uuid())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(message).unwrap(),
            serde_json::to_value(&loaded[0]).unwrap()
        );
        assert!(
            adapters
                .get_message(other.uuid(), event.id.uuid())
                .await
                .unwrap()
                .is_none()
        );
    }

    /// A tool result keeps the time and id it was stored with, on both worker
    /// paths. A load-time `now()` re-rendered the `<facts>` block after every
    /// tool result on each request and cost the prompt cache.
    #[test]
    fn tool_results_keep_stored_time_and_id() {
        let event: Event = serde_json::from_value(json!({
            "id": "event_01a175cc692270d89a533ec978ac4908",
            "type": "tool.completed",
            "ts": "2026-10-26T01:21:32.956025Z",
            "session_id": "session_01a175cc34c3743fb9e70396ffe6b7a6",
            "context": {},
            "data": {
                "tool_call_id": "call_1", "tool_name": "test",
                "success": true, "status": "success",
                "result": [{"type": "text", "text": "result"}]
            },
            "sequence": 12
        }))
        .unwrap();

        for message in [
            event_to_message(event.clone()).unwrap(),
            crate::worker_link::grpc_service::event_to_message(&event).unwrap(),
        ] {
            assert_eq!(message.created_at, event.ts);
            assert_eq!(message.id.uuid(), event.id.uuid());
        }
    }

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
