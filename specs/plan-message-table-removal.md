# Plan: Message Table Removal

## Abstract

This document analyzes removing the `messages` table and using the `events` table as the sole source of truth for conversation data.

## Current State Analysis

### Key Finding: Events Already Mirror Messages

The `events` table currently **only contains message events**. Every message stored creates:
1. A row in `messages` table (via `db.create_message()`)
2. A row in `events` table (via `db.create_event()`)

This dual write is redundant. The events table already has the full message content.

### Current Event Data Payload

```json
{
  "message_id": "uuid",
  "role": "assistant",
  "content": [...],      // Full Vec<ContentPart>
  "sequence": 5,
  "created_at": "2024-12-26T..."
}
```

**Missing from events:** `controls`, `metadata`, `tags` (but `tags` is always empty in worker context)

### Usage Patterns

| Consumer | Operation | Current Implementation |
|----------|-----------|----------------------|
| Worker | Load messages for LLM | `db.list_messages(session_id)` |
| Worker | Store message | `db.create_message()` + `db.create_event()` |
| API | Create user message | `db.create_message()` + `db.create_event()` |
| API | List messages | `db.list_messages(session_id)` (rarely used) |
| UI | Display messages | Fetches events, transforms to messages |

**The UI already uses events as source of truth** via `eventsToMessages()`.

## Recommendation: Remove Messages Table

Given:
- ✅ Events already contain full message content
- ✅ UI already works from events
- ✅ Message APIs have almost no use cases
- ✅ Dual write is redundant overhead
- ✅ Events table only contains message events currently

**The messages table can be removed.**

## Implementation Plan

### Phase 1: Extend Event Payload (Non-Breaking)

Add missing fields to event data:

```rust
// In message_store.rs, update event creation:
data: serde_json::json!({
    "message_id": stored_msg.id,
    "role": role.to_string(),
    "content": content,
    "controls": message.controls,      // NEW
    "metadata": message.metadata,      // NEW
    "sequence": stored_msg.sequence,
    "created_at": stored_msg.created_at,
}),
```

Also update `services/message.rs` for user messages.

**Files to modify:**
- `crates/everruns-storage/src/message_store.rs` (lines 72-78)
- `crates/everruns-api/src/services/message.rs` (lines 74-79)

### Phase 2: Add Partial Index

Create index for efficient message event queries:

```sql
-- Migration: add_message_events_index.sql
CREATE INDEX idx_events_messages ON events(session_id, sequence)
WHERE event_type IN ('message.user', 'message.assistant', 'message.tool_call', 'message.tool_result');
```

This makes `SELECT FROM events WHERE event_type IN (...) AND session_id = ?` as fast as the current messages query.

### Phase 3: Implement Event-Based MessageStore

Create new implementation that reads from events:

```rust
// New: EventMessageStore
impl MessageStore for EventMessageStore {
    async fn store(&self, session_id: Uuid, message: Message) -> Result<()> {
        // Only create event, no message row
        let event_type = match message.role {
            MessageRole::User => "message.user",
            MessageRole::Assistant => "message.assistant",
            MessageRole::ToolCall => "message.tool_call",
            MessageRole::ToolResult => "message.tool_result",
            MessageRole::System => "message.system",
        };

        let event = CreateEventRow {
            session_id,
            event_type: event_type.to_string(),
            data: serde_json::json!({
                "role": message.role.to_string(),
                "content": message.content,
                "controls": message.controls,
                "metadata": message.metadata,
            }),
        };
        self.db.create_event(event).await?;
        Ok(())
    }

    async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let events = self.db.list_message_events(session_id).await?;
        // Transform events to messages
        events.into_iter().map(|e| {
            let data = e.data;
            Message {
                id: data["message_id"].as_str().map(Uuid::parse_str).unwrap()?,
                role: MessageRole::from(data["role"].as_str().unwrap()),
                content: serde_json::from_value(data["content"].clone())?,
                controls: data.get("controls").cloned().map(serde_json::from_value).transpose()?,
                metadata: data.get("metadata").cloned().map(serde_json::from_value).transpose()?,
                created_at: e.created_at,
            }
        }).collect()
    }
}
```

**New repository method:**

```rust
// In repositories.rs
pub async fn list_message_events(&self, session_id: Uuid) -> Result<Vec<EventRow>> {
    sqlx::query_as::<_, EventRow>(
        r#"
        SELECT id, session_id, sequence, event_type, data, created_at
        FROM events
        WHERE session_id = $1
          AND event_type IN ('message.user', 'message.assistant', 'message.tool_call', 'message.tool_result', 'message.system')
        ORDER BY sequence ASC
        "#,
    )
    .bind(session_id)
    .fetch_all(&self.pool)
    .await
}
```

### Phase 4: Update API Layer

Modify message service to use event-based store:

```rust
// services/message.rs
impl MessageService {
    pub async fn create(&self, ...) -> Result<Message> {
        // Create event only (no message row)
        let event = self.db.create_event(CreateEventRow {
            session_id,
            event_type: "message.user".to_string(),
            data: serde_json::json!({
                "role": "user",
                "content": content,
                "controls": request.controls,
                "metadata": request.metadata,
            }),
        }).await?;

        // Transform event to message for response
        Ok(event_to_message(event))
    }

    pub async fn list(&self, session_id: Uuid) -> Result<Vec<Message>> {
        let events = self.db.list_message_events(session_id).await?;
        Ok(events.into_iter().map(event_to_message).collect())
    }
}
```

### Phase 5: Migration - Drop Messages Table

```sql
-- Migration: drop_messages_table.sql
DROP TABLE messages;
```

### Phase 6: Update Specs and Docs

- Update `specs/models.md` - Remove Message table, update Event documentation
- Update `specs/architecture.md` - Reflect event-sourced messages

## File Changes Summary

| File | Change |
|------|--------|
| `migrations/XXX_extend_event_payload.sql` | (optional) Add constraints/comments |
| `migrations/XXX_add_message_events_index.sql` | Add partial index |
| `migrations/XXX_drop_messages_table.sql` | Drop messages table |
| `crates/everruns-storage/src/models.rs` | Remove `MessageRow`, `CreateMessageRow` |
| `crates/everruns-storage/src/repositories.rs` | Remove message CRUD, add `list_message_events` |
| `crates/everruns-storage/src/message_store.rs` | Rewrite to use events |
| `crates/everruns-api/src/services/message.rs` | Use events instead of messages |
| `crates/everruns-api/src/messages.rs` | Update types if needed |
| `specs/models.md` | Update documentation |

## Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| Data loss during migration | Backfill events from messages before dropping table |
| Event payload schema changes | Make changes in Phase 1, deploy, verify before proceeding |
| Performance regression | Partial index ensures equivalent query performance |

## Rollback Plan

If issues arise after Phase 5:
1. Re-run messages table creation migration
2. Backfill messages from events (since events have full data)
3. Revert code changes

## Open Questions

1. **System messages** - Currently no event emitted for `MessageRole::System`. Should we add `message.system` event type?
2. **Message ID generation** - Events use their own UUID. Should message_id in data be the event id, or generate separately?
3. **Tags** - Currently unused in worker context. Keep in event payload or remove?

## Conclusion

Removing the messages table is feasible and recommended given:
- Events already contain full message content
- UI already works from events
- Dual write is unnecessary overhead
- Clear migration path with rollback option

The implementation can be done incrementally with each phase deployable independently.
