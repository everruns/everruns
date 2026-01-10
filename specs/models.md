# Data Models Specification

## Abstract

This document defines the core data models for Everruns - a durable AI agent execution platform.

## Requirements

### Agent

Configuration for an agentic loop. An agent can have many concurrent sessions.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `name` | string | Display name |
| `description` | string? | Optional description |
| `system_prompt` | string | System prompt for the LLM |
| `default_model_id` | UUID? | Reference to llm_models table |
| `tags` | string[] | Tags for organization/filtering |
| `capabilities` | CapabilityId[] | Enabled capabilities |
| `status` | enum | `active` or `archived` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

**Input Validation Limits:**

Last-resort validation limits to guard against abuse. API returns generic `400 Bad Request` with message "Input exceeds allowed limits" when violated.

| Field | Max Size | Notes |
|-------|----------|-------|
| `name` | 2 KB | Display name |
| `description` | 10 KB | Optional description |
| `system_prompt` | 1 MB | Allows large prompts with embedded context |
| `capabilities` | 250 items | Maximum capabilities per agent |
| Import file | 3 MB | Maximum size for `/v1/agents/import` body |

### Session

An instance of agentic loop execution. Multiple sessions can exist concurrently for a single agent.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `agent_id` | UUID v7 | Parent agent reference |
| `title` | string? | Session title (user-provided or auto-generated) |
| `tags` | string[] | Tags for organization/filtering |
| `model_id` | UUID? | Override model (null = use agent default) |
| `status` | enum | `pending`, `running`, `failed` |
| `created_at` | timestamp | Creation time |
| `started_at` | timestamp? | Execution start time |
| `finished_at` | timestamp? | Completion time (only set on failure) |

Status transitions: `pending` → `running` → `pending` (cycles indefinitely) | `failed`

Sessions work indefinitely - after processing a message, status returns to `pending` (ready for more messages). Only `failed` is a terminal state.

### Message

Conversation data stored as events in the `events` table with `event_type` prefixed by `message.`. Messages are reconstructed from events when loaded.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier (stored in event.data.message_id) |
| `session_id` | UUID v7 | Parent session reference (from event.session_id) |
| `sequence` | integer | Order within session (from event.sequence) |
| `role` | enum | `user`, `assistant`, `tool_result` |
| `content` | ContentPart[] | Array of content parts (see below) |
| `controls` | Controls? | Runtime controls for message processing |
| `metadata` | object? | Message-level metadata (e.g., locale) |
| `tags` | string[] | Tags for organization/filtering |
| `created_at` | timestamp | Creation time (from event.created_at) |

**Note:** Messages are stored as events with types `message.user`, `message.agent`, `message.tool_result`. Tool calls are embedded in `message.agent` events via `ContentPart::ToolCall`. System messages are handled internally and not persisted to events.

**ContentPart types (discriminated by `type` field):**

```json
// type=text
{ "type": "text", "text": "Hello, how are you?" }

// type=image
{ "type": "image", "url": "https://..." }
// or
{ "type": "image", "base64": "...", "media_type": "image/png" }

// type=tool_call (assistant requesting tool execution)
{
  "type": "tool_call",
  "id": "call_abc123",
  "name": "search",
  "arguments": { "query": "test" }
}

// type=tool_result (result of tool execution)
{
  "type": "tool_result",
  "tool_call_id": "call_abc123",
  "result": { "matches": [...] },
  "error": null
}
```

**Controls structure:**

```json
{
  "model_id": "550e8400-e29b-41d4-a716-446655440000",
  "reasoning": {
    "effort": "medium"
  }
}
```

Controls are optional and allow per-message overrides for model selection and reasoning configuration.

**Model resolution priority:**

The model used for processing is determined using this priority chain:
1. `controls.model_id` (from the last user message) - highest priority
2. `session.model_id` - session-level override
3. `agent.default_model_id` - agent's default model
4. System default model - fallback if no model is configured above

Each level references a UUID that points to a configured model in the `llm_models` table.

**CreateMessageRequest structure:**

```json
{
  "message": {
    "role": "user",
    "content": [
      { "type": "text", "text": "Compare these two images." },
      { "type": "image", "url": "https://example.com/image1.png" }
    ]
  },
  "controls": {
    "model_id": "550e8400-e29b-41d4-a716-446655440000",
    "reasoning": { "effort": "medium" }
  },
  "metadata": {
    "locale": "en-US",
    "request_id": "req_123"
  },
  "tags": ["important", "review"]
}
```

**Note:** The `message.role` field defaults to `"user"` and can be omitted. Only `user` messages can be created via the API; `assistant`, `tool_result`, and `system` messages are created internally by the system.

**InputContentPart types (allowed in user messages):**

Only text and image content can be sent by users:
- `{ "type": "text", "text": "..." }`
- `{ "type": "image", "url": "..." }` or `{ "type": "image", "base64": "...", "media_type": "image/png" }`

Tool calls and tool results are system-generated and cannot be created via the API.

**Database storage:**

Messages are stored in the `events` table with the full content in the `data` JSONB field:

```json
// Event for a user message (event_type: "message.user")
{
  "message_id": "01234567-89ab-cdef-0123-456789abcdef",
  "role": "user",
  "content": [{"type": "text", "text": "Hello, how are you?"}],
  "controls": null,
  "metadata": null,
  "tags": []
}

// Event for an agent message with tool calls (event_type: "message.agent")
{
  "message_id": "...",
  "role": "assistant",
  "content": [
    {"type": "text", "text": "Let me search for that."},
    {"type": "tool_call", "id": "call_abc123", "name": "search", "arguments": {"query": "test"}}
  ],
  "controls": null,
  "metadata": null,
  "tags": []
}
```

### Event

The primary data store for conversation messages and SSE notifications.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session |
| `event_type` | string | Type of notification (see below) |
| `data` | JSON | Event-specific payload |
| `created_at` | timestamp | Event time |

**Event Type Naming Convention:**

Event types follow the pattern `{entity}.{action}` where:
- `entity` - The domain entity (e.g., `message`, `step`, `tool`, `session`)
- `action` - The action or state (e.g., `user`, `agent`, `started`, `completed`)

This convention ensures consistent, predictable event type names across the system.

**Event Types:**

1. **Message Events** - Primary conversation data (stored in `data` field)
   - `message.user` - User message
   - `message.agent` - Agent response (from LLM, may contain tool calls in content)
   - `message.tool_result` - Tool execution result

2. **Turn Events** - Turn lifecycle notifications
   - `turn.started` - Turn execution started
   - `turn.completed` - Turn completed successfully
   - `turn.failed` - Turn failed

3. **Atom Events** - Atom lifecycle notifications
   - `input.received` - User input received
   - `reason.started` - ReasonAtom started (LLM inference began)
   - `reason.completed` - ReasonAtom completed (LLM response received)
   - `act.started` - ActAtom started (tool batch execution)
   - `act.completed` - ActAtom completed

4. **Tool Events** - Individual tool execution
   - `tool.call_started` - Tool execution began
   - `tool.call_completed` - Tool execution finished (includes result)

5. **LLM Events** - LLM API visibility
   - `llm.generation` - Full LLM API call with messages and response

6. **Session Events** - Session lifecycle
   - `session.started` - Session began processing

## Flow Example

```
User sends: "How much is 2+2?"

1. POST /v1/agents/{id}/sessions/{id}/messages
   → Creates Message(role=user, content: { text: "How much is 2+2?" })
   → Emits Event(message.user)
   → Triggers session workflow

2. Workflow starts
   → Updates Session(status=running)
   → Emits Event(session.started)

3. Turn starts
   → Emits Event(turn.started)
   → Emits Event(input.received)

4. LLM call (ReasonAtom)
   → Emits Event(reason.started)
   → LLM responds
   → Creates Message(role=assistant, content: { text: "The answer is 4" })
   → Emits Event(reason.completed)
   → Emits Event(llm.generation)
   → Emits Event(message.agent)

5. Turn complete
   → Emits Event(turn.completed)
   → Updates Session(status=pending)

User can send another message to continue the conversation.
```

### Capability

Modular functionality that can be enabled on Agents. Capabilities contribute to system prompts, provide tools, and modify agent behavior.

| Field | Type | Description |
|-------|------|-------------|
| `id` | CapabilityId | Unique identifier (enum) |
| `name` | string | Display name |
| `description` | string | Description of functionality |
| `status` | enum | `available`, `coming_soon`, `deprecated` |
| `icon` | string? | Icon name for UI rendering |
| `category` | string? | Category for grouping in UI |

**Built-in Capability IDs:**

| ID | Status | Description |
|----|--------|-------------|
| `noop` | available | No-op capability for testing |
| `current_time` | available | Tool to get current date/time |
| `research` | coming_soon | Deep research with scratchpad |
| `sandbox` | coming_soon | Sandboxed code execution |
| `file_system` | coming_soon | File system access tools |

### AgentCapability

Junction table linking Agents to Capabilities with ordering.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `agent_id` | UUID v7 | Parent agent reference |
| `capability_id` | CapabilityId | Capability identifier |
| `position` | integer | Order in capability chain (lower = earlier) |
| `created_at` | timestamp | Creation time |

**Constraints:**
- Each agent can have each capability at most once (`UNIQUE(agent_id, capability_id)`)
- Capabilities are applied in `position` order when building agent configuration

### LLM Provider

Configuration for LLM API providers. Stores encrypted API keys and provider-specific settings.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `name` | string | Display name |
| `provider_type` | enum | `openai`, `anthropic`, `azure_openai` |
| `base_url` | string? | Custom API endpoint (for Azure or proxies) |
| `api_key_encrypted` | bytes? | AES-256-GCM encrypted API key |
| `api_key_set` | boolean | Whether API key is configured |
| `is_default` | boolean | Default provider for new agents |
| `status` | enum | `active` or `disabled` |
| `settings` | JSON | Provider-specific settings (e.g., Azure deployment_name) |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

