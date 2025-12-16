# M2: Agent/Session/Messages Model (Revised)

## Overview

Version: **0.2.0**

This milestone clarifies the data model to properly separate **Messages** (primary data) from **Events** (UI notification channel).

## Correct Mental Model

```
> Create Agent
> Create Session
> Create Message(role=user, content="How much 2+2")

< Event: step.generating → UI
< Event: step.generating → UI
< Event: step.generated → UI
< Message(role=assistant, content="4")
```

**Key insight**: Events are a **second channel** - a notification stream for the UI. When a message or step happens, a corresponding event is emitted. But Messages are the primary data store.

## Data Model

| Entity | Purpose |
|--------|---------|
| **Agent** | Configuration for agentic loop (system prompt, model, tools) |
| **Session** | Instance of execution (one agent can have many sessions) |
| **Message** | Primary conversation data (user, assistant, tool_call, tool_result) |
| **Event** | SSE notification stream for UI (step.generating, step.generated, etc.) |

### Key Differences from Previous M2

| Previous (Wrong) | Correct |
|------------------|---------|
| Events ARE the messages | Messages are primary, Events are notifications |
| `message.user` event = user message | Message record + optionally `message.created` event |
| Tool calls stored as events | Tool calls stored as Messages with `role=tool_call` |
| No separate Messages table | Separate Messages table |

## Entity Definitions

### Agent

Configuration for an agentic loop. An agent can have many concurrent sessions.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `slug` | string | URL-safe identifier (e.g., `code-assistant`) |
| `name` | string | Display name |
| `description` | string? | Optional description |
| `system_prompt` | string | System prompt for the LLM |
| `default_model_id` | UUID? | Reference to llm_models table |
| `temperature` | float? | LLM temperature |
| `max_tokens` | integer? | Max tokens for response |
| `tools` | JSON | Tool definitions |
| `tags` | string[] | Tags for organization |
| `status` | enum | `active`, `archived` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

### Session

An instance of agentic loop execution.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `agent_id` | UUID v7 | Parent agent reference |
| `title` | string? | Session title |
| `tags` | string[] | Tags |
| `model_id` | UUID? | Override model (null = use agent default) |
| `status` | enum | `pending`, `running`, `completed`, `failed` |
| `created_at` | timestamp | Creation time |
| `started_at` | timestamp? | Execution start time |
| `finished_at` | timestamp? | Completion time |

### Message

The actual conversation content. This is the **primary data store**.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session |
| `role` | enum | `user`, `assistant`, `tool_call`, `tool_result`, `system` |
| `content` | JSON | Message content (see below) |
| `tool_call_id` | string? | For tool_result, references the tool_call |
| `created_at` | timestamp | Creation time |

**Content schemas by role:**

```json
// role=user or role=assistant or role=system
{
  "text": "Hello, how are you?"
}

// role=tool_call (assistant requesting tool execution)
{
  "id": "call_abc123",
  "name": "search",
  "arguments": { "query": "test" }
}

// role=tool_result (result of tool execution)
{
  "result": { "matches": [...] },
  "error": null
}
```

### Event (SSE Stream)

Notifications for UI about what's happening. **Not the primary data store.**

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session |
| `event_type` | string | Type of notification |
| `data` | JSON | Event-specific payload |
| `created_at` | timestamp | Event time |

**Event Types:**

1. **Step Events** - Workflow progress notifications
   - `step.started` - Started processing (e.g., LLM call began)
   - `step.generating` - Generation in progress (can be repeated for streaming)
   - `step.generated` - Generation complete
   - `step.error` - Step failed

2. **Message Events** - Notifications about messages (optional, for real-time updates)
   - `message.created` - A new message was created
   - `message.delta` - Streaming content update

3. **Tool Events** - Tool execution notifications
   - `tool.started` - Tool execution began
   - `tool.completed` - Tool execution finished

4. **Session Events** - Session lifecycle
   - `session.started` - Session began processing
   - `session.completed` - Session finished successfully
   - `session.failed` - Session encountered error

## Flow Example

