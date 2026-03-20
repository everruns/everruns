# Client Hints

## Summary

Generic mechanism for clients to declare capabilities and preferences to the server via arbitrary key-value pairs. Distinct from the agent-scoped `capabilities` system — hints are client-scoped signals that influence server behavior without a fixed schema.

## Design

### Storage

- **Session level:** `Session.hints: Option<HashMap<String, Value>>` — defaults for every turn.
- **Message level:** `Controls.hints: Option<HashMap<String, Value>>` — per-message overrides.

### Resolution

Effective hints are resolved per turn via shallow merge:

```
effective_hints = session.hints ∪ last_user_message.controls.hints
```

Per-message hints override session hints key-by-key. See `Controls::resolve_hints()` in `crates/core/src/message.rs`.

### API surface

- `POST /v1/sessions` — `CreateSessionRequest.hints` sets session-level defaults.
- `POST /v1/sessions/{id}/messages` — `CreateMessageRequest.controls.hints` sets per-message overrides.
- `GET /v1/sessions/{id}` — `Session.hints` in response.

### No server-side validation

Any key-value pair is valid. Unknown keys are silently ignored. This ensures third-party clients and future UIs can declare arbitrary hints without requiring server schema changes.

## Known hint keys

| Key | Type | Meaning |
| --- | ---- | ------- |
| `setup_connection` | `bool` | Client can handle inline `setup_connection` tool calls |

These are conventions, not a fixed enum.
