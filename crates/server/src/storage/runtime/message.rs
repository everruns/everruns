// Event-based MessageRetriever implementation
//
// Design decision: MessageRetriever is retrieval-only.
// Messages are stored via EventService (input.message, output.message.completed events).
// This trait provides read access for building LLM context.
//
// This module implements the core MessageRetriever trait using the events table
// as the sole source of truth for conversation messages.
//
// Messages are reconstructed from typed events when loaded.

use crate::kernel_imports::{
    ContentPart, Event, EventData, InputMessage, MessageFilter, MessageHistory, MessageQuery,
    MessageRetriever, RuntimeMessage, RuntimeMessageRole,
    contracts::error::Result,
    contracts::error::StoreResultExt,
    contracts::typed_id::MessageId,
    contracts::typed_id::SessionId,
    events::{
        EventContext, EventRequest, InputMessageData, OutputMessageCompletedData, ToolCompletedData,
    },
};
use async_trait::async_trait;
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

use crate::EventService;
use crate::storage::StorageBackend;

// ============================================================================
// DbMessageRetriever - Retrieves messages from events
// ============================================================================

/// Event-based message retriever
///
/// Retrieves conversation messages reconstructed from events in the events table.
/// Used by activities to load messages during workflow execution.
///
/// Note: Message creation happens via EventService, not this struct.
/// The `add` method is provided for API layer convenience.
#[derive(Clone)]
pub struct DbMessageRetriever {
    db: Arc<StorageBackend>,
    event_service: EventService,
}

impl DbMessageRetriever {
    pub fn new(db: Arc<StorageBackend>) -> Self {
        let event_service = EventService::new(
            db.clone(),
            crate::live_updates::event_delivery::EventDelivery::in_memory(),
        );
        Self { db, event_service }
    }

    /// Add a new message and store it as an event
    ///
    /// Note: This is provided for API layer convenience.
    /// Messages are stored via EventService as typed events.
    pub async fn add(&self, session_id: Uuid, input: InputMessage) -> Result<RuntimeMessage> {
        // Create the message
        let message = RuntimeMessage {
            id: Uuid::now_v7().into(),
            role: input.role,
            content: input.content,
            phase: None,
            phase_source: None,
            controls: input.controls,
            metadata: input.metadata,
            external_actor: None,
            created_at: Utc::now(),
        };

        // Emit as typed event based on role
        let session_id: SessionId = session_id.into();
        let event_request = match message.role {
            RuntimeMessageRole::User => EventRequest::new(
                session_id,
                EventContext::empty(),
                InputMessageData::new(message.clone()),
            ),
            RuntimeMessageRole::Agent => EventRequest::new(
                session_id,
                EventContext::empty(),
                OutputMessageCompletedData::new(message.clone()),
            ),
            // System and ToolResult messages are not stored as separate events
            RuntimeMessageRole::System | RuntimeMessageRole::ToolResult => {
                return Ok(message);
            }
        };

        self.event_service.emit(event_request).await.store_err()?;

        Ok(message)
    }
}

#[async_trait]
impl MessageRetriever for DbMessageRetriever {
    async fn get(
        &self,
        session_id: SessionId,
        message_id: MessageId,
    ) -> Result<Option<RuntimeMessage>> {
        retrieve_message(&self.db, session_id, message_id).await
    }

    async fn load(&self, session_id: SessionId) -> Result<Vec<RuntimeMessage>> {
        let events = self.db.list_message_events(session_id).await.store_err()?;

        let mut messages = Vec::with_capacity(events.len());

        for event_row in events {
            match row_to_message(&event_row) {
                Ok(message) => messages.push(message),
                Err(e) => {
                    tracing::warn!("Failed to parse message from event {}: {}", event_row.id, e);
                }
            }
        }

        Ok(messages)
    }

    async fn load_filtered(&self, query: MessageQuery) -> Result<Vec<RuntimeMessage>> {
        Ok(self.load_filtered_history(query).await?.messages)
    }

