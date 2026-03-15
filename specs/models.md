# Data Models Specification

## Abstract

This document defines the core data models for Everruns - a durable agentic harness engine. For full field definitions, see the Rust source types linked below.

## Requirements

### Agent

Configuration for an agentic loop. An agent can have many concurrent sessions.

See `crates/core/src/agent.rs` for full field definitions.

Key design points:
- All entity IDs use the dual-ID pattern (internal UUID PK + external public_id). See `specs/id-schema.md`.
- `capabilities` field stores enabled capability references (resolved at runtime from registry)
- `status`: `active`, `archived`, or `deleted`
- `archived_at` and `deleted_at` capture lifecycle timestamps

### Building Block Lifecycle

The default lifecycle for user-managed building blocks is:

`active -> archived -> deleted`

Applies to:
- Agents
- Harnesses (except built-ins)
- Skills
- MCP servers visible in the MCP tab
- Apps

Contract:
- `archived` means read-only, not assignable, not executable, hidden from lists by default, visible when explicitly filtered in. The filter label is always "Show archived" (no entity suffix) — the page context already communicates what entity type is listed.
- `deleted` is a tombstone state used only to preserve historical references. Normal detail APIs return `404`, normal lists exclude deleted items, and runtime execution must not use them.
- Existing references are preserved by ID. UI/API reference surfaces render tombstones like `<Deleted Agent>` instead of resolving the deleted entity normally.
- If a session or app references an archived or deleted dependency, execution must stop gracefully on the next atom with a user-visible explanation instead of crashing.
- Built-in/system-managed entities do not participate in archive/delete flows.

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

See `crates/core/src/session.rs` for full field definitions.

See `specs/localization.md` for locale/timezone resolution and durable preference rules.

Key design points:
- Sessions are direct children of organizations (not agents). The `agent_id` specifies which agent is assigned.
- `locale` is an optional session-level BCP 47 tag (for example `uk-UA`). The worker carries it through turn loading and prompt construction so scheduled runs, resumed runs, and subagents can inherit localized behavior.
- `timezone` should be a separate optional session-level IANA timezone for unattended execution defaults. Interactive turns may override it with live browser timezone for that turn.
- `features` field is computed at read time by aggregating `features()` from all active capabilities (not stored). See `specs/capabilities.md#capability-features`.
- `capabilities` allows session-level capabilities additive to the agent's. Agent capabilities applied first, then session capabilities.
- Status transitions: `started` → `active` (processing) → `idle` (waiting for input)
- Sessions work indefinitely — after processing, status returns to `idle`.

### Message

Conversation data stored as events in the `events` table. Messages are reconstructed from events when loaded.

See `crates/core/src/message.rs` for `Message`, `ContentPart`, `Controls`, and `InputContentPart` types.

Key design points:
- Messages stored as events with types `input.message`, `output.message.completed`. Tool calls embedded in `output.message.completed` via `ContentPart::ToolCall`. Tool results from `tool.completed` events.
- System messages handled internally, not persisted.
- `ContentPart` variants: `text`, `image`, `image_file`, `tool_call`, `tool_result`. Users can only send `text`, `image`, `image_file` via API.
- `metadata.locale` and `metadata.timezone` are reserved for one-turn execution-context overrides. See `specs/localization.md`.

### User Profile

Users should carry durable defaults for:
- `locale`
- `timezone`

These values are fallbacks for session creation and turn resolution. They are not substitutes for live browser timezone on interactive requests.

See `specs/localization.md` for precedence rules.

**Controls:**

Optional per-message overrides for model selection and reasoning configuration.

**Extended Thinking:**

When `controls.reasoning.effort` is set, reasoning models generate chain-of-thought before responding. The `thinking` and `thinking_signature` fields must be preserved for multi-turn conversations. See [LLM Drivers spec](llm-drivers.md) for provider-specific requirements.

**Model resolution priority:**

1. `controls.model_id` (from the last user message) - highest priority
2. `session.model_id` - session-level override
3. `agent.default_model_id` - agent's default model
4. System default model - fallback if no model is configured above

### Image

Global storage for uploaded images. Images can be attached to messages via the `image_file` content part type.

See `crates/server/src/storage/models.rs` for the `ImageRow` type.

**Constraints:**
- Maximum file size: 100MB (body limit: 101MB including multipart overhead)
- Allowed content types: image/png, image/jpeg, image/gif, image/webp
- Thumbnails generated automatically (max 200x200, Lanczos3 scaling)

**Storage:**
- PostgreSQL: Full images in BYTEA columns
- In-memory (DEV_MODE): Lost on restart
- Future: S3 storage planned

### Image Resolution

When messages containing `image_file` content parts are sent to an LLM, the system resolves references to actual image data via the `ImageResolver` trait (`GrpcImageResolver` in worker).

**Resolution Process:**
1. Extract unique `image_id` values from content parts
2. Batch resolve via gRPC (worker → control-plane)
3. Convert to `data:` URLs
4. Each LLM provider formats for its vision API

**gRPC Transfer:**

Image data transferred via gRPC with 150MB message limit (100MB raw + ~33% encoding overhead).

> **Warning:** The 150MB gRPC limit is a temporary workaround. Future: presigned URLs, streaming, or direct storage access. Revert to default 4MB when implemented.

