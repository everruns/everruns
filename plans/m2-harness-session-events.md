# M2: Harness/Session/Events Model Refactoring

## Overview

Version: **0.2.0**

This milestone introduces a new data model that better represents agentic loop concepts:

| Current Model | New Model | Description |
|---------------|-----------|-------------|
| Agent | **Harness** | Setup for agentic loop (system prompt, slug, display name, model, tools, budgets) |
| Thread + Run | **Session** | Instance of agentic loop execution (one harness can have many concurrent sessions) |
| Messages + RunEvents | **Events** | All operations in a session (messages, tool calls, data retrieval) |

## Motivation

The current Agent/Thread/Run/Message model was designed around the OpenAI Assistants API pattern. However, this model has limitations:

1. **Coupling**: Threads are separate from Runs, requiring complex coordination
2. **Message vs Event confusion**: Messages and RunEvents serve overlapping purposes
3. **Naming**: "Agent" doesn't clearly convey it's a configuration/template
4. **Extensibility**: Adding tools, budgets, etc. requires changes across multiple entities

The Harness/Session/Events model provides:

1. **Clear separation**: Harness is the configuration, Session is the execution instance
2. **Unified events**: Everything in a session is an Event (messages, tool calls, etc.)
3. **Intuitive naming**: Harness = test harness for AI, Session = one interaction session
4. **Extensibility**: Harness definition can grow without affecting execution model
5. **Concurrency**: Multiple sessions can run simultaneously under one harness

## Migration Approach

**Big-bang, non-backward-compatible upgrade.** This is a private system, so we can:
- Replace old tables/endpoints in one release
- No deprecation period required
- Kill/cleanup any in-flight runs, or reset database before migration
- Nice-to-have: temporary compatibility during transition phase

**Database Migration Strategy:**
- Single migration (`v1`) that creates new schema
- Keep stable components unchanged: `llm_providers`, `llm_models`
- Drop legacy tables: `agents`, `threads`, `messages`, `runs`, `run_events`, `actions`

## Data Models

### Harness

Represents configuration for an agentic loop. A harness can have many concurrent sessions.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `slug` | string | URL-safe identifier (e.g., `code-assistant`). Mutable. Future: unique per tenant. |
| `display_name` | string | Human-readable name |
| `description` | string? | Optional description |
| `system_prompt` | string | System prompt for the LLM |
| `default_model_id` | UUID v7 | Reference to llm_models table |
| `config` | JSON | Additional configuration (temperature, max_tokens, etc.) |
| `status` | enum | `active`, `archived` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

**Config schema:**
```json
{
  "temperature": 0.7,
  "max_tokens": 4096,
  "tools": [],        // Future: tool configurations
  "budgets": {}       // Future: token/cost budgets
}
```

### Session

Represents an instance of agentic loop execution. Multiple sessions can exist concurrently for a single harness.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `harness_id` | UUID v7 | Parent harness reference |
| `status` | enum | `pending`, `active`, `completed`, `failed`, `cancelled` |
| `model_id` | UUID v7? | Override model (null = use harness default) |
| `metadata` | JSON | Session metadata (title, tags, etc.) |
| `created_at` | timestamp | Creation time |
| `started_at` | timestamp? | First event time |
| `finished_at` | timestamp? | Completion time |

**Metadata schema:**
```json
{
  "title": "Debug login issue",    // Auto-generated or user-provided
  "tags": ["debugging", "auth"]    // Optional tags
}
```

**Status transitions:**
```
pending → active → completed
                 → failed
                 → cancelled
```

### Event

Represents any operation within a session. All operations are events.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session (auto-increment per session) |
| `event_type` | string | Type of event (see below) |
| `data` | JSON | Event-specific payload |
| `created_at` | timestamp | Event time |

**Event Types:**

1. **Message Events** - User and assistant messages
   - `message.user` - User message
   - `message.assistant` - Assistant message (complete)
   - `message.system` - System message

2. **Streaming Events** (AG-UI compatible)
   - `text.start` - Start of text generation
   - `text.delta` - Text chunk
   - `text.end` - End of text generation

3. **Tool Events** (AG-UI compatible)
   - `tool.call.start` - Tool invocation started
   - `tool.call.args` - Tool arguments (streaming)
   - `tool.call.end` - Tool call completed
   - `tool.result` - Tool execution result

