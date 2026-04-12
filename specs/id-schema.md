# ID Schema Specification

## Abstract

This document defines the standardized identifier schema for all entity types in Everruns. All external-facing identifiers use **Stripe-style prefixed IDs with UUIDv7** for type safety, debuggability, and consistency.

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
- `{32-hex-chars}` - UUIDv7 in simple format (lowercase hex, no dashes)

**Example:** `agent_01933b5a00007000800000000000001`

### Entity Prefixes

For the full list of entity prefixes and type aliases, see `crates/core/src/typed_id.rs`.

**Example:** `agent_01933b5a00007000800000000000002`

### ID Generation

- IDs are generated using UUIDv7 for time-ordering benefits
- The UUID is formatted as lowercase hex without dashes (32 chars)
- The prefix is prepended with an underscore separator

```rust
// Example generation
let uuid = Uuid::now_v7();
let id = format!("agent_{}", uuid.simple()); // agent_0193...
```

### ID Validation

IDs must match `^{prefix}_[0-9a-f]{32}$`. See `TypedId` validation in `crates/core/src/typed_id.rs`.

### Database Storage — Dual-ID Pattern

All entities use a **dual-ID pattern** with an internal UUID primary key and an external public_id:

| Layer | Column | Type | Purpose |
|-------|--------|------|---------|
| Internal | `id` | `UUID` (PK) | FK references, joins, internal queries. Never exposed in API. |
| External | `public_id` | `TEXT` (UNIQUE per org) | API-facing identifier. Client-supplied or auto-generated. Format: `{prefix}_{32-hex}`. |

**Rules:**
- API always shows `public_id` as `"id"` — internal UUID is never exposed
- Client can supply `public_id` on create; server auto-generates `{prefix}_{uuidv7_hex}` if omitted
- `UNIQUE(org_id, public_id)` — same public_id allowed across orgs
- Format validated: `^{prefix}_[0-9a-f]{32}$`
- FKs between tables use internal `UUID` columns
- When auto-generating: derive `public_id` from the internal UUID for consistency (`{prefix}_{uuid_hex}`)

**Domain struct convention:** See `crates/core/src/agent.rs` for the canonical example of the dual-ID pattern with `public_id` / `internal_id` fields.

**Upsert semantics (PUT):**
- `PUT /v1/{entity}/{public_id}` creates if not exists (201), updates if exists (200)
- Enabled by the `UNIQUE(org_id, public_id)` constraint + `ON CONFLICT DO UPDATE`

**Session FK resolution:**
- `sessions.agent_id` stores the agent's internal UUID (FK to `agents.id`)
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

For the full list of well-known IDs and range allocations, see `crates/core/src/typed_id.rs` and `crates/server/src/seed.rs`.

## Design Decisions

| Question | Decision |
|----------|----------|
| Why prefixed IDs? | Type safety, debuggability, prevents mixing ID types |
| Why UUIDv7? | Time-ordering benefits, sortable, globally unique |
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

`TypedId<T>` provides type-safe ID handling with serde, sqlx, Display/Debug, and validation. See `crates/core/src/typed_id.rs` for the full implementation, type aliases, and usage patterns.

**IMPORTANT: Typed IDs are mandatory throughout the codebase.** All code MUST use typed IDs (e.g., `SessionId`, `AgentId`) instead of raw `Uuid` for entity identifiers.
