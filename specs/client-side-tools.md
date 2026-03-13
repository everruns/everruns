# Client-Side Tools Specification

## Abstract

Client-side tools let API/SDK consumers define tools that execute on the client, not the server. When the LLM requests a client-side tool call, the server pauses execution, emits a `tool.call_requested` event, and waits for the client to submit results via API. This enables integrations where tool logic lives outside Everruns (e.g., browser actions, local file access, proprietary APIs).

## Requirements

### Concept

Standard (server-side) tools execute within the Everruns worker process. Client-side tools invert this: the server advertises the tool to the LLM, but delegates execution to the calling client. The agent loop pauses until the client submits results or the wait is cancelled.

```
┌────────┐       ┌──────────┐       ┌─────┐       ┌────────┐
│ Client │       │  Server  │       │ LLM │       │ Client │
└───┬────┘       └────┬─────┘       └──┬──┘       └───┬────┘
    │  POST message    │                │              │
    │─────────────────>│                │              │
    │                  │  LLM call      │              │
    │                  │───────────────>│              │
    │                  │  tool_call     │              │
    │                  │<───────────────│              │
    │                  │                │              │
    │  SSE: tool.call_requested        │              │
    │<─────────────────│                │              │
    │                  │ (paused, waiting_for_tool_results)
    │                  │                │              │
    │  POST tool-results               │              │
    │─────────────────>│                │              │
    │                  │  LLM call      │              │
    │                  │───────────────>│              │
    │                  │  final text    │              │
    │                  │<───────────────│              │
    │  SSE: output.message.completed   │              │
    │<─────────────────│                │              │
```

### Tool Definition

Client-side tools are defined on the **agent** using `type: "client"`. They follow the same JSON Schema format as server-side tools but are not backed by a server implementation.

```json
{
  "type": "client",
  "name": "lookup_crm",
  "description": "Look up a customer record in the CRM system",
  "parameters": {
    "type": "object",
    "properties": {
      "customer_id": {
        "type": "string",
        "description": "The customer ID to look up"
      }
    },
    "required": ["customer_id"]
  }
}
```

**Fields:**

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | Yes | Must be `"client"` |
| `name` | string | Yes | Tool name (sent to LLM) |
| `description` | string | Yes | Tool description (sent to LLM) |
| `parameters` | object | Yes | JSON Schema for tool arguments |

Client-side tools are registered on the agent alongside capabilities. They are included in the LLM tool definitions but have no server-side executor.

### API Flow

#### 1. Create Agent with Client-Side Tools

```http
POST /v1/agents
Content-Type: application/json

{
  "name": "CRM Assistant",
  "system_prompt": "You help users look up customer information.",
  "capabilities": [
    { "ref": "current_time" }
  ],
  "client_tools": [
    {
      "type": "client",
      "name": "lookup_crm",
      "description": "Look up a customer record in the CRM system",
      "parameters": {
        "type": "object",
        "properties": {
          "customer_id": {
            "type": "string",
            "description": "The customer ID to look up"
          }
        },
        "required": ["customer_id"]
      }
    }
  ]
}
```

#### 2. Create Session

```http
POST /v1/sessions
Content-Type: application/json

{
  "agent_id": "agent_01234567-..."
}
```

#### 3. Send Message (Triggers Turn)

```http
POST /v1/sessions/{session_id}/messages
Content-Type: application/json

{
  "message": {
    "content": [
      { "type": "text", "text": "Look up customer CUST-42" }
    ]
  }
}
```

#### 4. Receive `tool.call_requested` Event via SSE

The server calls the LLM, which returns a tool call for `lookup_crm`. Since this is a client-side tool, the server does **not** execute it. Instead, it emits a `tool.call_requested` event and pauses.

```
event: tool.call_requested
data: {
  "id": "01937abc-...",
  "type": "tool.call_requested",
  "ts": "2024-01-15T10:30:01.000Z",
  "session_id": "session_01234567-...",
  "context": {
    "turn_id": "turn_01234567-...",
    "input_message_id": "message_01234567-..."
  },
  "data": {
    "tool_calls": [
      {
        "id": "call_abc123",
        "name": "lookup_crm",
        "arguments": { "customer_id": "CUST-42" }
      }
    ],
    "tool_summaries": [
      {
        "id": "call_abc123",
        "name": "lookup_crm",
        "display_name": "Lookup CRM",
        "narration": "Looking up customer CUST-42"
      }
    ],
    "headline": "Looking up customer CUST-42"
  }
}
```

At this point, the session status transitions to `waiting_for_tool_results`.

#### 5. Submit Tool Results

```http
POST /v1/sessions/{session_id}/tool-results
Content-Type: application/json

{
  "tool_results": [
    {
      "tool_call_id": "call_abc123",
      "result": {
        "name": "Alice Johnson",
        "email": "alice@example.com",
        "plan": "enterprise",
        "since": "2022-03-15"
      }
    }
  ]
}
```

**Response:** `200 OK`

```json
{
  "status": "accepted",
  "tool_results_count": 1
}
```

The server resumes the agent loop: it feeds the tool results back to the LLM, which produces a final text response streamed via `output.message.completed`.

#### 6. Receive Final Response via SSE

```
event: output.message.completed
data: {
  "type": "output.message.completed",
  ...
  "data": {
    "message": {
      "role": "agent",
      "content": [
        { "type": "text", "text": "Customer CUST-42 is Alice Johnson (alice@example.com), on the enterprise plan since March 2022." }
      ]
    }
  }
}
```

### Event Types

#### `tool.call_requested`

Emitted when the LLM requests one or more client-side tool calls. The agent loop pauses until results are submitted.

