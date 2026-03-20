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

## `setup_connection` gating (EVE-162)

When a tool returns `ConnectionRequired`, the worker checks the session's `setup_connection` hint:

- **Hint `true`:** Worker emits synthetic `setup_connection` tool calls and sets session to `waiting_for_tool_results`. The UI renders an inline connection card.
- **Hint absent/`false`:** Worker skips synthetic tool calls and lets the workflow continue. The tool result already contains `{"connection_required": "<provider>"}` with `success: false`, so the LLM can inform the user that a connection is needed.

The Chat UI auto-declares `setup_connection: true` in `useCreateSession()` so all UI-created sessions get the interactive flow. API-only clients that don't handle synthetic tool calls simply omit the hint and get the fallback behavior.
