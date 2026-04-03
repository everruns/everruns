# Session Export (JSONL)

Export session messages as a JSONL file for offline analysis, backup, or integration with external tools.

## API

```
GET /v1/sessions/{session_id}/export
```

Returns `application/x-ndjson` with `Content-Disposition: attachment` header.

Each line is a JSON object representing one message (user, agent, or tool_result).
Delta events are excluded — only materialized messages are exported.

### JSONL line schema

```json
{
  "id": "message_...",
  "session_id": "session_...",
  "sequence": 42,
  "role": "user" | "agent",
  "content": [{ "type": "text", "text": "..." }, ...],
  "created_at": "2024-01-15T10:30:00.000Z"
}
```

Fields `controls`, `metadata`, `external_actor` are included when present.

### Response headers

- `Content-Type: application/x-ndjson`
- `Content-Disposition: attachment; filename="session_{id}.jsonl"`

### Errors

- 400: Invalid session ID format
- 404: Session not found
- 500: Internal error

### Auth & Policy

Same as `GET /v1/sessions/{session_id}` — requires `SESSION_VIEW`.

## UI

Download button on the `SessionCard` component (sessions list page). Icon: `Download` from lucide-react.
Clicking triggers a browser download of the JSONL file via `fetch` + blob URL.
