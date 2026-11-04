//! A tool result's inline screenshot is stored as an image row and the event
//! keeps a reference to it.

use super::*;
use base64::Engine;
use everruns_contracts::runtime::message::{ContentPart, ImageContentPart};

#[tokio::test]
async fn tool_screenshot_is_stored_as_an_image_reference() {
    let db = Arc::new(StorageBackend::test_database());
    let event_service = EventService::new(db.clone(), EventDelivery::in_memory());
    let agent_id = AgentId::from_uuid(
        db.create_test_agent(DEFAULT_ORG_ID, uuid::Uuid::now_v7())
            .await,
    );
    let session = db
        .create_session(test_session_input(agent_id))
        .await
        .unwrap()
        .id;

    let png = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4];
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    let mut data: everruns_core::ToolCompletedData = serde_json::from_value(serde_json::json!({
        "tool_call_id": "call_1",
        "tool_name": "browser_screenshot",
        "success": true,
        "status": "success",
    }))
    .unwrap();
    data.result = Some(vec![
        ContentPart::text("captured"),
        ContentPart::Image(ImageContentPart::from_base64(&encoded, "image/png")),
        ContentPart::Image(ImageContentPart::from_base64(&encoded, "application/pdf")),
    ]);
    let emitted = event_service
        .emit(EventRequest::new(session, EventContext::empty(), data))
        .await
        .unwrap();

    let stored = event_service
        .list(session.uuid(), None, None, &[], &[], None, None)
        .await
        .unwrap();
    for event in [&emitted, &stored[0]] {
        let EventData::ToolCompleted(completed) = &event.data else {
            panic!("expected tool.completed, got {:?}", event.data);
        };
        let result = completed.result.as_ref().unwrap();
        assert!(matches!(&result[0], ContentPart::Text(_)));
        let ContentPart::ImageFile(reference) = &result[1] else {
            panic!("screenshot should be a reference, got {:?}", result[1]);
        };
        let image = db
            .get_image(DEFAULT_ORG_ID, reference.image_id.uuid())
            .await
            .unwrap()
            .expect("image row");
        assert_eq!(image.data, png);
        assert_eq!(image.content_type, "image/png");
        // PDFs ride the same inline part and stay inline.
        assert!(
            matches!(&result[2], ContentPart::Image(part) if part.base64.as_deref() == Some(encoded.as_str()))
        );
    }
}