    async fn load_filtered_history(&self, query: MessageQuery) -> Result<MessageHistory> {
        // Check if we have any custom filters that need in-memory processing
        let has_custom_filters = query.has_custom_filters();
        let needs_exact_filtered_count =
            query.prepend_transform.is_some() && !query.filters.is_empty();
        let needs_unbounded_candidates = has_custom_filters || needs_exact_filtered_count;
        let mut db_query = query.clone();

        if needs_unbounded_candidates {
            db_query.limit = None;
            db_query.offset = None;
        }

        // Load events using the filtered query
        let events = self
            .db
            .list_message_events_filtered(&db_query)
            .await
            .store_err()?;
        let source_sequence = events
            .iter()
            .map(|event| i64::from(event.sequence))
            .max()
            .or(query.after_sequence);

        // Convert events to messages
        let mut messages = Vec::with_capacity(events.len());
        for event_row in events {
            match row_to_message(&event_row) {
                Ok(message) => messages.push(message),
                Err(e) => {
                    tracing::warn!("Failed to parse message from event {}: {}", event_row.id, e);
                }
            }
        }

        // Apply custom filters in-memory if any
        if has_custom_filters {
            messages.retain(|msg| {
                query.filters.iter().all(|filter| match filter {
                    MessageFilter::Custom(predicate) => predicate(msg),
                    _ => true, // DB filters already applied
                })
            });
        }

        let count_before_limit = if query.prepend_transform.is_none() {
            0
        } else if needs_unbounded_candidates {
            messages.len()
        } else {
            self.db
                .count_message_events(query.session_id)
                .await
                .store_err()? as usize
        };

        if needs_unbounded_candidates {
            query.apply_window_bounds(&mut messages);
        }
        query.prepend_excluded_notice(&mut messages, count_before_limit);

        // Apply injections
        if query.has_injections() {
            query.apply_injections(&mut messages);
        }

        Ok(MessageHistory {
            messages,
            source_sequence,
        })
    }

    async fn count(&self, session_id: SessionId) -> Result<usize> {
        let count = self.db.count_message_events(session_id).await.store_err()?;
        Ok(count as usize)
    }
}

// ============================================================================
// Event Parsing
// ============================================================================

/// Shared by the runtime retriever and both worker transports. Turn startup
/// asks for one input; loading history here copies and parses the whole window
/// and cannot find inputs outside that window.
pub(crate) async fn retrieve_message(
    db: &StorageBackend,
    session_id: SessionId,
    message_id: MessageId,
) -> Result<Option<RuntimeMessage>> {
    use futures::TryStreamExt;

    let mut events = db.message_events_by_id(session_id, message_id);
    while let Some(event) = events.try_next().await.store_err()? {
        match event_to_message(&event.data, &event.event_type) {
            Ok(message) if message.id == message_id => return Ok(Some(message)),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(event_id = %event.id, error = %e, "Failed to parse message from event");
            }
        }
    }
    Ok(None)
}

/// Convert a stored message event row to a RuntimeMessage.
///
/// Tool results carry no stored timestamp of their own, so they take the
/// event's. A load-time `now()` would re-render every `<facts>` block that
/// follows a tool result on each request and lose the provider prompt cache
/// from the first tool call on.
fn row_to_message(row: &crate::storage::EventRow) -> std::result::Result<RuntimeMessage, String> {
    let mut message = event_to_message(&row.data, &row.event_type)?;
    if message.role == RuntimeMessageRole::ToolResult {
        message.created_at = row.ts;
    }
    Ok(message)
}

