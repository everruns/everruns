# Session Schedules — Implementation Plan

## Overview

Add session-scoped scheduling capability that lets agents schedule future work within a session. When a schedule fires, a system message is injected into the session, triggering a turn.

**Key difference from existing durable schedules**: Session schedules are user-facing, session-bound, created via agent tools, and trigger conversational turns — not system-level workflows.

## Architecture

```
Agent (via tools)                   Session Schedules Tab (UI)
    │                                        │
    ▼                                        ▼
┌─────────────────────────────────────────────────┐
│  session_schedules table (PostgreSQL)           │
│  - session_id FK, cron/one-shot, description    │
│  - max 5 active per session                     │
└────────────────────┬────────────────────────────┘
                     │
                     ▼
┌─────────────────────────────────────────────────┐
│  SessionScheduler (control-plane)               │
│  - Polls due session schedules                  │
│  - Injects input.message (role: "schedule")     │
│  - Triggers turn via runner.start_run()         │
└─────────────────────────────────────────────────┘
```

## Data Model

### New table: `session_schedules`

| Field | Type | Description |
|-------|------|-------------|
| id | UUID PK (uuidv7) | Primary key |
| public_id | TEXT UNIQUE | Format: `sched_{32-hex}` |
| session_id | UUID FK → sessions | Parent session |
| description | TEXT NOT NULL | What the agent should do |
| cron_expression | TEXT | Cron expression (NULL for one-shot) |
| scheduled_at | TIMESTAMPTZ | One-shot trigger time (NULL for recurring) |
| timezone | TEXT DEFAULT 'UTC' | IANA timezone |
| enabled | BOOLEAN DEFAULT true | Active flag |
| next_trigger_at | TIMESTAMPTZ | Next computed trigger (indexed) |
| last_triggered_at | TIMESTAMPTZ | Last trigger |
| trigger_count | INTEGER DEFAULT 0 | Total triggers |
| created_at | TIMESTAMPTZ | Creation time |
| updated_at | TIMESTAMPTZ | Last update |

Constraint: `CHECK (cron_expression IS NOT NULL OR scheduled_at IS NOT NULL)` — must have one scheduling method.

Index: `CREATE INDEX idx_session_schedules_polling ON session_schedules (next_trigger_at) WHERE enabled = true;`

Index: `CREATE INDEX idx_session_schedules_session ON session_schedules (session_id) WHERE enabled = true;`

### New TypedId: `ScheduleId` (prefix: `sched_`)

Add to `typed_id.rs` and `id-schema.md`.

### New event type: `schedule.triggered`

Data: `{ schedule_id, description }` — emitted when a schedule fires (informational).

### Message injection

When a schedule fires, inject an `input.message` event with a special metadata marker:

```json
{
  "type": "input.message",
  "data": {
    "message": {
      "role": "user",
      "content": [{"type": "text", "text": "Scheduled task: <description>"}],
      "metadata": { "source": "schedule", "schedule_id": "sched_..." }
    }
  }
}
```

Using `role: "user"` with metadata marker (not a new role) keeps compatibility with all LLM providers. The UI can render these differently based on `metadata.source === "schedule"`.

## Implementation Steps

### Phase 1: Core (Rust)

1. **Migration** (`010_session_schedules.sql`)
   - `session_schedules` table with all fields above
   - Polling and session indexes

2. **TypedId** — add `ScheduleId` with `sched_` prefix

3. **Storage layer** (`server/src/storage/`)
   - `SessionScheduleRow`, `CreateSessionScheduleRow`, `UpdateSessionScheduleRow`
   - CRUD operations + `list_by_session`, `claim_due_schedules`, `count_active_for_session`

4. **Service layer** (`server/src/services/session_schedule.rs`)
   - `SessionScheduleService` — business logic
   - Enforce max 5 active per session
   - Compute `next_trigger_at` from cron/one-shot
   - Trigger: inject message + start turn

5. **Session event type** — add `SCHEDULE_TRIGGERED` constant

6. **Scheduler loop** (`server/src/session_scheduler.rs`)
   - Poll `session_schedules` every 1s for due schedules
   - Claim via `SELECT FOR UPDATE SKIP LOCKED`
   - For each: inject message event, start turn workflow
   - For one-shot: set `enabled = false` after trigger
   - For recurring: compute and set next `next_trigger_at`
   - Start in server alongside existing durable scheduler

### Phase 2: Capability & Tools (Rust)

7. **Capability** (`core/src/capabilities/session_schedule.rs`)
   - ID: `session_schedule`
   - System prompt addition explaining scheduling tools
   - 3 tools: `create_schedule`, `cancel_schedule`, `list_schedules`

8. **Tools implementation**
   - `create_schedule`: params `{ description, cron_expression?, scheduled_at?, timezone? }`
     - Validates max 5 active
     - Computes next_trigger_at
     - Returns schedule details
   - `cancel_schedule`: params `{ schedule_id }`
     - Sets `enabled = false`
   - `list_schedules`: no params
     - Returns all schedules for session (active and recent inactive)