4. **Lifecycle Events** (AG-UI compatible)
   - `session.started` - Session execution began
   - `session.finished` - Session completed successfully
   - `session.error` - Session failed with error

5. **State Events**
   - `state.snapshot` - Full state at a point in time
   - `state.delta` - Incremental state change

**Message Event Data Schema:**
```json
{
  "message": {
    "role": "user|assistant|system",
    "content": [
      { "type": "text", "text": "Hello" }
    ]
  }
}
```

**Tool Event Data Schema:**
```json
{
  "tool_call_id": "call_123",
  "name": "search",
  "arguments": { "query": "test" },
  "result": { "matches": [...] }
}
```

## Database Schema

### Migration: v1 (replaces 001_initial_schema.sql)

This migration creates the new schema while preserving stable components (`llm_providers`, `llm_models`).

```sql
-- ============================================
-- Stable tables (unchanged from v0.1.x)
-- ============================================
-- llm_providers - Keep as-is
-- llm_models - Keep as-is

-- ============================================
-- New tables for Harness/Session/Events model
-- ============================================

-- Harnesses table (replaces agents)
CREATE TABLE harnesses (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug VARCHAR(255) NOT NULL UNIQUE,
    display_name VARCHAR(255) NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    default_model_id UUID REFERENCES llm_models(id),
    config JSONB NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_harnesses_slug ON harnesses(slug);
CREATE INDEX idx_harnesses_status ON harnesses(status);

-- Sessions table (replaces threads + runs)
CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    harness_id UUID NOT NULL REFERENCES harnesses(id),
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    model_id UUID REFERENCES llm_models(id),
    metadata JSONB NOT NULL DEFAULT '{}',
    -- Temporal workflow tracking (if using Temporal runner)
    temporal_workflow_id VARCHAR(255),
    temporal_run_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

CREATE INDEX idx_sessions_harness_id ON sessions(harness_id);
CREATE INDEX idx_sessions_status ON sessions(status);
CREATE INDEX idx_sessions_created_at ON sessions(created_at DESC);
CREATE UNIQUE INDEX idx_sessions_temporal_workflow_id
    ON sessions(temporal_workflow_id) WHERE temporal_workflow_id IS NOT NULL;

-- Events table (replaces messages + run_events)
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);

CREATE INDEX idx_events_session_id ON events(session_id);
CREATE INDEX idx_events_session_sequence ON events(session_id, sequence);
CREATE INDEX idx_events_event_type ON events(event_type);

-- Session actions table (replaces actions)
CREATE TABLE session_actions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    kind VARCHAR(50) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_session_actions_session_id ON session_actions(session_id);

-- ============================================
-- Drop legacy tables
-- ============================================
DROP TABLE IF EXISTS actions CASCADE;
DROP TABLE IF EXISTS run_events CASCADE;
DROP TABLE IF EXISTS runs CASCADE;
DROP TABLE IF EXISTS messages CASCADE;
DROP TABLE IF EXISTS threads CASCADE;
DROP TABLE IF EXISTS agents CASCADE;
```

## API Changes

### Response Wrapper Convention

All list endpoints return responses wrapped in a `data` field:

```json
{
  "data": [...]
}
```

Single-item responses return the object directly (no wrapper).

### New Endpoints (v1)

**Harness Management:**
- `POST /v1/harnesses` - Create harness
- `GET /v1/harnesses` - List harnesses → `{ data: [...] }`
- `GET /v1/harnesses/{harness_id}` - Get harness by ID
- `GET /v1/harnesses/slug/{slug}` - Get harness by slug
- `PATCH /v1/harnesses/{harness_id}` - Update harness
- `DELETE /v1/harnesses/{harness_id}` - Archive harness

**Session Management:**
- `POST /v1/harnesses/{harness_id}/sessions` - Create session in harness
- `GET /v1/harnesses/{harness_id}/sessions` - List sessions in harness → `{ data: [...] }`
- `GET /v1/harnesses/{harness_id}/sessions/{session_id}` - Get session
- `PATCH /v1/harnesses/{harness_id}/sessions/{session_id}` - Update session (cancel, etc.)
- `DELETE /v1/harnesses/{harness_id}/sessions/{session_id}` - Delete session