/// Convert stored event data to a RuntimeMessage
///
/// Handles two formats:
/// - Legacy format: full Event struct with id, type, data, etc.
/// - New format: EventData directly (InputMessageData, OutputMessageCompletedData, etc.)
fn event_to_message(
    data: &serde_json::Value,
    event_type: &str,
) -> std::result::Result<RuntimeMessage, String> {
    // Helper to extract message from EventData
    let extract_message = |event_data: EventData| -> std::result::Result<RuntimeMessage, String> {
        match event_data {
            EventData::InputMessage(d) => Ok(d.message),
            EventData::OutputMessageCompleted(d) => Ok(d.message),
            EventData::ToolCompleted(d) => Ok(tool_completed_to_message(d)),
            _ => Err(format!("unexpected event type for message: {}", event_type)),
        }
    };

    // First try to parse as full Event (legacy format)
    // This has required fields like id, type, session_id, data
    if let Ok(event) = serde_json::from_value::<Event>(data.clone()) {
        return extract_message(event.data);
    }

    // Fallback: try to parse as EventData directly (new format)
    // Note: EventData has #[serde(untagged)] with Raw variant, so we use type hint
    match event_type {
        "input.message" => {
            let d: InputMessageData = serde_json::from_value(data.clone())
                .map_err(|e| format!("invalid input.message data: {}", e))?;
            Ok(d.message)
        }
        "output.message.completed" => {
            let d: OutputMessageCompletedData = serde_json::from_value(data.clone())
                .map_err(|e| format!("invalid output.message.completed data: {}", e))?;
            Ok(d.message)
        }
        "tool.completed" => {
            let d: ToolCompletedData = serde_json::from_value(data.clone())
                .map_err(|e| format!("invalid tool.completed data: {}", e))?;
            Ok(tool_completed_to_message(d))
        }
        _ => Err(format!("unexpected event type for message: {}", event_type)),
    }
}

/// Convert ToolCompletedData to a ToolResult message.
///
/// Every path that hands a transcript to the model goes through here (the
/// worker's gRPC and in-process message loads too): image parts become
/// `ContentPart::Image` next to the result so drivers send them as images. A
/// copy that serialized the whole part list into the result text left the
/// model with base64 inside a truncated string and no screenshot.
pub(crate) fn tool_completed_to_message(data: ToolCompletedData) -> RuntimeMessage {
    // Separate text from media. Media (inline images, and screenshots the
    // server moved to the image store as `image_file`) ride as their own parts
    // after the result, in their original order.
    let mut media: Vec<ContentPart> = Vec::new();
    let metadata = tool_result_metadata(&data);
    let result: Option<serde_json::Value> = data.result.map(|parts| {
        for part in &parts {
            match part {
                ContentPart::Image(img) if img.base64.is_some() && img.media_type.is_some() => {
                    media.push(part.clone());
                }
                ContentPart::ImageFile(_) => media.push(part.clone()),
                _ => {}
            }
        }
        // For simple text results, extract just the text
        let text_parts: Vec<&ContentPart> = parts
            .iter()
            .filter(|p| matches!(p, ContentPart::Text(_)))
            .collect();
        if text_parts.len() == 1
            && let ContentPart::Text(t) = text_parts[0]
        {
            return serde_json::Value::String(t.text.clone());
        }
        // Fall back to serializing text parts only (media handled separately)
        if !text_parts.is_empty() {
            serde_json::to_value(&text_parts).unwrap_or_default()
        } else {
            serde_json::Value::Null
        }
    });

    let mut message = if media.is_empty() {
        RuntimeMessage::tool_result(&data.tool_call_id, result, data.error)
    } else {
        let mut message = RuntimeMessage::tool_result(&data.tool_call_id, result, None);
        message.content.extend(media);
        message
    };
    message.metadata = metadata;
    message
}

