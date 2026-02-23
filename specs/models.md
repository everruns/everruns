# Data Models Specification

## Abstract

This document defines the core data models for Everruns - a durable agentic harness engine.

## Requirements

### Agent

Configuration for an agentic loop. An agent can have many concurrent sessions.

See [Agent struct](../crates/core/src/agent.rs) for field definitions.

**Key design points:**
- All entity IDs use the dual-ID pattern (internal UUID PK + external public_id). See `specs/id-schema.md` for details.
- `AgentId` uses `agent_` prefix. Client-supplied or auto-generated.

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

An instance of agentic loop execution. Sessions are top-level entities under organizations, with an agent assigned to work in each session.

See [Session struct](../crates/core/src/session.rs) for field definitions.

**Key design points:**

- **Organization-scoped:** Sessions are direct children of organizations (not agents). The `agent_id` specifies which agent is assigned to work in the session. This allows for organization-wide session management and future flexibility to reassign agents.
- **Session Features:** The `features` field is computed at read time by aggregating `features()` from all active capabilities (harness + agent + session-level), after resolving dependencies. It is not stored in the database. See `specs/capabilities.md#capability-features`.
- **Session Capabilities:** The `capabilities` field allows setting session-level capabilities that are additive to the agent's capabilities. When building the RuntimeAgent, agent capabilities are applied first, then session capabilities are applied after. This enables temporarily extending an agent's capabilities for specific sessions.
- **Status transitions:** `started` → `active` (processing) → `idle` (waiting for input). Sessions work indefinitely - after processing a message, status returns to `idle` (ready for more messages).

### Message

Conversation data stored as events in the `events` table. Messages are reconstructed from events when loaded.

See [Message struct](../crates/core/src/message.rs) for field definitions and [ContentPart enum](../crates/core/src/message.rs) for content part types.

**Key design points:**

