use everruns_core::events::{EventContext, EventRequest, InputMessageData};
use everruns_core::{
    EventEmitter, Message, MessageRetriever, MessageRole, SessionId, ToolCall, ToolResultImage,
};
use everruns_test_support::{InMemoryEventEmitter, InMemoryMessageRetriever};
use serde_json::json;

#[tokio::test]
async fn message_fixture_preserves_tool_calls_in_seeded_history() {
    let session_id = SessionId::new();
    let fixture = InMemoryMessageRetriever::new();
    let message = Message::assistant_with_tools(
        "Checking.",
        vec![ToolCall {
            id: "call_weather".into(),
            name: "get_weather".into(),
            arguments: json!({"city": "Tokyo"}),
        }],
    );

    fixture.store(session_id, message).await.unwrap();

    let loaded = fixture.load(session_id).await.unwrap();
    assert_eq!(loaded.len(), 1);
    assert_eq!(loaded[0].tool_calls()[0].name, "get_weather");
}

#[tokio::test]
async fn message_fixture_preserves_full_parallel_tool_conversation() {
    let session_id = SessionId::new();
    let fixture = InMemoryMessageRetriever::new();
    let calls = ["Tokyo", "London", "New York"]
        .into_iter()
        .enumerate()
        .map(|(index, city)| ToolCall {
            id: format!("call_{index}"),
            name: "get_weather".into(),
            arguments: json!({"city": city}),
        })
        .collect::<Vec<_>>();

    fixture
        .store(
            session_id,
            Message::assistant_with_tools("Checking all cities.", calls.clone()),
        )
        .await
        .unwrap();
    for call in &calls {
        fixture
            .store(
                session_id,
                Message::tool_result(&call.id, Some(json!({"ok": true})), None),
            )
            .await
            .unwrap();
    }
    fixture
        .store(session_id, Message::assistant("All done."))
        .await
        .unwrap();

    let loaded = fixture.load(session_id).await.unwrap();
    assert_eq!(loaded.len(), 5);
    assert_eq!(loaded[0].tool_calls().len(), 3);
    assert_eq!(loaded[1].role, MessageRole::ToolResult);
    assert_eq!(loaded[4].text(), Some("All done."));
}

#[tokio::test]
async fn message_fixture_preserves_tool_result_images() {
    let session_id = SessionId::new();
    let fixture = InMemoryMessageRetriever::new();
    fixture
        .store(
            session_id,
            Message::tool_result_with_images(
                "call_image",
                Some(json!({"ok": true})),
                vec![ToolResultImage {
                    base64: "AAAA".into(),
                    media_type: "image/png".into(),
                }],
            ),
        )
        .await
        .unwrap();

    let loaded = fixture.load(session_id).await.unwrap();
    assert_eq!(loaded[0].role, MessageRole::ToolResult);
    assert_eq!(loaded[0].tool_call_id(), Some("call_image"));
    assert_eq!(loaded[0].content.len(), 2);
}

#[tokio::test]
async fn event_fixture_supports_deterministic_isolated_assertions() {
    let session_id = SessionId::new();
    let fixture = InMemoryEventEmitter::new();

    for text in ["one", "two"] {
        fixture
            .emit(EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(Message::user(text)),
            ))
            .await
            .unwrap();
    }

    let events = fixture.events().await;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].sequence, Some(1));
    assert_eq!(events[1].sequence, Some(2));
}
