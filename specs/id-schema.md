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
| Turn | `turn_` | `turn_01933b5a00007000800000000000006` |
| Execution | `exec_` | `exec_01933b5a00007000800000000000007` |
| LLM Provider | `provider_` | `provider_01933b5a00007000800000000000008` |
| LLM Model | `model_` | `model_01933b5a00007000800000000000009` |
| Image | `img_` | `img_01933b5a0000700080000000000000a` |
| MCP Server | `mcp_` | `mcp_01933b5a0000700080000000000000b` |

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

**Domain struct convention:**

```rust
pub struct Agent {
    #[serde(rename = "id")]
    pub public_id: AgentId,       // External, shown in API as "id"

    #[serde(skip, default = "Uuid::nil")]
    pub internal_id: Uuid,        // Internal, never serialized

    // ... other fields
}
```

**Migration for existing tables:**

```sql
ALTER TABLE {table} ADD COLUMN public_id TEXT;
UPDATE {table} SET public_id = '{prefix}_' || replace(id::text, '-', '');
ALTER TABLE {table} ALTER COLUMN public_id SET NOT NULL;
CREATE UNIQUE INDEX idx_{table}_org_public_id ON {table}(org_id, public_id);
ALTER TABLE {table} ADD CONSTRAINT {table}_public_id_format
    CHECK (public_id ~ '^{prefix}_[0-9a-f]{32}$');
```

**Implementation status:**
- Organizations: uses `public_id` pattern (original adopter)
- Agents: uses `public_id` pattern (implemented)
- Sessions, Messages, etc.: follow in subsequent PRs

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

The `TypedId<T>` generic type provides:
- Type-safe ID handling (prevents mixing agent IDs with session IDs)
- Automatic serialization/deserialization
- Validation on construction
- Display and Debug implementations
- Database compatibility via sqlx `Type`/`Encode`/`Decode` implementations

```rust
// Type aliases for each entity
pub type AgentId = TypedId<AgentIdMarker>;
pub type SessionId = TypedId<SessionIdMarker>;
pub type MessageId = TypedId<MessageIdMarker>;
pub type EventId = TypedId<EventIdMarker>;
pub type TurnId = TypedId<TurnIdMarker>;
pub type ExecId = TypedId<ExecIdMarker>;
pub type ProviderId = TypedId<ProviderIdMarker>;
pub type ModelId = TypedId<ModelIdMarker>;
pub type ImageId = TypedId<ImageIdMarker>;
pub type McpServerId = TypedId<McpServerIdMarker>;
```

### Usage Requirements

**IMPORTANT: Typed IDs are mandatory throughout the codebase.**

All code MUST use typed IDs instead of raw `Uuid` for entity identifiers:

```rust
// ✅ Correct: Use typed IDs
fn get_session(session_id: SessionId) -> Option<Session>;
fn create_event(session_id: SessionId, agent_id: AgentId) -> Event;

// ❌ Wrong: Raw UUIDs lose type safety
fn get_session(session_id: Uuid) -> Option<Session>;
fn create_event(session_id: Uuid, agent_id: Uuid) -> Event;
```

### Common Patterns

```rust
// Create new ID (uses UUIDv7 internally)
let session_id = SessionId::new();

// Convert from existing UUID
let session_id = SessionId::from_uuid(uuid);
let session_id: SessionId = uuid.into();

// Extract underlying UUID (for database queries, external APIs)
let uuid = session_id.uuid();
let uuid: Uuid = session_id.into();

// String formatting (produces prefixed format)
let s = session_id.to_string(); // "session_01933b5a..."

// Parse from string
let session_id: SessionId = "session_01933b5a...".parse()?;
```

### Trait Implementations

All core traits use typed IDs:

```rust
// MessageRetriever uses SessionId and MessageId
async fn get(&self, session_id: SessionId, message_id: MessageId) -> Result<Option<Message>>;
async fn load(&self, session_id: SessionId) -> Result<Vec<Message>>;

// AgentStore uses AgentId
async fn get_agent(&self, agent_id: AgentId) -> Result<Option<Agent>>;

// SessionStore uses SessionId
async fn get_session(&self, session_id: SessionId) -> Result<Option<Session>>;

// LlmProviderStore uses ModelId
async fn get_model_with_provider(&self, model_id: ModelId) -> Result<Option<ModelWithProvider>>;
```

### Database Integration

TypedId implements sqlx traits for direct database usage:

```rust
// Direct use in queries - stored as UUID in database
let row = sqlx::query_as!(
    AgentRow,
    "SELECT * FROM agents WHERE id = $1",
    agent_id  // AgentId works directly
)
.fetch_one(&pool)
.await?;
```
