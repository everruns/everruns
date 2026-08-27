---
type: Specification
title: "Data Models Specification"
description: "Data models (Agent, Session, Message, etc.)."
tags:
  - everruns
  - foundations
---
# Data Models Specification

## Abstract

This document defines the core data models for Everruns - a durable agentic harness engine. For full field definitions, see the Rust source types linked below.

## Requirements

### Agent

Configuration for an agentic loop. An agent can have many concurrent sessions.

See `crates/platform/src/agent.rs` for full field definitions.

Key design points:
- All entity IDs use the dual-ID pattern (internal UUID PK + external public_id). See `knowledge/foundations/id-schema.md`.
- An agent may pin a harness or inherit the organization's current default and platform fallback. Read APIs expose the effective harness and selection provenance so callers do not need to reconstruct runtime resolution from the raw binding.
- `capabilities` field stores enabled capability references (resolved at runtime from registry)
- `default_version_id` selects the immutable AgentVersion used by default deployments when `FEATURE_AGENT_VERSIONS` is enabled. See `knowledge/runtime-resources/agent-versions.md`.
- `status`: `active`, `archived`, or `deleted`
- `archived_at` and `deleted_at` capture lifecycle timestamps

### AgentVersion

Immutable snapshot of an Agent's authored and resolved configuration. AgentVersion is a pilot-specific model, not a generic entity versioning abstraction.

See `crates/platform/src/agent.rs` for full field definitions and `knowledge/runtime-resources/agent-versions.md` for behavior.

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
- `archived` means read-only, not assignable, not executable, hidden from lists by default, visible when explicitly filtered in. The filter label is always "Show archived" (no entity suffix), the page context already communicates what entity type is listed.
- `deleted` is a tombstone state used only to preserve historical references. Normal detail APIs return `404`, normal lists exclude deleted items, and runtime execution must not use them.
- Existing references are preserved by ID. UI/API reference surfaces render tombstones like `<Deleted Agent>` instead of resolving the deleted entity normally.
- If a session or app references an archived or deleted dependency, execution must stop gracefully on the next atom with a user-visible explanation instead of crashing.
- Built-in/system-managed entities do not participate in archive/delete flows.

**Input Validation Limits:**

Last-resort validation limits to guard against abuse. API returns generic `400 Bad Request` with message "Input exceeds allowed limits" when violated. See `crates/server/src/services/` for the current limit constants.

### Session

An instance of agentic loop execution. Sessions are top-level entities under organizations, with an optional host agent assigned to work in each session.

See `crates/core/src/session.rs` for full field definitions.

See `knowledge/operations/localization.md` for locale/timezone resolution and durable preference rules.

Key design points:
- Sessions are direct children of organizations (not agents). The `agent_id` column is a denormalized pointer to the active host agent for existing reads.
- `session_participants` records the users and agents associated with the session. Existing sessions are backfilled with an owner user participant and, when `agent_id` is present, a host agent participant.
- The database enforces at most one active host agent participant per session.
- `agent_version_id` captures the immutable AgentVersion used for runtime execution when agent versions are enabled.
- `app_id` is a nullable internal backreference set only when the server creates a session from an App channel. User/API, MCP, and platform-management session creation paths cannot set it.
- `locale` is an optional session-level BCP 47 tag (for example `uk-UA`). The worker carries it through turn loading and prompt construction so scheduled runs, resumed runs, and subagents can inherit localized behavior.
- `timezone` should be a separate optional session-level IANA timezone for unattended execution defaults. Interactive turns may override it with live browser timezone for that turn.
- `features` field is computed at read time by aggregating `features()` from all active capabilities (not stored). See `knowledge/execution/capabilities.md#capability-features`.
- `capabilities` allows session-level capabilities additive to the agent's. Agent capabilities applied first, then session capabilities.
- Server-managed agent/user scoped memories are represented by `memories.scope`
  (`org`, `agent`, `user`) and are mounted at session creation under reserved
  `/memory/*` paths. See `knowledge/runtime-resources/memory.md`.
- Status transitions: `started` → `active` (processing) → `idle` (waiting for input)
- Sessions work indefinitely, after processing, status returns to `idle`.

### Principal Ownership

