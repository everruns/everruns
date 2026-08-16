---
type: Specification
title: "ID Schema Specification"
description: "Standardized prefixed ID format."
tags:
  - everruns
  - foundations
---
# ID Schema Specification

## Abstract

This document defines the standardized identifier schema for all entity types in Everruns. All external-facing identifiers use **Stripe-style prefixed IDs backed by a UUID** (UUIDv7 for DB-backed keys, UUIDv4 for random-public ids, see [ID classes](#id-classes-time-ordered-vs-random-public)) for type safety, debuggability, and consistency.

This pattern was popularized by Stripe (`cus_`, `sub_`, `pi_`) and formalized by the [TypeID spec](https://github.com/jetpack-io/typeid). Our implementation uses hex encoding (32 chars) rather than TypeID's base32 (26 chars) for simpler debugging and UUID compatibility.

## Requirements

### ID Format

All entity identifiers follow a consistent prefixed format:

```
{prefix}_{32-hex-chars}
```

Where:
- `{prefix}` - A descriptive lowercase string identifying the entity type
- `_` - Underscore separator
- `{32-hex-chars}` - a UUID in simple format (lowercase hex, no dashes); UUIDv7 or UUIDv4 depending on the [id class](#id-classes-time-ordered-vs-random-public)

**Example:** `agent_01933b5a00007000800000000000001`

### Entity Prefixes

For the full list of entity prefixes and type aliases, see `crates/provider/src/typed_id.rs`.

**Example:** `agent_01933b5a00007000800000000000002`

Agent version IDs use the `agentver_` prefix.

### ID Generation

- The UUID is formatted as lowercase hex without dashes (32 chars); the prefix is prepended with an underscore separator
- The UUID *version* depends on the id class (see below). The wire format (`{prefix}_{32-hex}`) and validation regex are identical for every class, so version is an internal generation detail, never a parse or storage concern

```rust
// Example generation (time-ordered, default)
let uuid = Uuid::now_v7();
let id = format!("agent_{}", uuid.simple()); // agent_0193...
```

#### ID classes: time-ordered vs random-public

`TypedId::new()` dispatches on the id class via `IdMarker::generate_uuid()`:

| Class | UUID version | Rationale |
|-------|--------------|-----------|
| **Time-ordered** (default), DB-backed keys: `agent`, `session`, `event`, `turn`, `org`, and all other entities with a table/PK/index | UUIDv7 | Time-ordering gives B-tree insert locality and lets the id double as a sort key. |
| **Random-public**: `message` | UUIDv4 (`TypedId::new_random()`) | Messages are **not** DB entities: they live embedded in `events.data` JSONB with no table, FK, index, or sort dependency on the id, and the id is the *public* identifier serialized to clients (`output.message.completed`, `EventContext.input_message_id`). UUIDv7's time-ordering does no work here and would leak a creation timestamp into a client-visible id, so message ids are random. Same wire format → no migration; legacy UUIDv7 message ids keep parsing. |

`TurnId` stays UUIDv7: turn ordering is used by durable execution, and the raw turn UUID's current public exposure via AG-UI is being removed by the streaming `message_id` work rather than by re-randomizing the turn id. To make a new id class random, override `generate_uuid()` on its marker in `crates/provider/src/typed_id.rs`.

### ID Validation

IDs must match `^{prefix}_[0-9a-f]{32}$`. See `TypedId` validation in `crates/provider/src/typed_id.rs`.

### Database Storage, Dual-ID Pattern

All entities use a **dual-ID pattern** with an internal UUID primary key and an external public_id:

| Layer | Column | Type | Purpose |
|-------|--------|------|---------|
| Internal | `id` | `UUID` (PK) | FK references, joins, internal queries. Never exposed in API. |
| External | `public_id` | `TEXT` (UNIQUE per org) | API-facing identifier. Client-supplied or auto-generated. Format: `{prefix}_{32-hex}`. |

**Rules:**
- API always shows `public_id` as `"id"`, internal UUID is never exposed
- Client can supply `public_id` on create; server auto-generates `{prefix}_{uuidv7_hex}` if omitted
- `UNIQUE(org_id, public_id)`, same public_id allowed across orgs
- Format validated: `^{prefix}_[0-9a-f]{32}$`
- FKs between tables use internal `UUID` columns
- When auto-generating: derive `public_id` from the internal UUID for consistency (`{prefix}_{uuid_hex}`)

**Domain struct convention:** See `crates/platform/src/agent.rs` for the canonical example of the dual-ID pattern with `public_id` / `internal_id` fields.

**Upsert semantics (PUT):**
- `PUT /v1/{entity}/{public_id}` creates if not exists (201), updates if exists (200)
- Enabled by the `UNIQUE(org_id, public_id)` constraint + `ON CONFLICT DO UPDATE`

**Session FK resolution:**
- `sessions.agent_id` stores the agent's internal UUID (FK to `agents.id`)
- `sessions.agent_version_id` stores the agent version internal UUID when a session is bound to an immutable agent snapshot
- API responses resolve this to the agent's `public_id` via a lookup/JOIN
- Session creation accepts the agent's `public_id`, resolves to internal UUID for storage

### API Serialization

IDs are serialized as strings in JSON:

```json
{
  "id": "agent_01933b5a00007000800000000000001",
  "session_id": "session_01933b5a00007000800000000000003"
}
```

### Well-Known IDs

For the full list of well-known IDs and range allocations, see `crates/provider/src/typed_id.rs` and `crates/server/src/seed.rs`.

## Design Decisions

| Question | Decision |
|----------|----------|
| Why prefixed IDs? | Type safety, debuggability, prevents mixing ID types |
| Why UUIDv7 for DB-backed ids? | Time-ordering gives B-tree insert locality + sortability; globally unique |
| Why UUIDv4 for message ids? | Messages are not DB entities (embedded in `events.data` JSONB, no index/sort on the id) and the id is client-visible; v7 would leak a creation timestamp with no benefit, see the ID-class table |
| Why dual-ID? | Internal UUID PK for FK integrity; external public_id for client-facing API, client-supplied IDs, upsert |
| Why not expose UUID? | Decouples internal PK from API contract; allows client-supplied IDs |
| Why lowercase hex? | Consistency, case-insensitive matching, URL-safe |

## Migration Strategy

Existing UUIDs are migrated to prefixed format:
1. Add new TEXT column with prefixed IDs
2. Migrate data: `{prefix}_{uuid_without_dashes}`
3. Drop old UUID column
4. Rename new column to original name

## Type Implementation

`TypedId<T>` provides type-safe ID handling with serde, sqlx, Display/Debug, and validation. See `crates/provider/src/typed_id.rs` for the full implementation, type aliases, and usage patterns.

**IMPORTANT: Typed IDs are mandatory throughout the codebase.** All code MUST use typed IDs (e.g., `SessionId`, `AgentId`) instead of raw `Uuid` for entity identifiers.