fn tool_result_metadata(
    data: &ToolCompletedData,
) -> Option<std::collections::HashMap<String, serde_json::Value>> {
    let mut metadata = std::collections::HashMap::new();
    metadata.insert("tool_name".to_string(), serde_json::json!(data.tool_name));
    if let Some(fingerprint) = &data.tool_call_fingerprint {
        metadata.insert(
            "tool_call_fingerprint".to_string(),
            serde_json::json!(fingerprint),
        );
    }
    if let Some(fingerprint) = &data.tool_result_fingerprint {
        metadata.insert(
            "tool_result_fingerprint".to_string(),
            serde_json::json!(fingerprint),
        );
    }
    (!metadata.is_empty()).then_some(metadata)
}

// ============================================================================
// Factory
// ============================================================================

/// Create a database-backed message retriever
pub fn create_db_message_retriever(db: Arc<StorageBackend>) -> DbMessageRetriever {
    DbMessageRetriever::new(db)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod lookup_tests {
    //! Single-message retrieval must not depend on the conversation window.

    use super::*;
    use crate::storage::CreateEventRow;

    fn assert_message(actual: Option<RuntimeMessage>, expected: &RuntimeMessage) {
        assert_eq!(
            serde_json::to_value(actual.unwrap()).unwrap(),
            serde_json::to_value(expected).unwrap()
        );
    }

    async fn store(
        db: &StorageBackend,
        session_id: SessionId,
        event_type: &str,
        data: serde_json::Value,
    ) {
        db.create_event(CreateEventRow {
            session_id,
            event_type: event_type.into(),
            ts: Utc::now(),
            context: serde_json::json!({}),
            data,
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
    }

    async fn history(db: &StorageBackend, session_id: SessionId, count: usize) {
        // Bulk fixture: real event rows and triggers, without thousands of client
        // round trips. Distinct message ids and a 1 KiB payload model long chats.
        let data = serde_json::to_value(InputMessageData::new(RuntimeMessage::user(
            "x".repeat(1024),
        )))
        .unwrap();
        sqlx::query(
            "INSERT INTO events (session_id, sequence, event_type, ts, context, data) \
             SELECT $1, allocate_event_sequence($1), 'input.message', NOW(), '{}', \
                    jsonb_set($3, '{message,id}', to_jsonb('message_' || replace(gen_random_uuid()::text, '-', ''))) \
             FROM generate_series(1, $2)",
        )
        .bind(session_id.uuid())
        .bind(count as i32)
        .bind(data)
        .execute(db.database().pool())
        .await
        .unwrap();
    }

    /// Loading history twice must give the model the same transcript: a tool
    /// result keeps the time it was stored, not the time it was loaded.
    #[tokio::test]
    async fn loaded_tool_results_keep_their_stored_time() {
        let db = Arc::new(StorageBackend::test_database());
        let session = db.create_test_session().await;
        let stored_at = Utc::now() - chrono::Duration::hours(1);
        db.create_event(CreateEventRow {
            session_id: session,
            event_type: "tool.completed".into(),
            ts: stored_at,
            context: serde_json::json!({}),
            data: serde_json::json!({
                "tool_call_id": "call_1", "tool_name": "test",
                "success": true, "status": "success",
                "result": [{"type": "text", "text": "result"}]
            }),
            metadata: None,
            tags: None,
        })
        .await
        .unwrap();
        let retriever = DbMessageRetriever::new(db.clone());
        let loaded = retriever.load(session).await.unwrap();
        let filtered = retriever
            .load_filtered(MessageQuery::new(session))
            .await
            .unwrap();
        for message in [&loaded[0], &filtered[0]] {
            assert_eq!(message.role, RuntimeMessageRole::ToolResult);
            // Postgres keeps microseconds.
            assert_eq!(
                message.created_at.timestamp_micros(),
                stored_at.timestamp_micros()
            );
        }
    }

    #[tokio::test]
    async fn get_finds_messages_older_than_the_history_window() {
        let db = Arc::new(StorageBackend::test_database());
        let session = db.create_test_session().await;
        let retriever = DbMessageRetriever::new(db.clone());
        let first = retriever
            .add(session.uuid(), InputMessage::user("original task"))
            .await
            .unwrap();
        history(
            &db,
            session,
            crate::storage::repository::MESSAGE_SAFETY_LIMIT,
        )
        .await;

        assert!(
            retriever
                .load(session)
                .await
                .unwrap()
                .iter()
                .all(|m| m.id != first.id)
        );
        assert_message(retriever.get(session, first.id).await.unwrap(), &first);
    }

    #[tokio::test]
    async fn get_preserves_message_formats_and_session_isolation() {
        let db = Arc::new(StorageBackend::test_database());
        let session = db.create_test_session().await;
        let other_session = db.create_test_session().await;
        let retriever = DbMessageRetriever::new(db.clone());

        for legacy in [false, true] {
            for agent in [false, true] {
                let message = if agent {
                    RuntimeMessage::assistant("answer")
                } else {
                    RuntimeMessage::user("question")
                };
                let event = if agent {
                    Event::new(
                        session,
                        EventContext::empty(),
                        OutputMessageCompletedData::new(message.clone()),
                    )
                } else {
                    Event::new(
                        session,
                        EventContext::empty(),
                        InputMessageData::new(message.clone()),
                    )
                };
                let data = if legacy {
                    serde_json::to_value(&event).unwrap()
                } else {
                    serde_json::to_value(&event.data).unwrap()
                };
                store(
                    &db,
                    session,
                    if agent {
                        "output.message.completed"
                    } else {
                        "input.message"
                    },
                    data,
                )
                .await;
                assert_message(retriever.get(session, message.id).await.unwrap(), &message);
                assert!(
                    retriever
                        .get(other_session, message.id)
                        .await
                        .unwrap()
                        .is_none()
                );
            }
        }
        assert!(
            retriever
                .get(session, MessageId::new())
                .await
                .unwrap()
                .is_none()
        );

        let malformed_id = MessageId::new();
        store(
            &db,
            session,
            "input.message",
            serde_json::json!({"message": {"id": malformed_id}}),
        )
        .await;
        assert!(
            retriever
                .get(session, malformed_id)
                .await
                .unwrap()
                .is_none()
        );

        // The old loader skipped malformed rows. A duplicate id must not hide a
        // later valid message, and the first valid occurrence still wins.
        let mut valid = RuntimeMessage::user("recovered input");
        valid.id = malformed_id;
        store(
            &db,
            session,
            "input.message",
            serde_json::to_value(InputMessageData::new(valid.clone())).unwrap(),
        )
        .await;
        assert_message(retriever.get(session, malformed_id).await.unwrap(), &valid);
        let mut duplicate = valid.clone();
        duplicate.content = vec![ContentPart::text("later duplicate")];
        store(
            &db,
            session,
            "input.message",
            serde_json::to_value(InputMessageData::new(duplicate)).unwrap(),
        )
        .await;
        assert_message(retriever.get(session, malformed_id).await.unwrap(), &valid);
    }

    #[tokio::test]
    async fn input_lookup_excludes_assistant_messages_and_other_sessions() {
        let db = Arc::new(StorageBackend::test_database());
        let session = db.create_test_session().await;
        let other_session = db.create_test_session().await;
        let retriever = DbMessageRetriever::new(db.clone());
        let input = retriever
            .add(session.uuid(), InputMessage::user("input"))
            .await
            .unwrap();
        let output = RuntimeMessage::assistant("output");
        store(
            &db,
            session,
            "output.message.completed",
            serde_json::to_value(OutputMessageCompletedData::new(output.clone())).unwrap(),
        )
        .await;

        let found = db
            .find_input_message_event(session, input.id)
            .await
            .unwrap()
            .unwrap();
        assert_message(
            Some(event_to_message(&found.data, &found.event_type).unwrap()),
            &input,
        );
        assert!(
            db.find_input_message_event(session, output.id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.find_input_message_event(other_session, input.id)
                .await
                .unwrap()
                .is_none()
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::kernel_imports::{
        ContentPart, Event, ExcludedNoticeTransform, MessageRetriever, ToolCall,
    };
    use everruns_contracts::typed_id::SessionId;
    use everruns_core::events::EventContext;
    use serde_json::json;

    use super::*;

    async fn add_user_messages(
        retriever: &DbMessageRetriever,
        session_id: SessionId,
        texts: &[&str],
    ) {
        for text in texts {
            retriever
                .add(session_id.uuid(), InputMessage::user(*text))
                .await
                .unwrap();
        }
    }

    #[tokio::test]
    async fn load_filtered_applies_custom_filter_before_latest_limit() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = db.create_test_session().await;
        let retriever = DbMessageRetriever::new(db);

        add_user_messages(
            &retriever,
            session_id,
            &["keep 1", "drop 1", "keep 2", "drop 2", "keep 3"],
        )
        .await;

        let query = MessageQuery::new(session_id)
            .with_filter(MessageFilter::Custom(Arc::new(|message| {
                message.text().is_some_and(|text| text.starts_with("keep"))
            })))
            .with_limit(2);

        let messages = retriever.load_filtered(query).await.unwrap();

        let texts: Vec<_> = messages.iter().filter_map(RuntimeMessage::text).collect();
        assert_eq!(texts, vec!["keep 2", "keep 3"]);
    }

    #[tokio::test]
    async fn load_filtered_offset_not_applied_twice_for_db_mappable_query() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = db.create_test_session().await;
        let retriever = DbMessageRetriever::new(db);

        add_user_messages(&retriever, session_id, &["m1", "m2", "m3", "m4", "m5"]).await;

        let query = MessageQuery::new(session_id).with_offset(2);

        let messages = retriever.load_filtered(query).await.unwrap();

        let texts: Vec<_> = messages.iter().filter_map(RuntimeMessage::text).collect();
        assert_eq!(texts, vec!["m3", "m4", "m5"]);
    }

    #[tokio::test]
    async fn load_filtered_offset_and_limit_not_applied_twice_for_db_mappable_query() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = db.create_test_session().await;
        let retriever = DbMessageRetriever::new(db);

        add_user_messages(&retriever, session_id, &["m1", "m2", "m3", "m4", "m5"]).await;

        let query = MessageQuery::new(session_id).with_offset(2).with_limit(2);

        let messages = retriever.load_filtered(query).await.unwrap();

        let texts: Vec<_> = messages.iter().filter_map(RuntimeMessage::text).collect();
        assert_eq!(texts, vec!["m4", "m5"]);
    }
    #[tokio::test]
    async fn load_filtered_notice_counts_filtered_candidates_before_limit() {
        let db = Arc::new(StorageBackend::test_database());
        let session_id = db.create_test_session().await;
        let retriever = DbMessageRetriever::new(db);

        add_user_messages(
            &retriever,
            session_id,
            &["rust 1", "go 1", "rust 2", "go 2", "rust 3"],
        )
        .await;

        let query = MessageQuery::new(session_id)
            .with_filter(MessageFilter::Search("rust".to_string()))
            .with_limit(2)
            .with_prepend_transform(Arc::new(ExcludedNoticeTransform::infinity_context()));

        let messages = retriever.load_filtered(query).await.unwrap();

        assert_eq!(messages.len(), 3);
        assert!(
            messages[0]
                .text()
                .is_some_and(|text| text.contains("1 earlier messages"))
        );

        let visible_texts: Vec<_> = messages[1..]
            .iter()
            .filter_map(RuntimeMessage::text)
            .collect();
        assert_eq!(visible_texts, vec!["rust 2", "rust 3"]);
    }

    // ========================================================================
    // Test: Event to RuntimeMessage parsing
    // ========================================================================

    #[test]
    fn test_parse_output_message_completed_event() {
        let session_id = SessionId::new();
        let message = RuntimeMessage::assistant("Hello from agent!");
        let event = Event::new(
            session_id,
            EventContext::empty(),
            OutputMessageCompletedData::new(message),
        );

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "output.message.completed");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, RuntimeMessageRole::Agent);
        assert_eq!(parsed.text(), Some("Hello from agent!"));
    }

    #[test]
    fn test_parse_tool_completed_event() {
        let session_id = SessionId::new();
        let completed = ToolCompletedData::success(
            "call_123".to_string(),
            "get_weather".to_string(),
            vec![ContentPart::text("Sunny, 72°F")],
            Some(100),
        );
        let event = Event::new(session_id, EventContext::empty(), completed);

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "tool.completed");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, RuntimeMessageRole::ToolResult);
        assert_eq!(parsed.tool_call_id(), Some("call_123"));
    }

    #[test]
    fn test_parse_tool_completed_error() {
        let session_id = SessionId::new();
        let completed = ToolCompletedData::failure(
            "call_456".to_string(),
            "read_file".to_string(),
            "error".to_string(),
            "File not found".to_string(),
            Some(25),
        );
        let event = Event::new(session_id, EventContext::empty(), completed);

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "tool.completed");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, RuntimeMessageRole::ToolResult);

        if let ContentPart::ToolResult(tr) = &parsed.content[0] {
            assert_eq!(tr.error.as_deref(), Some("File not found"));
        } else {
            panic!("Expected ToolResult content part");
        }
    }

    #[test]
    fn test_parse_output_message_with_tool_calls() {
        let session_id = SessionId::new();
        let message = RuntimeMessage::assistant_with_tools(
            "Let me search for that",
            vec![ToolCall {
                id: "call_search".to_string(),
                name: "search".to_string(),
                arguments: json!({"query": "rust"}),
            }],
        );
        let event = Event::new(
            session_id,
            EventContext::empty(),
            OutputMessageCompletedData::new(message),
        );

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "output.message.completed");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert!(parsed.has_tool_calls());
        assert_eq!(parsed.tool_calls().len(), 1);
        assert_eq!(parsed.tool_calls()[0].name, "search");
    }

    #[test]
    fn test_parse_input_message_preserves_external_actor() {
        let session_id = SessionId::new();
        let mut message = RuntimeMessage::user("Hello from Slack!");
        message.external_actor = Some(everruns_core::ExternalActor {
            actor_id: "U0123456789".to_string(),
            actor_name: Some("Alice".to_string()),
            source: "slack".to_string(),
            metadata: None,
        });

        let event = Event::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(message),
        );

        let stored = serde_json::to_value(&event).unwrap();
        let result = event_to_message(&stored, "input.message");

        assert!(result.is_ok());
        let parsed = result.unwrap();
        assert_eq!(parsed.role, RuntimeMessageRole::User);
        assert_eq!(parsed.text(), Some("Hello from Slack!"));

        let actor = parsed
            .external_actor
            .expect("external_actor should be preserved");
        assert_eq!(actor.actor_id, "U0123456789");
        assert_eq!(actor.actor_name.as_deref(), Some("Alice"));
        assert_eq!(actor.source, "slack");
        assert_eq!(actor.display_label(), "Alice");
    }

    #[test]
    fn test_parse_input_message_external_actor_fallback_to_id() {
        let session_id = SessionId::new();
        let mut message = RuntimeMessage::user("Hello!");
        message.external_actor = Some(everruns_core::ExternalActor {
            actor_id: "U0123456789".to_string(),
            actor_name: None,
            source: "slack".to_string(),
            metadata: None,
        });

        let event = Event::new(
            session_id,
            EventContext::empty(),
            InputMessageData::new(message),
        );

        let stored = serde_json::to_value(&event).unwrap();
        let parsed = event_to_message(&stored, "input.message").unwrap();

        let actor = parsed
            .external_actor
            .expect("external_actor should be preserved");
        assert_eq!(actor.display_label(), "U0123456789");
    }
}