**Event Management:**
- `POST /v1/harnesses/{harness_id}/sessions/{session_id}/events` - Add event (user message)
- `GET /v1/harnesses/{harness_id}/sessions/{session_id}/events` - Stream events (SSE) → `{ data: [...] }` or SSE stream
- `GET /v1/harnesses/{harness_id}/sessions/{session_id}/messages` - Get message events only → `{ data: [...] }`

**AG-UI Protocol:**
- `POST /v1/ag-ui` - CopilotKit endpoint (updated for sessions)

**LLM Providers (unchanged):**
- `/v1/llm-providers/*` - Keep existing endpoints
- `/v1/llm-models/*` - Keep existing endpoints

### Request/Response Schemas

**List Harnesses:**
```json
GET /v1/harnesses
Response:
{
  "data": [
    {
      "id": "uuid",
      "slug": "code-assistant",
      "display_name": "Code Assistant",
      "description": "Helps with coding tasks",
      "status": "active",
      "created_at": "2024-01-01T00:00:00Z",
      "updated_at": "2024-01-01T00:00:00Z"
    }
  ]
}
```

**Create Harness:**
```json
POST /v1/harnesses
{
  "slug": "code-assistant",
  "display_name": "Code Assistant",
  "description": "Helps with coding tasks",
  "system_prompt": "You are a helpful coding assistant...",
  "default_model_id": "uuid",
  "config": {
    "temperature": 0.7
  }
}
```

**Create Session:**
```json
POST /v1/harnesses/{harness_id}/sessions
{
  "metadata": {
    "title": "Debug login issue"
  }
}
```

**Add User Message:**
```json
POST /v1/harnesses/{harness_id}/sessions/{session_id}/events
{
  "event_type": "message.user",
  "data": {
    "message": {
      "role": "user",
      "content": [
        { "type": "text", "text": "Hello, help me debug this" }
      ]
    }
  }
}
```

## Architecture

### Services Layer

Introduce a services layer between API handlers and storage to encapsulate business logic:

```
┌─────────────────────────────────────────────────────────┐
│                      API Layer                          │
│  (routes/harnesses.rs, routes/sessions.rs, etc.)        │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                   Services Layer                        │
│  (HarnessService, SessionService, EventService)         │
│  - Business logic                                       │
│  - Validation                                           │
│  - Cross-entity operations                              │
└─────────────────────────────────────────────────────────┘
                            │
                            ▼
┌─────────────────────────────────────────────────────────┐
│                    Storage Layer                        │
│  (everruns-storage)                                     │
│  - Database queries                                     │
│  - Data models                                          │
└─────────────────────────────────────────────────────────┘
```

**Option A: Services in everruns-api crate** (simpler, start here)
- Add `src/services/` module in everruns-api
- Services own business logic, call storage directly

**Option B: Separate everruns-services crate** (if complexity grows)
- New crate between api and storage
- Better separation but more boilerplate

**Recommendation:** Start with Option A, extract to Option B if needed.

## Implementation Phases

### Phase 1: Database Schema & Storage Layer

1. Create new migration file (rename to v1 approach or use 002)
2. Add new models in `everruns-storage`:
   - `HarnessRow`, `CreateHarness`, `UpdateHarness`
   - `SessionRow`, `CreateSession`, `UpdateSession`
   - `EventRow`, `CreateEvent`
   - `SessionActionRow`, `CreateSessionAction`
3. Add query functions for all CRUD operations
4. Add integration tests for storage layer

**Files to change:**
- `crates/everruns-storage/migrations/002_harness_session_events.sql` (new)
- `crates/everruns-storage/src/models.rs` (add new models)
- `crates/everruns-storage/src/lib.rs` (add new query functions)
- `crates/everruns-storage/src/tests/` (integration tests)

### Phase 2: Contracts Layer

1. Add new DTOs in `everruns-contracts`:
   - `Harness`, `CreateHarnessRequest`, `UpdateHarnessRequest`
   - `Session`, `CreateSessionRequest`, `UpdateSessionRequest`
   - `Event`, `CreateEventRequest`
   - `ListResponse<T>` wrapper for `{ data: [...] }`
2. Update event types to match new naming
3. Add OpenAPI annotations
4. Ensure AG-UI compatibility

