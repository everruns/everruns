---
type: Specification
title: "Session Export (JSONL)"
description: "Session export to JSONL."
tags:
  - everruns
  - runtime-resources
---
# Session Export (JSONL)

Export session messages as a JSONL file for offline analysis, backup, or integration with external tools.

## API

```
GET /v1/sessions/{session_id}/export
```

Returns `application/x-ndjson` with `Content-Disposition: attachment` header.

Each line is a JSON object representing one message (user or agent).
Tool results are embedded as `tool_result` content parts within agent messages.
Delta events are excluded, only materialized messages are exported.

### `?format=atif`

`GET /v1/sessions/{session_id}/export?format=atif` returns a single ATIF-v1.7
trajectory JSON document instead (Content-Type `application/json`, filename
`{session_id}.atif.json`), folded from the session's event log with always-on
secret scrubbing. See `knowledge/evaluation/atif-adoption.md`. The default (`jsonl`) behavior
is unchanged.

Documents over the server size cap are rejected with HTTP 413. Add
`&segmented=true` to get the recoverable path: a forward-linked chain of
byte-bounded segments (each a standalone ATIF-v1.7 document) instead of one
document. Follow each segment's root `continued_trajectory_ref` URL, which
carries an opaque `cursor` query param, until a segment omits it (the final
one). A malformed or foreign cursor is rejected with HTTP 400. Segment
bookkeeping (`X-Atif-Segment-Index`, `X-Atif-Next-Cursor`, per-segment
`X-Atif-Images-Omitted`) is mirrored in headers. See the segmented-export
contract in `knowledge/evaluation/atif-adoption.md`.

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
- `Content-Disposition: attachment; filename="{session_id}.jsonl"`

### Errors

- 400: Invalid session ID format
- 404: Session not found
- 500: Internal error

### Auth & Policy

Same as `GET /v1/sessions/{session_id}`, requires `SESSION_VIEW`.

## UI

Download menu on the `SessionCard` component (sessions list page). Icon: `Download` from lucide-react.
The menu offers "Export JSONL" (default, `{session_id}.jsonl`) and "Export ATIF"
(`{session_id}.atif.json`); both download via `fetch` + blob URL.

ATIF limit alerts surface on the shared toast surface (notifications provider):
HTTP 413 (document over the server size cap) shows an error toast with the
server message when parseable, and a non-zero `X-Atif-Images-Omitted` response
header still downloads but shows an informational toast. Both signals are
optional, against servers without them the flow behaves like JSONL export.
