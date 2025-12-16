# M2: Harness/Session/Events Model Refactoring

## Overview

Version: **0.2.0**

This milestone introduces a new data model that better represents agentic loop concepts:

| Current Model | New Model | Description |
|---------------|-----------|-------------|
| Agent | **Harness** | Setup for agentic loop (system prompt, slug, display name, model, tools, budgets) |
| Thread + Run | **Session** | Instance of agentic loop execution |
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

## Data Models

### Harness

Represents configuration for an agentic loop.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `slug` | string | URL-safe unique identifier (e.g., `code-assistant`) |
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

Represents an instance of agentic loop execution.

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

### Migration: 002_harness_session_events.sql

```sql
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

-- Actions table (updated to reference sessions)
CREATE TABLE session_actions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    session_id UUID NOT NULL REFERENCES sessions(id),
    kind VARCHAR(50) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}',
    by_user_id UUID,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_session_actions_session_id ON session_actions(session_id);
```

### Data Migration Strategy

1. **Create new tables** alongside existing tables
2. **Migrate data** with transformation:
   - `agents` → `harnesses` (rename fields, restructure definition → config)
   - `threads` + `runs` → `sessions` (merge with run status)
   - `messages` + `run_events` → `events` (unify with event types)
3. **Verify data integrity** via checksums
4. **Update API** to use new tables
5. **Drop old tables** in subsequent migration (M2.1)

**Agent → Harness Transformation:**
```sql
INSERT INTO harnesses (id, slug, display_name, description, system_prompt, default_model_id, config, status, created_at, updated_at)
SELECT
    id,
    LOWER(REGEXP_REPLACE(name, '[^a-zA-Z0-9]', '-', 'g')), -- Generate slug from name
    name,
    description,
    COALESCE(definition->>'system', ''),
    default_model_id,
    COALESCE(definition->'llm', '{}'),
    status,
    created_at,
    updated_at
FROM agents;
```

**Thread + Run → Session Transformation:**
```sql
INSERT INTO sessions (id, harness_id, status, temporal_workflow_id, temporal_run_id, metadata, created_at, started_at, finished_at)
SELECT
    r.id,
    r.agent_id,  -- agent_id becomes harness_id
    r.status,
    r.temporal_workflow_id,
    r.temporal_run_id,
    jsonb_build_object('thread_id', r.thread_id),  -- Preserve thread reference
    r.created_at,
    r.started_at,
    r.finished_at
FROM runs r;
```

**Messages + RunEvents → Events Transformation:**
```sql
-- Messages → Events (message.user, message.assistant, etc.)
INSERT INTO events (id, session_id, sequence, event_type, data, created_at)
SELECT
    m.id,
    r.id,  -- Session ID (from run)
    ROW_NUMBER() OVER (PARTITION BY r.id ORDER BY m.created_at),
    'message.' || m.role,
    jsonb_build_object('message', jsonb_build_object(
        'role', m.role,
        'content', jsonb_build_array(jsonb_build_object('type', 'text', 'text', m.content))
    )),
    m.created_at
FROM messages m
JOIN runs r ON r.thread_id = m.thread_id;

-- RunEvents → Events (preserve AG-UI event types)
INSERT INTO events (id, session_id, sequence, event_type, data, created_at)
SELECT
    re.id,
    re.run_id,
    re.sequence_number + (SELECT COALESCE(MAX(sequence), 0) FROM events WHERE session_id = re.run_id),
    CASE re.event_type
        WHEN 'RunStarted' THEN 'session.started'
        WHEN 'RunFinished' THEN 'session.finished'
        WHEN 'RunError' THEN 'session.error'
        ELSE re.event_type
    END,
    re.event_data,
    re.created_at
FROM run_events re;
```

## API Changes

### New Endpoints (v1)

**Harness Management:**
- `POST /v1/harnesses` - Create harness
- `GET /v1/harnesses` - List harnesses
- `GET /v1/harnesses/{id}` - Get harness by ID
- `GET /v1/harnesses/slug/{slug}` - Get harness by slug
- `PATCH /v1/harnesses/{id}` - Update harness
- `DELETE /v1/harnesses/{id}` - Archive harness

**Session Management:**
- `POST /v1/harnesses/{harness_id}/sessions` - Create session in harness
- `GET /v1/harnesses/{harness_id}/sessions` - List sessions in harness
- `GET /v1/sessions/{id}` - Get session
- `PATCH /v1/sessions/{id}` - Update session (cancel, etc.)
- `DELETE /v1/sessions/{id}` - Delete session

**Event Management:**
- `POST /v1/sessions/{session_id}/events` - Add event (user message)
- `GET /v1/sessions/{session_id}/events` - Stream events (SSE)
- `GET /v1/sessions/{session_id}/messages` - Get message events only

**AG-UI Protocol:**
- `POST /v1/ag-ui` - CopilotKit endpoint (updated for sessions)

### Request/Response Schemas

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
POST /v1/sessions/{session_id}/events
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

### Deprecation Plan

1. **v0.2.0**: New endpoints available, old endpoints deprecated (HTTP 299 warning)
2. **v0.3.0**: Old endpoints return HTTP 410 Gone
3. **v0.4.0**: Old endpoints removed

