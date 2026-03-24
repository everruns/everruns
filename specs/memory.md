# Memory Capability

Cross-session persistent memory that agents can write to and read from.
Memories survive beyond individual sessions and are scoped to **memory stores**
within an organization.

## Concepts

- **Memory Store** — Named, org-scoped container for memories. An org can have
  many stores (e.g. "team-knowledge", "ops-runbooks"). Agents select which
  store(s) to use via capability config. A default org-wide store is created
  automatically.
- **Memory** — A single unit of knowledge: short text content plus optional
  rich content parts (images, references). Tagged, importance-scored,
  deduplicated on write.
- **MemoryContentPart** — Reuses the same discriminated-union shape as message
  `ContentPart` (text + image variants). Recall returns multicontent so the LLM
  sees both text and images inline.

## Capability Config

```json
{
  "ref": "memory",
  "config": {
    "store": "memstore_abc123",
    "passive_recall_count": 5
  }
}
```

| Field | Type | Default | Description |
|-------|------|---------|-------------|
| `store` | `MemoryStoreId?` | org default store | Which store to read/write |
| `passive_recall_count` | `usize` | `5` | Memories auto-injected per turn (0 = disable) |

When `store` is omitted, the agent uses the org's default memory store.

## Tools

### `remember`

Create or update a memory.

```json
{
  "type": "object",
  "properties": {
    "content": { "type": "string", "maxLength": 2000, "description": "1-3 sentence knowledge to persist" },
    "kind": { "type": "string", "enum": ["fact", "preference", "correction", "procedure", "context"], "default": "fact" },
    "importance": { "type": "integer", "minimum": 1, "maximum": 10, "default": 5 },
    "tags": { "type": "array", "items": { "type": "string" }, "maxItems": 10 }
  },
  "required": ["content"],
  "additionalProperties": false
}
```

Returns `{ "memory_id": "mem_xxx", "created": true }` or
`{ "memory_id": "mem_xxx", "created": false, "note": "merged with existing similar memory" }`.

### `recall`

Search memories by keyword, tag, or kind. Returns multicontent parts.

```json
{
  "type": "object",
  "properties": {
    "query": { "type": "string", "description": "Keyword search across memory content" },
    "tags": { "type": "array", "items": { "type": "string" }, "description": "Filter by tags (AND)" },
    "kind": { "type": "string", "enum": ["fact", "preference", "correction", "procedure", "context"] },
    "limit": { "type": "integer", "minimum": 1, "maximum": 50, "default": 10 }
  },
  "additionalProperties": false
}
```

Response uses `MemoryContentPart` array per memory (same shape as message `ContentPart`):

```json
{
  "memories": [
    {
      "id": "mem_xxx",
      "kind": "correction",
      "importance": 8,
      "created_at": "2026-03-24T12:00:00Z",
      "content_parts": [
        { "type": "text", "text": "Don't use unwrap() in production code..." },
        { "type": "image", "url": "data:image/png;base64,...", "media_type": "image/png" }
      ],
      "tags": ["rust", "error-handling"]
    }
  ],
  "total": 42
}
```

### `forget`

Deactivate a memory by ID (soft-delete).

```json
{
  "type": "object",
  "properties": {
    "memory_id": { "type": "string", "description": "Memory ID to forget" }
  },
  "required": ["memory_id"],
  "additionalProperties": false
}
```

## Data Model

See `crates/core/src/memory_store.rs` for full type definitions.

Core types:
- `MemoryStoreId` — prefixed typed ID (`mst_...`)
- `MemoryId` — prefixed typed ID (`mem_...`)
- `MemoryStore` — org-scoped named store with capacity config
- `Memory` — content, kind, importance, tags, active flag, store FK
- `MemoryContentPart` — text or image (same shape as `ContentPart`, minus tool variants)

## Capacity Limits

| Limit | Default | Scope | Notes |
|-------|---------|-------|-------|
| Active memories per store | 10,000 | Per-store | Oldest low-importance auto-archived beyond this |
| Stores per org | 50 | Per-org | |
| Memory content length | 2,000 chars | Per-memory | Hard cap |
| Tags per memory | 10 | Per-memory | |
| Image attachments per memory | 4 | Per-memory | Inline base64 in content_parts |
| Max single image | 5 MB | Per-image | |
| Total image data per memory | 10 MB | Per-memory | Sum of all image parts |

Enforcement: checked on write (`remember` tool + REST API). Returns clear
`ToolError` when exceeded.

## Storage

In-memory implementation for dev mode (`InMemoryMemoryStore`).
PostgreSQL implementation for production (future migration `012_memory.sql`).

## System Prompt

When memory capability is enabled, a section is injected:

```
## Persistent Memory

You have persistent memory across sessions via `remember` and `recall` tools.
Use `remember` to save important facts, user preferences, corrections, or
procedures. Use `recall` to search your memory before answering questions where
prior context may help. Use `forget` to remove outdated or incorrect memories.
```
