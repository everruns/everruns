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

| Entity | Prefix | Example |
|--------|--------|---------|
| Organization | `org_` | `org_01933b5a00007000800000000000001` |
| Agent | `agent_` | `agent_01933b5a00007000800000000000002` |
| Session | `session_` | `session_01933b5a00007000800000000000003` |
| Message | `message_` | `message_01933b5a00007000800000000000004` |
| Event | `event_` | `event_01933b5a00007000800000000000005` |
| LLM Provider | `provider_` | `provider_01933b5a00007000800000000000006` |
| LLM Model | `model_` | `model_01933b5a00007000800000000000007` |
| Image | `img_` | `img_01933b5a00007000800000000000008` |
| MCP Server | `mcp_` | `mcp_01933b5a00007000800000000000009` |

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

IDs must pass validation:
1. Must start with the correct prefix for the entity type
2. Must have exactly 32 hex characters after the prefix
3. Hex characters must be lowercase
4. No non-hex characters allowed in the suffix

```rust
// Validation pattern (example for agent_id)
// ^agent_[0-9a-f]{32}$
```

### Database Storage

IDs are stored as:
- **Primary Key Column:** `TEXT` (the full prefixed string)
- **Index:** B-tree index on ID columns for efficient lookups

This approach:
- Preserves type information in the database
- Makes debugging easier (can identify entity type from ID)
- Maintains compatibility with external systems expecting string IDs

### API Serialization

IDs are serialized as strings in JSON:

```json
{
  "id": "agent_01933b5a00007000800000000000001",
  "session_id": "session_01933b5a00007000800000000000003"
}
```

### Well-Known IDs

Certain entities have well-known IDs for seeding and testing:

**Default Organization:**
```
org_00000000000000000000000000000001
```

**Seeded Providers:**
| Provider | ID |
|----------|-----|
| OpenAI | `provider_01933b5a00007000800000000001` |
| Anthropic | `provider_01933b5a00007000800000000002` |

**Seeded Models (range allocation):**
- `0x001-0x0FF`: LLM Providers
- `0x100-0x1FF`: Seed Agents
- `0x200-0x2FF`: OpenAI Models
- `0x300-0x3FF`: Anthropic Models

## Design Decisions

| Question | Decision |
|----------|----------|
| Why prefixed IDs? | Type safety, debuggability, prevents mixing ID types |
| Why UUIDv7? | Time-ordering benefits, sortable, globally unique |
| Why TEXT storage? | Preserves type info, debuggable, no conversion needed |
| Why lowercase hex? | Consistency, case-insensitive matching, URL-safe |

## Migration Strategy

Existing UUIDs are migrated to prefixed format:
1. Add new TEXT column with prefixed IDs
2. Migrate data: `{prefix}_{uuid_without_dashes}`
3. Drop old UUID column
4. Rename new column to original name

## Type Implementation

The `TypedId<T>` generic type provides:
- Type-safe ID handling (prevents mixing agent IDs with session IDs)
- Automatic serialization/deserialization
- Validation on construction
- Display and Debug implementations

```rust
// Type aliases for each entity
pub type AgentId = TypedId<AgentIdMarker>;
pub type SessionId = TypedId<SessionIdMarker>;
pub type MessageId = TypedId<MessageIdMarker>;
// etc.
```
