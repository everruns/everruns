# Plan: Message Table Removal Analysis

## Abstract

This document analyzes the feasibility of removing the `messages` table and using the `events` table as the sole source of truth for conversation data. This would implement an event-sourcing pattern where events are the primary data store and messages are materialized views.

## Current Architecture

### Messages Table (Primary Data)

The `messages` table is the **primary conversation data store**:

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID | Foreign key to sessions |
| `sequence` | INTEGER | Order within session (unique per session) |
| `role` | VARCHAR | user, assistant, tool_call, tool_result, system |
| `content` | JSONB | Array of ContentPart (text, image, tool_call, tool_result) |
| `controls` | JSONB | Runtime controls (model_id, reasoning) |
| `metadata` | JSONB | Message-level metadata |
| `tags` | TEXT[] | Message tags |
| `created_at` | TIMESTAMPTZ | Creation time |

**Usage patterns:**
- API: `GET /messages` lists all messages for a session
- API: `POST /messages` creates a user message
- Worker: `MessageStore.load()` loads full conversation for LLM context
- Worker: `MessageStore.store()` saves assistant/tool messages

### Events Table (SSE Notifications)

The `events` table is the **secondary notification stream**:

| Column | Type | Description |
|--------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID | Foreign key to sessions |
| `sequence` | INTEGER | Order within session |
| `event_type` | VARCHAR | message.user, message.assistant, etc. |
| `data` | JSONB | Event payload (includes full content) |
| `created_at` | TIMESTAMPTZ | Event time |

**Current event payload for message events:**
```json
{
  "message_id": "uuid",
  "role": "assistant",
  "content": [...],  // Full ContentPart array
  "sequence": 5,
  "created_at": "2024-12-26T..."
}
```

**Missing from events:** `controls`, `metadata`, `tags`

## Key Finding: Events Already Contain Full Content

When a message is stored, `DbMessageStore.store()` emits an event with the complete message content (`crates/everruns-storage/src/message_store.rs:59-82`). The UI already transforms events back to messages via `eventsToMessages()` in `apps/ui/src/hooks/use-sessions.ts`.

## Feasibility Analysis

### Option A: Remove Messages Table (Event-Sourcing)

Make events the primary store, reconstruct messages from events.

**Required Changes:**

1. **Extend Event Data** - Add missing fields:
   ```json
   {
     "message_id": "uuid",
     "role": "assistant",
     "content": [...],
     "controls": {...},      // NEW
     "metadata": {...},      // NEW
     "tags": [...],          // NEW
     "sequence": 5,
     "created_at": "..."
   }
   ```

2. **New MessageStore Implementation** - Query events instead of messages:
   ```rust
   impl MessageStore for EventBasedMessageStore {
     async fn load(&self, session_id: Uuid) -> Result<Vec<Message>> {
       // SELECT * FROM events
       // WHERE session_id = ?
       //   AND event_type IN ('message.user', 'message.assistant', ...)
       // ORDER BY sequence
     }
   }
   ```

3. **Remove Messages Table** - Drop table after migration

4. **Update API** - `GET /messages` queries events, `POST /messages` only creates events

**Pros:**
- ✅ Single source of truth (event-sourcing pattern)
- ✅ Natural audit log of all changes
- ✅ Simpler schema (one less table)
- ✅ UI already works with events

**Cons:**
- ❌ **Query performance** - Every message load filters by event_type
- ❌ **Index overhead** - Need composite index (session_id, event_type, sequence)
- ❌ **Larger storage** - Events contain more metadata per row
- ❌ **Breaking change** - Documented architecture explicitly says "Messages = primary data"
- ❌ **Mixed concerns** - Events table would contain both messages and workflow events
- ❌ **Pagination complexity** - Can't simply paginate messages, need event filtering

### Option B: Keep Messages, Make Events Derived (Current)

Keep the current architecture where messages are primary and events are notifications.