**Supported Provider Types:**
- `openai` - OpenAI API (GPT-4o, o1, etc.)
- `anthropic` - Anthropic API (Claude models)
- `azure_openai` - Azure OpenAI Service

**Note:** Ollama and Custom provider types are no longer supported. LLM provider API keys are primarily configured in the database (via Settings > Providers UI), but environment variables can be used as fallbacks for development convenience.

**Default Providers:**

Default providers (OpenAI, Anthropic) and their models are created via database migration (`003_default_providers.sql`) on first startup. These providers have well-known UUIDs:

- OpenAI: `01933b5a-0000-7000-8000-000000000001`
- Anthropic: `01933b5a-0000-7000-8000-000000000002`

**API Key Resolution Order:**
1. **Database** (priority): Encrypted API key stored in `llm_providers.api_key_encrypted`
2. **Environment Variable** (fallback): `DEFAULT_OPENAI_API_KEY` or `DEFAULT_ANTHROPIC_API_KEY`

API keys can be configured via:
1. The Settings > Providers UI (stores in database)
2. The `scripts/patch-provider-keys.sh` script (patches database from `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`)
3. Environment variables for development: `DEFAULT_OPENAI_API_KEY`, `DEFAULT_ANTHROPIC_API_KEY` (used only when database key is not set)

### LLM Model

Configuration for a specific model within a provider.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `provider_id` | UUID v7 | Parent provider reference |
| `model_id` | string | Model identifier (e.g., "gpt-4o") |
| `display_name` | string | Display name |
| `features` | string[] | Model features (e.g., vision, function_calling, streaming) |
| `context_window` | integer? | Context window size |
| `is_default` | boolean | Default model for this provider |
| `status` | enum | `active` or `disabled` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

### LLM Model Profile

Read-only metadata describing model capabilities, costs, and limits. Profiles are computed at runtime (not stored in database) and attached to model responses.

**Data Source:** https://models.dev/api.json

| Field | Type | Description |
|-------|------|-------------|
| `name` | string | Display name (e.g., "GPT-4o") |
| `family` | string | Model family (e.g., "gpt-4o") |
| `release_date` | string? | Release date (YYYY-MM-DD) |
| `last_updated` | string? | Last update date (YYYY-MM-DD) |
| `attachment` | boolean | Supports file attachments |
| `reasoning` | boolean | Is a reasoning model |
| `temperature` | boolean | Supports temperature parameter |
| `knowledge` | string? | Knowledge cutoff date |
| `tool_call` | boolean | Supports tool/function calling |
| `structured_output` | boolean | Supports structured output |
| `open_weights` | boolean | Has open weights |
| `cost` | LlmModelCost? | Pricing per million tokens |
| `limits` | LlmModelLimits? | Context and output limits |
| `modalities` | LlmModelModalities? | Input/output modalities |
| `reasoning_effort` | ReasoningEffortConfig? | Reasoning effort options |

**LlmModelCost:**

| Field | Type | Description |
|-------|------|-------------|
| `input` | float | Input cost per million tokens (USD) |
| `output` | float | Output cost per million tokens (USD) |
| `cache_read` | float? | Cached input cost per million tokens |

**LlmModelLimits:**

| Field | Type | Description |
|-------|------|-------------|
| `context` | integer | Maximum context window tokens |
| `output` | integer | Maximum output tokens |

**LlmModelModalities:**

| Field | Type | Description |
|-------|------|-------------|
| `input` | Modality[] | Supported input types (text, image, audio, video) |
| `output` | Modality[] | Supported output types |

**ReasoningEffortConfig:**

Configuration for reasoning models (OpenAI o1, o1-mini, o3-mini, o1-pro).

| Field | Type | Description |
|-------|------|-------------|
| `values` | ReasoningEffortValue[] | Available effort levels |
| `default` | ReasoningEffort | Default effort level |

**ReasoningEffort enum:** `none`, `minimal`, `low`, `medium`, `high`, `xhigh`

**Supported Models:**

- **OpenAI:** gpt-4o, gpt-4o-mini, o1, o1-mini, o1-pro, o3-mini
- **Anthropic:** claude-sonnet-4, claude-opus-4, claude-3-5-sonnet, claude-3-5-haiku, claude-3-opus, claude-3-sonnet, claude-3-haiku

Profiles are matched by provider_type + model_id with version normalization (e.g., "gpt-4o-2024-11-20" → "gpt-4o").

## Design Decisions

| Question | Decision |
|----------|----------|
| What stores conversation? | **Events** table with `event_type` = `message.*` |
| What are Events for? | Primary data store for messages AND SSE notifications |
| Where are tool calls stored? | In `message.agent` events as `ContentPart::ToolCall` |
| Where are tool results stored? | Events with `event_type` = `message.tool_result` |
| Session status? | Explicit status field (pending, running, failed) |
| Where are capabilities defined? | In-memory registry in API layer |
| How are capabilities applied? | Resolved at API/service layer, merged into RuntimeAgent |
| Where are API keys stored? | Encrypted in database (llm_providers.api_key_encrypted), decrypted at runtime |
| Environment variables for API keys? | No - all API keys must be configured via database/UI |
