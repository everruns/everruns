# Session Schedules Specification

## Abstract

Session schedules allow agents to schedule future work within the current session. When a user asks the agent to "do X at 3am", the agent uses the `schedule_session_task` tool to create a schedule. At the specified time, a system-initiated message is injected into the session and triggers a new turn, executing the scheduled work.

## Goals

1. **Agent-initiated scheduling**: Agent schedules future work via a tool call
2. **Session-scoped**: Schedules belong to a session and execute within that session
3. **New message role**: `app` role distinguishes system-initiated messages from user messages
4. **Leverages existing infrastructure**: Uses the durable scheduler for reliable triggering
5. **Observable**: Schedule lifecycle visible via events and UI

## Non-Goals

1. Recurring/cron schedules within sessions (single-fire only for v1)
2. Cross-session scheduling
3. Schedule editing after creation (cancel only)

## Use Case

1. User opens session, does work with agent
2. User asks: "Deploy to staging at 3am"
3. Agent calls `schedule_session_task` tool with description and target time
4. At 3am, scheduler injects `app` role message: "Scheduled task: Deploy to staging"
5. Message triggers a turn, agent executes the work

## Architecture

```
┌────────────────────────────────────────────────────────────────────┐
│                        Session                                      │
│  User message ──► Agent ──► schedule_session_task tool              │
│                              │                                      │
│                              ▼                                      │
│                     session_schedules table                         │
│                              │                                      │
│                              │  (at scheduled time)                 │
│                              ▼                                      │
│                     DurableScheduler polls                          │
│                              │                                      │
│                              ▼                                      │
│                     Inject "app" message ──► Trigger turn           │
└────────────────────────────────────────────────────────────────────┘
```

## Data Model

### MessageRole: `App`

New role for system/application-initiated messages that trigger agent work.

- Stored as `"app"` in events
- Sent to LLM as `user` role (LLMs only understand system/user/assistant/tool)
- Displayed in UI with distinct styling (e.g., clock icon, "Scheduled" badge)
- Content format: "Scheduled task: {description}"

### SessionSchedule

| Field | Type | Description |
|-------|------|-------------|
| id | UUID v7 | Primary key |
| session_id | SessionId | Session this schedule belongs to |
| organization_id | TEXT | Org for auth/scoping |
| harness_id | HarnessId | For turn triggering |
| agent_id | AgentId? | For turn triggering |
| description | TEXT | What the agent should do |
| scheduled_at | TIMESTAMPTZ | When to trigger |
| status | TEXT | `pending`, `triggered`, `cancelled`, `failed` |
| triggered_at | TIMESTAMPTZ? | When actually triggered |
| message_id | MessageId? | Message created on trigger |
| created_at | TIMESTAMPTZ | Creation time |

## Database Migration

```sql
CREATE TABLE session_schedules (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL,
    organization_id TEXT NOT NULL,
    harness_id TEXT NOT NULL,
    agent_id TEXT,
    description TEXT NOT NULL,
    scheduled_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending'
        CHECK (status IN ('pending', 'triggered', 'cancelled', 'failed')),
    triggered_at TIMESTAMPTZ,
    message_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Index for scheduler polling
CREATE INDEX idx_session_schedules_pending
    ON session_schedules (scheduled_at)
    WHERE status = 'pending';

-- Index for listing by session
CREATE INDEX idx_session_schedules_session
    ON session_schedules (session_id, created_at DESC);
```

## Tool: `schedule_session_task`

Part of `session_schedule` capability.

### Parameters

```json
{
  "type": "object",
  "properties": {
    "description": {
      "type": "string",
      "description": "What to do when the scheduled time arrives"
    },
    "scheduled_at": {
      "type": "string",
      "description": "ISO 8601 datetime for when to execute (e.g., 2024-01-15T03:00:00Z)"
    }
  },
  "required": ["description", "scheduled_at"]
}
```

### Response

```json
{
  "schedule_id": "uuid",
  "description": "Deploy to staging",
  "scheduled_at": "2024-01-15T03:00:00Z",
  "status": "pending"
}
```

## Tool: `cancel_session_schedule`

### Parameters

```json
{
  "type": "object",
  "properties": {
    "schedule_id": {
      "type": "string",
      "description": "ID of the schedule to cancel"
    }
  },
  "required": ["schedule_id"]
}
```

## Tool: `list_session_schedules`

No parameters. Lists all schedules for the current session.

## Trigger Flow

1. **SessionSchedulePoller** runs alongside existing DurableScheduler
2. Polls `session_schedules` table for due items (`scheduled_at <= NOW() AND status = 'pending'`)
3. Uses `SELECT ... FOR UPDATE SKIP LOCKED` for multi-instance safety
4. For each due schedule:
   a. Create `app` role message with content "Scheduled task: {description}"
   b. Store as `input.message` event
   c. Call `AgentRunner::start_run()` to trigger turn
   d. Update schedule status to `triggered`

## Changes Required

### Core (everruns-core)

1. **MessageRole**: Add `App` variant
   - `Display`: "app"
   - `From<&str>`: "app" → App
   - Maps to `LlmMessageRole::User` for LLM calls

2. **Message**: Add `Message::app()` constructor

### API (everruns-server)

3. **API MessageRole**: Add `App` variant
4. **message_store.rs**: Handle `App` role in event conversion (same as User → `input.message`)
5. **New migration**: `session_schedules` table

### Capability (everruns-core)

6. **SessionScheduleCapability**: New capability with three tools
7. **SessionScheduleStore trait**: In `traits.rs` for tool context
8. **ToolContext**: Add `schedule_store` field

### Worker (everruns-worker)

9. **SessionSchedulePoller**: Background task that triggers due schedules

### UI (apps/ui)

10. **MessageRole type**: Add `"app"` to type union
11. **Message rendering**: Distinct styling for app messages

## Implementation Phases

### Phase 1: Core types + Tool
- Add `App` to MessageRole
- Create session_schedules migration
- Implement SessionScheduleStore trait + in-memory impl
- Create SessionScheduleCapability with tools

### Phase 2: Trigger mechanism
- Implement SessionSchedulePoller
- Wire into server startup
- Integration with AgentRunner

### Phase 3: UI
- Update message role types
- Add app message rendering