```json
{
  "id": "...",
  "type": "tool.call_requested",
  "ts": "...",
  "session_id": "...",
  "context": {
    "turn_id": "...",
    "input_message_id": "..."
  },
  "data": {
    "tool_calls": [
      {
        "id": "call_abc123",
        "name": "lookup_crm",
        "arguments": { "customer_id": "CUST-42" }
      }
    ]
  }
}
```

**Fields (data):**

| Field | Type | Description |
|-------|------|-------------|
| `tool_calls` | array | Client-side tool calls requested by the LLM |
| `tool_calls[].id` | string | Unique tool call ID (from LLM response) |
| `tool_calls[].name` | string | Tool name |
| `tool_calls[].arguments` | object | Tool arguments (parsed JSON) |
| `tool_summaries` | array | Optional server-authored display summaries for timeline UIs |
| `headline` | string | Optional server-authored readable summary for the requested batch |

### Session Status: `waiting_for_tool_results`

When a client-side tool call is requested, the session transitions to `waiting_for_tool_results`. This is a new status in addition to the existing `started`, `active`, `idle` states.

```
Status transitions:
  started → active → waiting_for_tool_results → active → idle
                            │
                            └──→ idle  (if user sends a new message, cancelling the wait)
```

**Visibility:** The `GET /v1/sessions/{session_id}` response includes this status so clients can determine whether to show a "waiting for tool results" indicator.

### Tool Results Endpoint

#### `POST /v1/sessions/{session_id}/tool-results`

Submit results for pending client-side tool calls. Resumes the agent loop.

**Request:**

```json
{
  "tool_results": [
    {
      "tool_call_id": "call_abc123",
      "result": { "key": "value" },
      "error": null
    }
  ]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `tool_results` | array | Yes | One result per requested tool call |
| `tool_results[].tool_call_id` | string | Yes | Must match an `id` from `tool.call_requested` |
| `tool_results[].result` | any | No | Tool execution result (JSON value). Required if `error` is null. |
| `tool_results[].error` | string | No | Error message if tool execution failed on client side |

**Response (success):**

```json
{
  "status": "accepted",
  "tool_results_count": 1
}
```

**Error responses:**

| Status | Condition |
|--------|-----------|
| `400` | Session is not in `waiting_for_tool_results` status |
| `400` | Missing or extra tool call IDs (must match exactly) |
| `404` | Session not found |

### Graceful Handling: User Message While Waiting

If the user sends a new message via `POST /v1/sessions/{session_id}/messages` while the session is in `waiting_for_tool_results` status, the pending tool wait is **cancelled**:

1. The pending client-side tool calls are discarded
2. Tool results are synthesized as `{"error": "Cancelled: user sent a new message"}`
3. The new user message starts a fresh turn
4. Session status transitions: `waiting_for_tool_results` → `active`

This prevents sessions from getting stuck if the client abandons the tool call flow.

### Mixed Tool Calls (Server + Client)

An LLM response may contain both server-side and client-side tool calls in the same response. The server handles this as follows:

1. **Execute server-side tools immediately** - All server-side tools run as normal
2. **Collect client-side tool calls** - Set aside for the `tool.call_requested` event
3. **Emit events for both**:
   - `tool.started` / `tool.completed` for each server-side tool (as today)
   - `tool.call_requested` for the batch of client-side tool calls
4. **Pause** - Wait for client to submit results for client-side tools
5. **Resume** - Feed all results (server + client) to the LLM in the next iteration

**Ordering:** Server-side tools execute first. Client-side tools pause after server-side tools complete. This ensures deterministic behavior and avoids partial result submission.

### Agent Model Changes

The `Agent` model gains a new optional field:

| Field | Type | Description |
|-------|------|-------------|
| `client_tools` | ClientToolDef[] | Client-side tool definitions (default: empty) |

Client tools are stored in the `agents` table as a JSONB column:

```sql
ALTER TABLE agents ADD COLUMN client_tools JSONB NOT NULL DEFAULT '[]';
```

**Input Validation:**

| Field | Max Size | Notes |
|-------|----------|-------|
| `client_tools` | 50 items | Maximum client-side tools per agent |
| `client_tools[].name` | 64 chars | Tool name |
| `client_tools[].description` | 1 KB | Tool description |
| `client_tools[].parameters` | 10 KB | JSON Schema |

### Timeout Behavior

Client-side tool waits have a configurable timeout (default: 5 minutes). If no results are submitted within the timeout:

1. Tool results are synthesized as `{"error": "Timed out waiting for client tool results"}`
2. The agent loop resumes with the error results
3. The LLM can interpret the timeout and respond accordingly
4. Session status transitions: `waiting_for_tool_results` → `active` → `idle`

### Security Considerations

1. **Tool call ID validation**: Submitted `tool_call_id` values must exactly match pending requests. Extra or missing IDs are rejected.
2. **Session scoping**: Tool results can only be submitted by clients with access to the session (same auth as other session endpoints).
3. **Result size limits**: Tool result payloads are limited to 100 KB per result to prevent abuse.
4. **No server-side execution**: Client-side tool definitions never trigger server-side code. They are metadata only.

### Design Decisions

| Question | Decision |
|----------|----------|
| Where are client tools defined? | On the Agent, in `client_tools` JSONB column |
| How does the LLM see them? | Merged with server-side tools in the tool definitions sent to LLM |
| What happens on tool call? | Server emits `tool.call_requested`, pauses until results submitted |
| Mixed server+client calls? | Server tools execute first, then pause for client tools |
| User message while waiting? | Cancels the wait, starts new turn |
| Timeout? | 5 minute default, synthesized error result on expiry |
| Can session capabilities add client tools? | Not in initial version; agent-level only |
