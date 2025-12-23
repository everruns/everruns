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
| `status` | enum | `active` or `archived` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

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

The primary conversation data. Stores all conversation content including user messages, assistant responses, tool calls, and tool results.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session (auto-increment per session) |
| `role` | enum | `user`, `assistant`, `tool_call`, `tool_result`, `system` |
| `content` | JSON | Message content (schema depends on role) |
| `tool_call_id` | string? | For tool_result, references the tool_call id |
| `created_at` | timestamp | Creation time |

**Content schemas by role:**

```json
// role=user, assistant, or system
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

### Event

SSE notification stream for real-time UI updates. **Not the primary data store** - use Messages for that.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `session_id` | UUID v7 | Parent session reference |
| `sequence` | integer | Order within session |
| `event_type` | string | Type of notification (see below) |
| `data` | JSON | Event-specific payload |
| `created_at` | timestamp | Event time |

**Event Types:**

1. **Step Events** - Workflow progress notifications
   - `step.started` - Started processing (e.g., LLM call began)
   - `step.generating` - Generation in progress (streaming delta)
   - `step.generated` - Generation complete
   - `step.error` - Step failed

2. **Message Events** - Notifications about messages
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
User sends: "How much is 2+2?"

1. POST /v1/agents/{id}/sessions/{id}/messages
   → Creates Message(role=user, content: { text: "How much is 2+2?" })
   → Triggers session workflow

2. Workflow starts
   → Updates Session(status=running)
   → Emits Event(session.started)

3. LLM call starts
   → Emits Event(step.started)

4. LLM responds (non-streaming for M2)
   → Creates Message(role=assistant, content: { text: "The answer is 4" })
   → Emits Event(step.finished)
   → Emits Event(message.created, data: { message_id: "..." })

5. Session cycle complete (ready for more messages)
   → Updates Session(status=pending)
   → Emits Event(session.finished)

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

**Note:** Ollama and Custom provider types are no longer supported. All LLM provider API keys must be configured in the database (via Settings > Providers UI) - they are not read from environment variables.

### LLM Model

Configuration for a specific model within a provider.

| Field | Type | Description |
|-------|------|-------------|
| `id` | UUID v7 | Unique identifier |
| `provider_id` | UUID v7 | Parent provider reference |
| `model_id` | string | Model identifier (e.g., "gpt-4o") |
| `display_name` | string | Display name |
| `capabilities` | string[] | Model capabilities |
| `context_window` | integer? | Context window size |
| `is_default` | boolean | Default model for this provider |
| `status` | enum | `active` or `disabled` |
| `created_at` | timestamp | Creation time |
| `updated_at` | timestamp | Last modification time |

## Design Decisions

| Question | Decision |
|----------|----------|
| What stores conversation? | **Messages** table - primary data |
| What are Events for? | SSE notifications for real-time UI updates |
| Where are tool calls stored? | Messages with role=tool_call |
| Where are tool results stored? | Messages with role=tool_result |
| Session status? | Explicit status field (pending, running, completed, failed) |
| Where are capabilities defined? | In-memory registry in API layer |
| How are capabilities applied? | Resolved at API/service layer, merged into AgentConfig |
| Where are API keys stored? | Encrypted in database (llm_providers.api_key_encrypted), decrypted at runtime |
| Environment variables for API keys? | No - all API keys must be configured via database/UI |