Deprecated endpoints:
- `/v1/agents/*` → Use `/v1/harnesses/*`
- `/v1/threads/*` → Use `/v1/sessions/*`
- `/v1/runs/*` → Use `/v1/sessions/*`

## Implementation Phases

### Phase 1: Database Schema (1 PR)

1. Create migration `002_harness_session_events.sql`
2. Add new tables (harnesses, sessions, events, session_actions)
3. Add indexes for performance
4. Update `everruns-storage` with new models and queries
5. Run migration and verify

**Files to change:**
- `crates/everruns-storage/migrations/002_harness_session_events.sql` (new)
- `crates/everruns-storage/src/models.rs` (add new models)
- `crates/everruns-storage/src/lib.rs` (add new query functions)

### Phase 2: Contracts & Events (1 PR)

1. Add new DTOs in `everruns-contracts`
2. Update event types to match new naming
3. Add OpenAPI annotations
4. Ensure AG-UI compatibility

**Files to change:**
- `crates/everruns-contracts/src/lib.rs` (export new types)
- `crates/everruns-contracts/src/harness.rs` (new)
- `crates/everruns-contracts/src/session.rs` (new)
- `crates/everruns-contracts/src/events.rs` (update event types)

### Phase 3: API Layer (1-2 PRs)

1. Add new route handlers
2. Implement harness CRUD
3. Implement session management
4. Implement event streaming
5. Add deprecation warnings to old endpoints

**Files to change:**
- `crates/everruns-api/src/routes/harnesses.rs` (new)
- `crates/everruns-api/src/routes/sessions.rs` (new)
- `crates/everruns-api/src/routes/mod.rs` (update routing)
- `crates/everruns-api/src/main.rs` (register routes)

### Phase 4: Workflow Layer (1 PR)

1. Update workflow to work with sessions
2. Update activities to emit new event types
3. Ensure backward compatibility during transition
4. Update runner abstraction

**Files to change:**
- `crates/everruns-worker/src/workflows.rs` (update for sessions)
- `crates/everruns-worker/src/activities.rs` (update event emission)
- `crates/everruns-worker/src/runner.rs` (update trait)

### Phase 5: Data Migration (1 PR)

1. Create migration script for existing data
2. Add migration verification
3. Test with production-like data
4. Document rollback procedure

**Files to change:**
- `crates/everruns-storage/migrations/003_migrate_data.sql` (new)
- `scripts/verify-migration.sh` (new)

### Phase 6: UI Updates (1-2 PRs)

1. Update hooks for new API
2. Create harness management pages
3. Update chat interface for sessions
4. Add deprecation notices in UI

**Files to change:**
- `apps/ui/src/hooks/use-harnesses.ts` (new)
- `apps/ui/src/hooks/use-sessions.ts` (new)
- `apps/ui/src/app/harnesses/` (new pages)
- `apps/ui/src/app/chat/` (update for sessions)

### Phase 7: Cleanup (1 PR)

1. Remove deprecated endpoints
2. Drop old tables
3. Update documentation
4. Update specs

**Files to change:**
- `specs/models.md` (update)
- `specs/apis.md` (update)
- `crates/everruns-storage/migrations/004_drop_legacy_tables.sql` (new)

## Breaking Changes

1. **API Endpoints**: All `/v1/agents`, `/v1/threads`, `/v1/runs` endpoints will be deprecated
2. **Event Types**: Some event type names change (e.g., `RunStarted` → `session.started`)
3. **Database Schema**: New tables with different structure
4. **Workflow IDs**: Run IDs become Session IDs in Temporal workflows

## Backward Compatibility

1. **Deprecation Period**: Old endpoints work with warnings for 2 minor versions
2. **Data Migration**: All existing data is preserved and migrated
3. **Event Mapping**: AG-UI events remain compatible with standard naming
4. **API Versioning**: Consider `/v2/` prefix for clean break (optional)

## Testing Strategy

1. **Unit Tests**: New models, queries, handlers
2. **Integration Tests**: API endpoints with new schema
3. **Migration Tests**: Data integrity before/after migration
4. **Smoke Tests**: Full workflow with new model
5. **UI Tests**: All pages load and function correctly

## Risks & Mitigations

| Risk | Mitigation |
|------|------------|
| Data loss during migration | Backup + verify checksums + rollback script |
| Breaking existing clients | Deprecation warnings + long transition period |
| Performance regression | Indexes on all query paths + load testing |
| Temporal workflow incompatibility | Version workflows + handle both old/new |

## Success Criteria

- [ ] All existing functionality works with new model
- [ ] No data loss during migration
- [ ] Smoke tests pass
- [ ] UI fully functional with new model
- [ ] Documentation updated
- [ ] Version bumped to 0.2.0

## Timeline Estimate

This refactoring is broken into 7 phases with incremental PRs. Each phase produces a working system.

## Open Questions

1. Should we support concurrent sessions per harness? (Yes, multiple sessions can exist)
2. Should slug be mutable? (Recommend: No, to preserve URL stability)
3. Should we add versioning to harnesses? (Future consideration for M3)
4. How to handle in-flight runs during migration? (Wait for completion or force-cancel)