Durable ownership is modeled through org-scoped `Principal` records instead of raw user IDs.

See `crates/core/src/principal.rs` for the durable principal type and `crates/server/src/services/principal.rs` for ownership resolution rules.

Key design points:
- `Principal.kind` is currently `user`, `agent_identity`, or `system`.
- Principals form a bounded same-org tree. `user` principals resolve to themselves; non-user principals resolve through `parent_principal_id`.
- First-wave owned entities store `owner_principal_id` plus denormalized `resolved_owner_user_id` for efficient filtering.
- `Session`, `SessionSchedule`, and `App` expose both the direct owner summary and the effective human owner summary in API responses.
- Ownership is separate from execution provenance. Event metadata may record who initiated or acted in a turn without changing the durable owner.
- Reassigning or clearing `agent_identity_id` must preserve the existing effective human owner unless an explicit transfer path says otherwise.

### Message

Conversation data stored as events in the `events` table. Messages are reconstructed from events when loaded.

See `crates/core/src/message.rs` for `Message`, `ContentPart`, `Controls`, and `InputContentPart` types.

Key design points:
- Messages stored as events with types `input.message`, `output.message.completed`. Tool calls embedded in `output.message.completed` via `ContentPart::ToolCall`. Tool results from `tool.completed` events.
- System messages handled internally, not persisted.
- `ContentPart` variants: `text`, `image`, `image_file`, `tool_call`, `tool_result`. Users can only send `text`, `image`, `image_file` via API.
- `metadata.locale` and `metadata.timezone` are reserved for one-turn execution-context overrides. See `knowledge/operations/localization.md`.
- Voice transcripts are stored as normal text content with `metadata.source = "voice"`. Raw audio is not a `ContentPart` in V1.

### Voice Connection

Short-lived provider realtime state attached to a durable Everruns session.

See [voice.md](../operations/voice.md) for full lifecycle, endpoint, and sideband design.

Key design points:
- `VoiceConnection` is a session resource and leased resource, not a top-level conversation.
- The external ID prefix is `voice_conn`.
- V1 supports OpenAI `gpt-realtime-2` through WebRTC.
- Provider API keys stay server-side. Browser clients receive either an SDP answer from the Everruns proxy or a provider client secret minted by the server.
- Provider call IDs and sideband sockets are implementation state. They must not become durable conversation references.
- Transcript text is persisted through `input.message` and `output.message.completed`; raw audio is not stored.

### User Profile

Users should carry durable defaults for:
- `locale`
- `timezone`

These values are fallbacks for session creation and turn resolution. They are not substitutes for live browser timezone on interactive requests.

See `knowledge/operations/localization.md` for precedence rules.

**Controls:**

Optional per-message overrides for model selection, reasoning configuration, speed (service tier), and verbosity.

**Speed (Service Tier):**

When `controls.speed` is set (`flex`, `default`, or `priority`), OpenAI requests carry the matching `service_tier`; see [LLM Drivers spec](llm-drivers.md).

**Verbosity:**

When `controls.verbosity` is set (`low`, `medium`, or `high`), OpenAI requests carry the matching `verbosity`, hinting how expansive the model's answer should be (independent of reasoning effort). Only sent to models whose profile advertises a `verbosity` config (currently the GPT-5.5 and GPT-5.6 series); it is stripped for models that do not support it. See [LLM Drivers spec](llm-drivers.md).

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

1. **Append-Only**: Events are immutable. UPDATE and DELETE blocked via database triggers.
2. **Atomic Per-Session Sequence**: Sequence numbers allocated atomically per session via `event_sequences` table (prevents race conditions).
3. **Event Type Consistency**: `event_type` field must match `data` payload type. Validated at service layer.

**Event Type Naming Convention:** `{entity}.{action}` pattern (e.g., `input.message`, `turn.completed`). See `knowledge/execution/events.md` for the full event type registry and lifecycle details.

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

See `knowledge/execution/capabilities.md` for the full capabilities specification.

Capability configuration and runtime attribution feed reporting facts with
separate usage meanings (`configured`, `resolved`, `exposed`, `invoked`,
`effect_ran`). See [knowledge/evaluation/reporting.md](../evaluation/reporting.md).