```
User: "How much is 2+2?"

1. POST /v1/agents/{id}/sessions/{id}/messages
   Body: { "role": "user", "content": { "text": "How much is 2+2?" } }
   → Creates Message(role=user)
   → Triggers session workflow

2. Workflow starts
   → Emits Event(step.started)

3. LLM streaming
   → Emits Event(step.generating, data: { "delta": "The answer" })
   → Emits Event(step.generating, data: { "delta": " is 4" })

4. LLM complete
   → Creates Message(role=assistant, content: { "text": "The answer is 4" })
   → Emits Event(step.generated)
   → Emits Event(message.created, data: { "message_id": "..." })

5. Session complete
   → Emits Event(session.completed)
   → Updates Session(status=completed)
```

## Database Schema

```sql
-- Agents table
CREATE TABLE agents (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    slug VARCHAR(255) NOT NULL UNIQUE,
    name VARCHAR(255) NOT NULL,
    description TEXT,
    system_prompt TEXT NOT NULL,
    default_model_id UUID REFERENCES llm_models(id),
    temperature REAL,
    max_tokens INTEGER,
    tools JSONB NOT NULL DEFAULT '[]',
    tags TEXT[] NOT NULL DEFAULT '{}',
    status VARCHAR(50) NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Sessions table
CREATE TABLE sessions (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    agent_id UUID NOT NULL REFERENCES agents(id),
    title VARCHAR(255),
    tags TEXT[] NOT NULL DEFAULT '{}',
    model_id UUID REFERENCES llm_models(id),
    status VARCHAR(50) NOT NULL DEFAULT 'pending',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    finished_at TIMESTAMPTZ
);

-- Messages table (primary conversation data)
CREATE TABLE messages (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    role VARCHAR(50) NOT NULL,
    content JSONB NOT NULL,
    tool_call_id VARCHAR(255),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);

-- Events table (SSE notification stream)
CREATE TABLE events (
    id UUID PRIMARY KEY DEFAULT uuidv7(),
    session_id UUID NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    sequence INTEGER NOT NULL,
    event_type VARCHAR(100) NOT NULL,
    data JSONB NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE(session_id, sequence)
);
```

## API Endpoints

### Agent Management
- `POST /v1/agents` - Create agent
- `GET /v1/agents` - List agents → `{ data: [...] }`
- `GET /v1/agents/{agent_id}` - Get agent
- `PATCH /v1/agents/{agent_id}` - Update agent
- `DELETE /v1/agents/{agent_id}` - Delete/archive agent

### Session Management
- `POST /v1/agents/{agent_id}/sessions` - Create session
- `GET /v1/agents/{agent_id}/sessions` - List sessions → `{ data: [...] }`
- `GET /v1/agents/{agent_id}/sessions/{session_id}` - Get session
- `DELETE /v1/agents/{agent_id}/sessions/{session_id}` - Delete session

### Message Management
- `POST /v1/agents/{agent_id}/sessions/{session_id}/messages` - Create message (triggers workflow for user messages)
- `GET /v1/agents/{agent_id}/sessions/{session_id}/messages` - List messages → `{ data: [...] }`

### Event Streaming
- `GET /v1/agents/{agent_id}/sessions/{session_id}/events` - SSE event stream

## Implementation Plan

### Phase 1: Update Specs
1. Update `specs/models.md` with correct model
2. Update `specs/apis.md` with correct endpoints

### Phase 2: Database
1. Create migration that:
   - Drops old tables (harnesses, session_events, etc.)
   - Creates new tables (agents, sessions, messages, events)

### Phase 3: Storage Layer
1. Add models: AgentRow, SessionRow, MessageRow, EventRow
2. Add CRUD operations for all entities

### Phase 4: Contracts
1. Add DTOs: Agent, Session, Message, Event
2. Add request/response types

### Phase 5: API Layer
1. Add route handlers for all endpoints
2. Add services layer

### Phase 6: Worker Layer
1. Update workflow to create Messages (not just Events)
2. Emit Events as notifications alongside Message creation

### Phase 7: UI
1. Update pages for new model
2. Use Messages for display, Events for real-time updates

## Success Criteria

- [ ] Messages are the primary data store
- [ ] Events are purely notifications for UI
- [ ] Tool calls stored as Messages with role=tool_call
- [ ] Tool results stored as Messages with role=tool_result
- [ ] Event stream provides real-time updates during processing
- [ ] All tests pass