**Pros:**
- ✅ Clear separation of concerns (data vs notifications)
- ✅ Optimal query performance for message retrieval
- ✅ Simple pagination and filtering on messages
- ✅ Events can be pruned/archived without affecting messages

**Cons:**
- ❌ Dual write (message + event on every store)
- ❌ Potential inconsistency if event emission fails

### Option C: Materialized View Pattern

Keep events as source of truth but add a materialized view or cache for messages.

**Implementation:**
1. Events are the write model (append-only)
2. Messages view/cache is the read model (computed from events)
3. Refresh view on event insert via trigger or async worker

**Pros:**
- ✅ Event-sourcing semantics (auditability, replay)
- ✅ Fast reads from materialized view
- ✅ Can rebuild messages from events if needed

**Cons:**
- ❌ More complexity (triggers, cache invalidation)
- ❌ Eventual consistency between events and view
- ❌ Two storage mechanisms to maintain

## Performance Comparison

### Current (Messages Table)

```sql
-- Load messages for LLM context
SELECT * FROM messages WHERE session_id = ? ORDER BY sequence;
-- Uses idx_messages_session_sequence, O(n) where n = messages in session
```

### Event-Based (Option A)

```sql
-- Load messages from events
SELECT * FROM events
WHERE session_id = ?
  AND event_type IN ('message.user', 'message.assistant', 'message.tool_call', 'message.tool_result')
ORDER BY sequence;
-- Requires filtering by event_type, potentially more rows scanned
```

**Performance Impact:**
- Events table contains ALL event types (workflow events, tool events, etc.)
- Message queries would scan non-message events and filter them out
- Composite index `(session_id, event_type, sequence)` would help but adds overhead

## Recommendation

**Recommended: Option B (Keep Current Architecture)**

The current architecture is well-designed for this use case:

1. **Messages are conversation data** - optimized for retrieval by session
2. **Events are notifications** - optimized for streaming to UI
3. **Dual write is acceptable** - both writes are to the same database in the same transaction (or could be)
4. **Separation of concerns** - messages can evolve independently of event schema

**If event-sourcing is desired in the future:**

Consider Option C (Materialized View) which provides event-sourcing benefits while maintaining read performance. This would require:
1. Making events the source of truth
2. Creating a PostgreSQL materialized view for messages
3. Adding a trigger to refresh on INSERT to events
4. Or using a change data capture (CDC) pattern

## Migration Path (If Proceeding with Option A)

If the decision is made to proceed with event-sourcing:

### Phase 1: Extend Events (Non-Breaking)
1. Add `controls`, `metadata`, `tags` to event data payload
2. Update `DbMessageStore.store()` to include these fields
3. Deploy and verify events contain all data

### Phase 2: Create Event-Based MessageStore
1. Implement `EventBasedMessageStore` that queries events
2. Add feature flag to switch between implementations
3. Test thoroughly in staging environment

### Phase 3: Migrate API
1. Update `GET /messages` to use event-based store
2. Update `POST /messages` to only create events (no message row)
3. Verify UI continues to work (already uses events)

### Phase 4: Remove Messages Table
1. Stop writing to messages table
2. Run migration to drop messages table
3. Update specs and documentation

### Risks
- **Data loss** - If events are incomplete, message data is lost
- **Performance regression** - Event queries may be slower
- **Breaking changes** - External integrations using messages table directly

## Open Questions

1. **Event retention** - Should events be pruned? If so, how do we preserve messages?
2. **Event replay** - Do we need to replay events to rebuild state?
3. **External integrations** - Are there consumers of the messages table outside the API?
4. **Audit requirements** - Is there a compliance need for event-sourcing?

## Conclusion

The messages table serves a clear purpose as the primary conversation data store, while events provide real-time notifications. Removing the messages table is technically feasible but would add complexity and potential performance overhead without clear benefits.

**The current dual-table architecture is appropriate** for the use case. If event-sourcing semantics are required for audit/compliance reasons, consider the materialized view pattern (Option C) rather than eliminating the messages table entirely.