**Error Handling:**
- Missing images: Replaced with `[Image not found: {id}]`
- Resolution failures: Logged as warnings, LLM call proceeds

### Event

The primary data store for conversation messages and SSE notifications. See `crates/core/src/events.rs` for `Event` and `EventData` types.

**Storage Guarantees:**

1. **Append-Only** — Events are immutable. UPDATE and DELETE blocked via database triggers.
2. **Atomic Per-Session Sequence** — Sequence numbers allocated atomically per session via `event_sequences` table (prevents race conditions).
3. **Event Type Consistency** — `event_type` field must match `data` payload type. Validated at service layer.

**Event Type Naming Convention:** `{entity}.{action}` pattern (e.g., `input.message`, `turn.completed`). See `specs/events.md` for the full event type registry and lifecycle details.

**Message Reconstruction:**

Messages reconstructed from events: `input.message`, `output.message.completed`, `tool.completed`. Tool calls embedded in output messages via `ContentPart::ToolCall`.

## Flow Example

```
User sends: "How much is 2+2?"

1. POST /v1/sessions/{id}/messages
   → Creates Message(role=user) → Emits input.message → Triggers workflow

2. Workflow starts → Session(status=active) → session.started

3. Turn starts → turn.started

4. LLM call (ReasonAtom)
   → reason.started → output.message.started
   → Streams output.message.delta (batched)
   → Creates Message(role=agent, "The answer is 4")
   → reason.completed → llm.generation → output.message.completed

5. Turn complete → turn.completed → Session(status=idle)

User can send another message to continue the conversation.
```

### Capability

Modular functionality that can be enabled on Agents. See `crates/core/src/capability_types.rs` for `CapabilityStatus`, `AgentCapabilityConfig`, and `MountPoint` types. See `crates/core/src/capabilities/mod.rs` for the `Capability` trait and `CapabilityRegistry`.

See `specs/capabilities.md` for the full capabilities specification.

### LLM Provider

Configuration for LLM API providers. See `crates/core/src/llm_models.rs` for full type definitions.

Key design points:
- `provider_type` stored as plain string without CHECK constraint (forward compatibility)
- Supported types: `openai`, `openai_completions`, `anthropic`, `gemini`, `llmsim`
- API keys encrypted with AES-256-GCM envelope encryption (see `specs/encryption.md`)

**API Key Resolution Order:**
1. **Database** (priority): Encrypted in `llm_providers.api_key_encrypted`
2. **Environment Variable** (fallback): `DEFAULT_OPENAI_API_KEY` or `DEFAULT_ANTHROPIC_API_KEY`

Default providers and models seeded on startup via `crates/server/src/seed.rs` (idempotent, well-known UUIDs).

### LLM Model

Configuration for a specific model within a provider. See `crates/core/src/llm_models.rs`.

Key design points:
- `source` enum: `manual` (user-added), `discovered` (from provider API), `predefined` (seeded)
- `installed` flag: only installed models appear in UI model pickers (Chat UI). All models remain available via API regardless of installed status. Default installed models: GPT-5.4, Opus 4.6, Sonnet 4.6, Haiku 4.6.
- Organization default model: stored in `organization_settings.default_model_id` (not on the model itself). Auto-elects a new default from installed models if the current default is uninstalled or deleted.
- Stale model detection: `last_seen_at < provider.last_synced_at` means model no longer returned by provider API. Stale models kept (not deleted) to preserve customizations.

### LLM Model Profile

Read-only metadata describing model capabilities, costs, and limits. Computed at runtime (not stored in database).

**Data Source:** https://models.dev/api.json

See `crates/core/src/llm_models.rs` for `LlmModelProfile`, `LlmModelCost`, `LlmModelLimits`, and `ReasoningEffortConfig` types.

Profiles matched by provider_type + model_id with version normalization (e.g., "gpt-4o-2024-11-20" → "gpt-4o").

### Model Discovery

Automatic discovery of available models from provider APIs (OpenAI, Anthropic).

- **Background Sync** — Every 24 hours (configurable via `MODEL_SYNC_INTERVAL_HOURS`, 0 to disable)
- **Manual Sync** — `POST /v1/llm-providers/:id/sync-models`
- Only providers with standard base URLs synced (custom URLs skipped)
- New models added as `discovered`; existing models have `last_seen_at` updated

### UserConnection

A linked external service account. User-scoped (not org-scoped). See [user-connections.md](user-connections.md) for full specification.

See `crates/server/src/storage/models.rs` for the `UserConnectionRow` type.

## Design Decisions

| Question | Decision |
|----------|----------|
| What stores conversation? | **Events** table with `event_type` = `message.*` |
| What are Events for? | Primary data store for messages AND SSE notifications |
| Where are tool calls stored? | In `output.message.completed` events as `ContentPart::ToolCall` |
| Where are tool results stored? | Events with `event_type` = `tool.completed` |
| Session status? | Explicit status field (started, active, idle) |
| Where are capabilities defined? | In-memory registry in API layer |
| How are capabilities applied? | Resolved at API/service layer, merged into RuntimeAgent |
| Where are API keys stored? | Encrypted in database, decrypted at runtime |
| Environment variables for API keys? | `DEFAULT_OPENAI_API_KEY` and `DEFAULT_ANTHROPIC_API_KEY` as fallbacks |