9. **ToolContext extension**
   - Add `session_schedule_store: Option<Arc<dyn SessionScheduleStore>>` to `ToolContext`
   - `SessionScheduleStore` trait in `core/src/traits.rs`

10. **Register capability** in `CapabilityRegistry::with_builtins_for_grade()`

### Phase 3: API (Rust)

11. **REST endpoints** (`server/src/api/session_schedules.rs`)
    - `GET /v1/sessions/{session_id}/schedules` — list schedules
    - `GET /v1/sessions/{session_id}/schedules/{schedule_id}` — get schedule
    - `PATCH /v1/sessions/{session_id}/schedules/{schedule_id}` — update (enable/disable)
    - `DELETE /v1/sessions/{session_id}/schedules/{schedule_id}` — delete schedule
    - `POST /v1/sessions/{session_id}/schedules/{schedule_id}/trigger` — manual trigger

12. **SSE event** — add `schedule.triggered` to supported event types list

13. **Session response extension**
    - Add `active_schedule_count: Option<u32>` to `Session` struct
    - Populate in session queries (subquery or join)

### Phase 4: UI (TypeScript/React)

14. **API client** (`lib/api/session-schedules.ts`)
    - Types: `SessionSchedule`, list/get/update/delete/trigger functions

15. **Hooks** (`hooks/use-session-schedules.ts`)
    - `useSessionSchedules(sessionId)`, `useUpdateSchedule()`, `useDeleteSchedule()`, `useTriggerSchedule()`

16. **Query keys** — add `sessionSchedules` section

17. **Schedules tab** (`sessions/[sessionId]/schedules/page.tsx`)
    - Table: description, type (one-shot/recurring), cron/time, next run, trigger count, status
    - Actions: enable/disable toggle, delete, trigger now
    - Empty state when no schedules

18. **Tab navigation** — add "Schedules" tab with `Clock` icon in session layout

19. **Schedule indicator** — in session layout header and session cards
    - Small badge/icon showing active schedule count
    - Only shown when count > 0

20. **Scheduled message rendering** — in chat view
    - Detect `metadata.source === "schedule"` on input messages
    - Render with clock icon and "Scheduled" label instead of user avatar

21. **Session context** — add `activeScheduleCount` from session data

### Phase 5: Tests

22. **Unit tests** (Rust)
    - Cron next-trigger computation
    - Max 5 enforcement
    - One-shot disabling after trigger
    - Tool parameter validation
    - Storage CRUD operations

23. **Integration tests** (Rust)
    - Schedule creation via API
    - Manual trigger via API
    - Enable/disable via API
    - Session response includes schedule count

24. **UI tests** — component tests for schedule list, indicator

## File Changes Summary

### New files
- `crates/server/migrations/010_session_schedules.sql`
- `crates/core/src/capabilities/session_schedule.rs`
- `crates/server/src/services/session_schedule.rs`
- `crates/server/src/api/session_schedules.rs`
- `crates/server/src/session_scheduler.rs`
- `apps/ui/src/lib/api/session-schedules.ts`
- `apps/ui/src/hooks/use-session-schedules.ts`
- `apps/ui/src/app/(main)/sessions/[sessionId]/schedules/page.tsx`

### Modified files
- `crates/core/src/typed_id.rs` — add ScheduleId
- `crates/core/src/capabilities/mod.rs` — register session_schedule
- `crates/core/src/traits.rs` — add SessionScheduleStore trait + ToolContext field
- `crates/core/src/events.rs` — add SCHEDULE_TRIGGERED constant
- `crates/core/src/session.rs` — add active_schedule_count field
- `crates/server/src/api/mod.rs` — add schedule routes
- `crates/server/src/server.rs` — start scheduler loop
- `crates/server/src/storage/` — add schedule storage
- `apps/ui/src/app/(main)/sessions/[sessionId]/layout.tsx` — add tab + indicator
- `apps/ui/src/components/session/session-card.tsx` — add indicator
- `apps/ui/src/lib/query-keys.ts` — add schedule keys
- `apps/ui/src/lib/api/types.ts` — add schedule types
- `apps/ui/src/app/(main)/sessions/[sessionId]/session-context.tsx` — add schedule count

### Spec update
- `specs/scheduled-tasks.md` — add session schedules section (or new `specs/session-schedules.md`)
- `specs/id-schema.md` — add ScheduleId

## Dependencies

### Rust
- `cron` crate (already in workspace for durable schedules)
- `chrono-tz` (already in workspace)

### npm
- `cronstrue` — human-readable cron descriptions (optional, nice-to-have)

## Decisions

1. **Message role**: Use `role: "user"` with `metadata.source: "schedule"` rather than new role — avoids LLM provider compatibility issues
2. **Storage**: Separate `session_schedules` table (not reusing `durable_schedules`) — different lifecycle, session-scoped, simpler schema
3. **Scheduler**: Separate scheduler loop from durable scheduler — different concerns, lighter weight
4. **Max 5**: Enforced at service layer (not DB constraint) for better error messages
5. **One-shot + recurring**: Both supported — one-shot auto-disables after trigger