**Files to change:**
- `crates/everruns-contracts/src/lib.rs` (export new types)
- `crates/everruns-contracts/src/harness.rs` (new)
- `crates/everruns-contracts/src/session.rs` (new)
- `crates/everruns-contracts/src/events.rs` (update event types)
- `crates/everruns-contracts/src/common.rs` (ListResponse wrapper)

### Phase 3: Services & API Layer

1. Add services layer in everruns-api:
   - `HarnessService` - harness CRUD + validation
   - `SessionService` - session lifecycle management
   - `EventService` - event creation + streaming
2. Add new route handlers using services
3. Remove old endpoints (agents, threads, runs, messages)
4. Update OpenAPI docs

**Files to change:**
- `crates/everruns-api/src/services/mod.rs` (new)
- `crates/everruns-api/src/services/harness.rs` (new)
- `crates/everruns-api/src/services/session.rs` (new)
- `crates/everruns-api/src/services/event.rs` (new)
- `crates/everruns-api/src/routes/harnesses.rs` (new)
- `crates/everruns-api/src/routes/sessions.rs` (new)
- `crates/everruns-api/src/routes/events.rs` (new)
- `crates/everruns-api/src/routes/mod.rs` (update routing)
- `crates/everruns-api/src/main.rs` (register routes)
- Remove: `routes/agents.rs`, `routes/threads.rs`, `routes/runs.rs`

### Phase 4: Workflow Layer

1. Update workflow to work with sessions instead of runs
2. Update activities to emit new event types
3. Update runner abstraction trait
4. Update both in-process and Temporal runners

**Files to change:**
- `crates/everruns-worker/src/workflows.rs` (update for sessions)
- `crates/everruns-worker/src/activities.rs` (update event emission)
- `crates/everruns-worker/src/runner.rs` (update trait)
- `crates/everruns-worker/src/temporal/` (update Temporal integration)

### Phase 5: UI Updates

1. Update API client hooks for new endpoints
2. Create harness management pages (list, create, edit)
3. Update chat interface for sessions with nested URLs
4. Update navigation and routing

**Files to change:**
- `apps/ui/src/lib/api.ts` (update API client)
- `apps/ui/src/hooks/use-harnesses.ts` (new)
- `apps/ui/src/hooks/use-sessions.ts` (new)
- `apps/ui/src/hooks/use-events.ts` (new)
- `apps/ui/src/app/harnesses/` (new pages)
- `apps/ui/src/app/harnesses/[harnessId]/sessions/` (new pages)
- Remove: `apps/ui/src/app/agents/`, `apps/ui/src/hooks/use-agents.ts`, etc.

### Phase 6: Testing & Cleanup

1. Update all integration tests
2. Update smoke tests for new model
3. Update specs documentation
4. Bump version to 0.2.0

**Files to change:**
- `crates/everruns-api/tests/` (integration tests)
- `scripts/smoke-test.sh` (update for new endpoints)
- `specs/models.md` (update)
- `specs/apis.md` (update)
- `Cargo.toml` (version bump)

## Testing Strategy

1. **Unit Tests**: New models, services, query functions
2. **Integration Tests**:
   - API endpoints with new schema
   - Full CRUD operations for harnesses, sessions, events
   - Event streaming via SSE
3. **Smoke Tests**: Full workflow with new model
   - Create harness
   - Create session
   - Send message → session runs → events stream
   - Verify completion
4. **UI Tests**: All pages load and function correctly

## Decisions Summary

| Question | Decision |
|----------|----------|
| Concurrent sessions per harness? | Yes, multiple sessions can exist |
| Slug mutable? | Yes (future: unique per tenant) |
| Harness versioning? | No, future consideration |
| In-flight runs during migration? | Kill/cleanup or reset database |
| Backward compatibility? | Nice-to-have, not required |
| API version? | Stay on v1 |
| Services layer? | Yes, in everruns-api (Option A) |

## Success Criteria

- [ ] Database migration runs successfully
- [ ] All new API endpoints working
- [ ] Event streaming works via SSE
- [ ] Smoke tests pass
- [ ] Integration tests pass
- [ ] UI fully functional with new model
- [ ] Documentation/specs updated
- [ ] Version bumped to 0.2.0
