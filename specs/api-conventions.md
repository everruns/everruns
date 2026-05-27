# API Conventions

This document captures cross-cutting conventions the Everruns HTTP API
follows so an LLM toolcaller can consume it without parsing prose. The
broader API design intent lives in [`apis.md`](apis.md); this file is the
durable contract for **hypermedia, link relations, and `allowed_actions`**.

## Hypermedia entity actions

Every entity response wraps its body in `WithUrls<T>` and may carry an
`allowed_actions: Vec<AllowedAction>` field. The field is omitted from
the wire shape when no actions apply. Action membership is computed
server-side from the current entity state — a `completed` session does
not get a `cancel` action, an `idle` session does.

```json
{
  "self_url": "https://api.example/v1/sessions/session_…",
  "view_url": "https://app.example/sessions/session_…/chat",
  "ui_link":  "https://app.example/sessions/session_…/chat",
  "allowed_actions": [
    {
      "rel": "cancel",
      "method": "POST",
      "operation_id": "cancel_turn",
      "href": "https://api.example/v1/sessions/session_…/cancel",
      "hint": "Cancel the active turn for this session."
    },
    {
      "rel": "events",
      "method": "GET",
      "operation_id": "get_session_events",
      "href": "https://api.example/v1/sessions/session_…/events",
      "hint": "Stream session events (SSE)."
    }
  ],
  "id": "session_…",
  "status": "active",
  …
}
```

This collapses dozens of "to cancel a session, POST to
`/v1/sessions/{id}/cancel`" prose docs into a single structured field
the agent can iterate over.

## `AllowedAction` shape

`AllowedAction` is reused across two surfaces — entity hypermedia and
error recovery — because the shape is identical; only the `rel`
vocabulary differs by surface.

| Field          | Type             | When set                                                                                              |
| -------------- | ---------------- | ----------------------------------------------------------------------------------------------------- |
| `rel`          | `string`         | Always. Closed vocabulary (see below).                                                                |
| `href`         | `string`         | Always on entity actions. Optional on error actions where the same operation is retried with its original method. |
| `method`       | `string`         | Always on entity actions (HTTP method). Usually omitted on error actions.                             |
| `operation_id` | `string?`        | OpenAPI `operationId` the caller should invoke.                                                       |
| `hint`         | `string?`        | Short, agent-readable note (e.g. "Shorten 'name' to <= 200 chars.").                                  |
| `schema_ref`   | `string?`        | OpenAPI `$ref` to the request-body schema, when the action takes one.                                 |

## Closed `rel` vocabulary

### Entity hypermedia rels

| rel       | Meaning                                                            | Method  |
| --------- | ------------------------------------------------------------------ | ------- |
| `self`    | Fetch the latest representation of this resource.                  | GET     |
| `update`  | Edit metadata (name, tags, etc.). Carries `schema_ref`.            | PATCH   |
| `delete`  | Delete this resource (idempotent).                                 | DELETE  |
| `cancel`  | Cancel an in-flight operation (e.g. a session turn).               | POST    |
| `pause`   | Pause a resource that supports lifecycle pause (schedules, …).     | POST    |
| `resume`  | Resume a paused resource.                                          | POST    |
| `pin`     | Pin this resource (toggle on).                                     | PUT     |
| `unpin`   | Unpin this resource (toggle off).                                  | DELETE  |
| `events`  | Subscribe to the entity's event stream (SSE).                      | GET     |

### Error-recovery rels

| rel             | Meaning                                                                |
| --------------- | ---------------------------------------------------------------------- |
| `retry`         | Replay the same request once the caller has fixed the input.          |
| `retry-later`   | Replay after `retry_after_seconds`. Used for 429/transient 503.        |
| `get-existing`  | Fetch the resource the caller tried to create when conflict was duplicate. |
| `unarchive`     | Reverse an archive before retrying.                                    |

Adding a new `rel` is a spec change. Reuse existing rels before
inventing new ones. Per-resource implementations live alongside the
resource (see e.g. `session_allowed_actions` in
`crates/server/src/api/common.rs`).

## State-aware membership

`allowed_actions` is recomputed per response from the current entity
state. The mapping rule lives in **one place** per resource — the
`ResourceUrlable::allowed_actions` impl — so that handlers can never
disagree about what's offered.

**Sessions** (pilot for this convention):

| Session status               | Includes `cancel` |
| ---------------------------- | ----------------- |
| `started`                    | no                |
| `active`                     | **yes**           |
| `idle`                       | no                |
| `waiting_for_tool_results`   | **yes**           |
| `paused`                     | no                |

`self`, `events`, `update`, `delete`, and `pin`/`unpin` (chosen by
`is_pinned`) are unconditional.

## Out of scope (tracked separately)

* Rolling the convention out to Agents, Harnesses, Volumes, Schedules,
  Skills, Apps, etc. (follow-on issue).
* A ratchet gate for "% of entity responses carrying allowed_actions" —
  designable after a few clusters have opted in so the floor isn't
  premature.
