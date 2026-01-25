# Events Contract Specification

This document defines the stability guarantees and compatibility rules for the Everruns event protocol.

## Overview

Events are a **public API contract**. Consumers can rely on the guarantees defined here when building integrations. The server is responsible for ensuring only well-defined events reach consumers.

## Compatibility Guarantees

### Non-Breaking Changes

These changes are safe and may happen in any release:

- **Adding new event types** - New event types may be added. Server filters unknown types internally.
- **Adding optional fields** - New optional fields may be added to existing event data types.
- **Adding enum values** - New values may be added to string enums (e.g., new status values).
- **Relaxing validation** - Required fields may become optional.

### Breaking Changes

These changes require a major version bump:

- **Removing event types** - Existing event types will not be removed without major version.
- **Removing fields** - Existing fields will not be removed without major version.
- **Renaming fields** - Field names are stable.
- **Changing field types** - Field types are stable (e.g., string stays string).
- **Making optional fields required** - Optional fields will not become required.
- **Changing field semantics** - The meaning of fields is stable.

## Server Responsibilities

The server ensures consumers receive only well-defined events:

1. **Filter unsupported events** - Events with unknown types are filtered before API responses.
2. **Log warnings** - Unknown event types trigger warning logs for debugging.
3. **Emit only defined types** - API responses contain only documented event types.
4. **Validate on emission** - Events are validated before storage and transmission.

## Consumer Guidelines

Consumers should follow these practices for robust integrations:

1. **No unknown type handling needed** - All events in API responses have documented types.
2. **Ignore unknown fields** - Deserialize with `#[serde(deny_unknown_fields)]` disabled.
3. **Handle optional fields** - Check for presence before accessing optional fields.
4. **Don't rely on field ordering** - JSON field order is not guaranteed.

## Event Structure Stability

The core event structure is frozen:

```json
{
  "id": "event_...",           // Stable: UUIDv7 with event_ prefix
  "type": "turn.completed",    // Stable: dot-notation event type
  "ts": "2024-01-15T10:30:00.000Z",  // Stable: ISO 8601 timestamp
  "session_id": "session_...", // Stable: session reference
  "sequence": 42,              // Stable: monotonic within session
  "context": { ... },          // Stable: correlation context
  "data": { ... }              // Type-specific: follows per-type schema
}
```

### EventContext Stability

The context object fields are stable:

| Field | Status | Description |
|-------|--------|-------------|
| `turn_id` | Stable | Turn identifier |
| `input_message_id` | Stable | Triggering message |
| `exec_id` | Stable | Atom execution ID |
| `trace_id` | Stable | OTel trace ID |
| `span_id` | Stable | OTel span ID |
| `parent_span_id` | Stable | Parent span for hierarchy |

New optional fields may be added to EventContext.

## Versioning

Events follow semantic versioning aligned with the API version:

- **Patch** (0.0.x): Bug fixes, no schema changes
- **Minor** (0.x.0): New event types, new optional fields
- **Major** (x.0.0): Breaking changes (removals, type changes)

## Testing

Contract tests validate these guarantees:

1. **Snapshot tests** - JSON structure for each event type
2. **Round-trip tests** - Serialize/deserialize equality
3. **Forward compatibility** - Unknown fields are ignored
4. **API filtering** - Unsupported events never reach consumers