### LLM Provider

Configuration for LLM API providers. See `crates/provider/src/provider.rs` for full type definitions.

Key design points:
- `provider_type` stored as plain string without CHECK constraint (forward compatibility)
- Supported types: `openai`, `openrouter`, `azure_openai`, `openai_completions`, `anthropic`, `gemini`, `bedrock`, `llmsim`
- API keys encrypted with AES-256-GCM envelope encryption (see `knowledge/security/encryption.md`)

**API Key Resolution Order:**
1. **Database** (priority): Encrypted in `llm_providers.api_key_encrypted`
2. **Environment Variable** (fallback): provider-specific `DEFAULT_*_API_KEY` (for example `DEFAULT_OPENAI_API_KEY`, `DEFAULT_AZURE_OPENAI_API_KEY`, `DEFAULT_ANTHROPIC_API_KEY`)

Default providers and models seeded on startup. See `crates/server/src/seed.rs` for default model configurations (idempotent, well-known UUIDs).

### LLM Model

Configuration for a specific model within a provider. See `crates/provider/src/model.rs`.

Key design points:
- `source` enum: `manual` (user-added), `discovered` (from provider API), `predefined` (seeded)
- `enabled` flag: only enabled models appear in UI model pickers (Chat UI). All models remain available via API regardless of enabled status. See `crates/server/src/seed.rs` for default enabled models.
- Model/provider assignment is editable so an existing model config can be moved to a different configured provider without deleting and recreating it.
- Organization default model: stored in `organization_settings.default_model_id` (not on the model itself). Auto-elects a new default from enabled models if the current default is disabled or deleted.
- Stale model detection: `last_seen_at < provider.last_synced_at` means model no longer returned by provider API. Stale models kept (not deleted) to preserve customizations.

### Execution Model Specification

`ModelSpec` is the canonical execution-facing model configuration. It contains
an open normalized provider key, the provider-visible model name, and optional
non-secret metadata. It never contains credentials, a base URL, authentication
metadata, or a protocol/vendor enum, so it is safe to serialize, persist, emit,
and print with derived `Debug`.

Persisted model rows resolve to a `ModelSpec`; the exact provider record resolves
independently into a non-serializable runtime `Provider`. There is no public
credential-bearing resolved-model value. Host and control-plane adapters join
the two only while constructing the ready execution driver, after the
serializable kernel input has been fixed.
The high-level `everruns` facade accepts a plain provider-visible model id plus
one runtime provider and constructs the `ModelSpec` internally.

### LLM Model Profile

Read-only metadata describing model capabilities, costs, and limits. Computed at runtime (not stored in database).

**Data Source:** https://github.com/sst/models.dev/tree/dev/providers, cross-referenced with official provider documentation.

**IMPORTANT:** Never guess or extrapolate profile data (pricing, limits, capabilities). Prefer models.dev when a model is listed there; otherwise source directly from the provider's official documentation (e.g. `developers.openai.com/api/docs/models/<id>`, Anthropic model cards). When a profile is added ahead of the models.dev entry, note the provider-doc source in a comment on the profile block and revisit once models.dev catches up. Never guess values.

See `crates/provider/src/model.rs` for `ModelProfile`, `ModelCost`, `ModelLimits`, and `ReasoningEffortConfig` types.

Profiles matched by provider_type + model_id with version normalization (e.g., "gpt-5.2-2025-12-11" → "gpt-5.2").

### Model Discovery

Automatic discovery of available models from provider APIs (OpenAI, OpenRouter via the OpenAI-compatible driver, Anthropic).

- **Background Sync**: Every 24 hours (configurable via `MODEL_SYNC_INTERVAL_HOURS`, 0 to disable)
- **Manual Sync**: `POST /v1/providers/:id/sync-models`
- Only providers with standard base URLs or driver-recognized model-listing URLs synced (for example OpenRouter's OpenAI-compatible endpoint); unsupported custom URLs are skipped
- New models added as `discovered`; existing models have `last_seen_at` updated

### UserConnection

A linked external service account. User-scoped (not org-scoped). See [user-connections.md](../../crates/server/specs/user-connections.md) for full specification.

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