- Messages are stored as events with types `input.message`, `output.message.completed`. Tool calls are embedded in `output.message.completed` events via `ContentPart::ToolCall`. Tool results are stored as `tool.completed` events. System messages are handled internally and not persisted to events.
- `image_file` content parts reference uploaded images. During LLM calls, references are resolved to actual image data using the `ImageResolver` trait. See [Image Resolution](#image-resolution).
- Only `user` messages can be created via the API; `agent`, `tool_result`, and `system` messages are created internally.

**Extended Thinking:**

When `controls.reasoning.effort` is set, models that support extended thinking (e.g., Anthropic Claude) generate internal reasoning before producing a response. Both `thinking` and `thinking_signature` fields must be preserved and sent back to the Anthropic API in subsequent turns. See [LLM Drivers spec](llm-drivers.md) for provider-specific requirements.

**Model resolution priority:**

1. `controls.model_id` (from the last user message) - highest priority
2. `session.model_id` - session-level override
3. `agent.default_model_id` - agent's default model
4. System default model - fallback if no model is configured above

### Image

Global storage for uploaded images. Images can be attached to messages via the `image_file` content part type.

**Constraints:**
- Maximum file size: 100MB
- Request body limit: 101MB (100MB file + 1MB multipart overhead)
- Allowed content types: image/png, image/jpeg, image/gif, image/webp
- Thumbnails generated automatically using Lanczos3 scaling

**Storage:**
- PostgreSQL: Full images stored in BYTEA columns
- In-memory (DEV_MODE): Images lost on restart
- Future: S3 storage planned

### Image Resolution

When messages containing `image_file` content parts are sent to an LLM, the system resolves these references to actual image data. This is handled by the `ImageResolver` trait, implemented by `GrpcImageResolver` in the worker.

**Resolution Process:**

1. **Extract IDs**: All unique `image_id` values are extracted from message content parts
2. **Batch Resolve**: Images are resolved via gRPC (worker → control-plane)
3. **Convert to Data URLs**: Resolved images are converted to `data:` URLs
4. **Provider Formatting**: Each LLM provider converts data URLs to their native format

**Provider-Specific Formats:**

OpenAI Vision:
```json
{
  "type": "image_url",
  "image_url": { "url": "data:image/png;base64,..." }
}
```

Anthropic Vision:
```json
{
  "type": "image",
  "source": {
    "type": "base64",
    "media_type": "image/png",
    "data": "..."
  }
}
```

**gRPC Transfer:**

Image data is transferred from control-plane to worker via gRPC. The gRPC message size limit is increased from 4MB (default) to 150MB to accommodate base64-encoded images (100MB raw + ~33% encoding overhead + metadata).

> **Warning:** The 150MB gRPC limit is a temporary workaround and should be removed in favor of a proper solution. Transferring large images inline via gRPC is inefficient and increases memory pressure on both control-plane and worker.
>
> **Recommended future approach:**
> - Presigned URLs: Worker fetches images directly from S3/blob storage
> - Streaming: Transfer images in chunks rather than single large messages
> - Direct storage access: Worker has read access to image storage
>
> When implementing one of these solutions, revert gRPC limit to default 4MB.

**Error Handling:**

- Missing images: Replaced with placeholder text `[Image not found: {id}]`
- Resolution failures: Logged as warnings, image treated as missing
- The LLM call proceeds even if some images cannot be resolved

### Event

The primary data store for conversation messages and SSE notifications.

See [Event struct](../crates/core/src/events.rs) for field definitions and [event type constants](../crates/core/src/events.rs) for the full list of event types.

**Storage Guarantees:**

1. **Append-Only** - Events are immutable. UPDATE and DELETE operations are blocked at the database level via triggers. Any attempt to modify existing events will fail with error "events are append-only".

2. **Atomic Per-Session Sequence** - Sequence numbers are allocated atomically per session using a dedicated `event_sequences` table. This prevents race conditions during concurrent writes that could occur with `MAX(sequence)+1` approach.

3. **Event Type Consistency** - The `event_type` field must match the type indicated by the `data` payload. This is validated at the service layer before storage. Raw/legacy events are exempt from this check.

**Event Type Naming Convention:**

Event types follow the pattern `{entity}.{action}` (e.g., `turn.started`, `tool.completed`). See [events.md](events.md) for event categories and lifecycle patterns.

**Message Reconstruction:**

Messages are reconstructed from events with types: `input.message`, `output.message.completed`, `tool.completed`. Tool calls are embedded in `output.message.completed` events via `ContentPart::ToolCall`. Tool results come from `tool.completed` events.

## Flow Example

```
User sends: "How much is 2+2?"

1. POST /v1/agents/{id}/sessions/{id}/messages
   → Creates Message(role=user, content: { text: "How much is 2+2?" })
   → Emits Event(input.message)
   → Triggers session workflow

2. Workflow starts
   → Updates Session(status=running)
   → Emits Event(session.started)

3. Turn starts
   → Emits Event(turn.started)

4. LLM call (ReasonAtom)
   → Emits Event(reason.started)
   → Emits Event(output.message.started)
   → LLM streams response
   → Emits Event(output.message.delta) (batched)
   → Creates Message(role=agent, content: { text: "The answer is 4" })
   → Emits Event(reason.completed)
   → Emits Event(llm.generation)
   → Emits Event(output.message.completed)

5. Turn complete
   → Emits Event(turn.completed)
   → Updates Session(status=pending)

User can send another message to continue the conversation.
```

### Capability

Modular functionality that can be enabled on Agents. Capabilities contribute to system prompts, provide tools, and modify agent behavior.

See [Capability trait](../crates/core/src/capabilities/mod.rs) for the trait definition and [CapabilityRegistry](../crates/core/src/capabilities/mod.rs) for the list of built-in capabilities. See also [capabilities.md](capabilities.md) for the full capability system specification.

### AgentCapability

Junction table linking Agents to Capabilities with ordering.

**Constraints:**
- Each agent can have each capability at most once (`UNIQUE(agent_id, capability_id)`)
- Capabilities are applied in `position` order when building agent configuration

### LLM Provider

Configuration for LLM API providers. Stores encrypted API keys and provider-specific settings.

See [LlmProvider struct](../crates/core/src/llm_models.rs) for field definitions.

**Key design points:**

- **Provider types:** `openai` (Responses API), `openai_completions` (Chat Completions, legacy), `anthropic`, `gemini`, `llmsim`. Ollama and Custom are no longer supported.
- `provider_type` is intentionally stored as a plain string without a database CHECK constraint. Provider validation and graceful unknown-provider handling are implemented in the application layer for forward compatibility.
- API keys are primarily configured in the database (via Settings > Providers UI), but environment variables can be used as fallbacks for development convenience.

**Default Providers:**

Default providers (OpenAI, Anthropic, Gemini, LlmSim) and their models are seeded on startup via the service seeding system (`server/src/seed.rs`). Seeding is idempotent (uses `ON CONFLICT DO NOTHING`) and runs in a background task. These providers have well-known UUIDs:

- OpenAI: `01933b5a-0000-7000-8000-000000000001`
- Anthropic: `01933b5a-0000-7000-8000-000000000002`
- LlmSim: `01933b5a-0000-7000-8000-000000000003`
- Gemini: `01933b5a-0000-7000-8000-000000000004`

Model UUIDs follow a range allocation scheme:
- `0x001-0x0FF`: LLM Providers
- `0x100-0x1FF`: Agents (seed agents like Dad Jokes Agent, Research Agent)
- `0x200-0x2FF`: OpenAI Models
- `0x300-0x3FF`: Anthropic Models
- `0x400-0x4FF`: LlmSim Models
- `0x600-0x6FF`: Gemini Models

**API Key Resolution Order:**
1. **Database** (priority): Encrypted API key stored in `llm_providers.api_key_encrypted`
2. **Environment Variable** (fallback): `DEFAULT_OPENAI_API_KEY` or `DEFAULT_ANTHROPIC_API_KEY`

API keys can be configured via:
1. The Settings > Providers UI (stores in database)
2. The `scripts/patch-provider-keys.sh` script (patches database from `OPENAI_API_KEY`, `ANTHROPIC_API_KEY`)
3. Environment variables for development: `DEFAULT_OPENAI_API_KEY`, `DEFAULT_ANTHROPIC_API_KEY` (used only when database key is not set)

### LLM Model

Configuration for a specific model within a provider.

See [LlmModel struct](../crates/core/src/llm_models.rs) for field definitions.

**Key design points:**

- **Model sources:** `manual` (user-added), `discovered` (from provider API), `predefined` (seeded on startup).
- **Stale model detection:** Discovered models become "stale" when `last_seen_at < provider.last_synced_at`. Stale models are kept in the database (not deleted) to preserve any user customizations.

### LLM Model Profile

Read-only metadata describing model capabilities, costs, and limits. Profiles are computed at runtime (not stored in database) and attached to model responses.

**Data Source:** https://models.dev/api.json

See [LlmModelProfile struct](../crates/core/src/llm_models.rs) for field definitions.

**Key design points:**
- Profiles include cost, limits, modalities, and reasoning effort configuration
- Profiles are matched by provider_type + model_id with version normalization (e.g., "gpt-4o-2024-11-20" → "gpt-4o")

### Model Discovery

Automatic discovery of available models from provider APIs.

**Supported Providers:**
- OpenAI - via `GET /v1/models`
- Anthropic - via `GET /v1/models`

**Discovery Flow:**

1. **Background Sync** - Every 24 hours (configurable via `MODEL_SYNC_INTERVAL_HOURS`, set to 0 to disable)
2. **Manual Sync** - `POST /v1/llm-providers/:id/sync-models`

**Sync Behavior:**

- Only providers with standard base URLs are synced (custom URLs are skipped)
- OpenAI models are filtered to chat/completion models only (excludes embeddings, TTS, image models)
- New models are automatically added with `source: "discovered"`
- Existing discovered models have `last_seen_at` updated
- Models not seen in sync become stale (detected via `last_seen_at < last_synced_at`)

**Listing Models:**

`GET /v1/llm-models` supports query parameters:
- `source` - Filter by source (`manual`, `discovered`, `predefined`)
- `include_stale` - Include stale models (default: true)
- `favorites_only` - Only return favorites (default: false)

### UserConnection

A linked external service account. User-scoped (not org-scoped). See [user-connections.md](user-connections.md) for full specification.

## Design Decisions

| Question | Decision |
|----------|----------|
| What stores conversation? | **Events** table with `event_type` = `message.*` |
| What are Events for? | Primary data store for messages AND SSE notifications |
| Where are tool calls stored? | In `output.message.completed` events as `ContentPart::ToolCall` |
| Where are tool results stored? | Events with `event_type` = `tool.completed` |
| Session status? | Explicit status field (pending, running, failed) |
| Where are capabilities defined? | In-memory registry in API layer |
| How are capabilities applied? | Resolved at API/service layer, merged into RuntimeAgent |
| Where are API keys stored? | Encrypted in database (llm_providers.api_key_encrypted), decrypted at runtime |
| Environment variables for API keys? | Yes - `DEFAULT_OPENAI_API_KEY` and `DEFAULT_ANTHROPIC_API_KEY` serve as fallbacks when database key is not set |
