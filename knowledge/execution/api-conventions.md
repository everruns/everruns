---
type: Specification
title: "API Conventions"
description: "Cross-cutting HTTP API conventions."
tags:
  - everruns
  - execution
---
# API Conventions

This document captures cross-cutting conventions the Everruns HTTP API
follows so an LLM toolcaller can consume it without parsing prose. The
broader API design intent lives in [`apis.md`](apis.md); this file is the
durable contract for **hypermedia, link relations, and `allowed_actions`**.

## Hypermedia entity actions

Top-level resource responses (those wrapped via `UrlBuilder::wrap` in
`WithUrls<T>`) may carry an `allowed_actions: Vec<AllowedAction>` field.
The field is omitted from the wire shape when no actions apply. Action
membership is computed server-side from the current entity state, a
`completed` session does not get a `cancel` action, an `idle` session
does.

Nested or composite payloads that don't go through `WithUrls` (e.g.
filesystem `SessionFile` entries) currently rely on the JSON link
decoration middleware in `decorate_value_links` for `self_url` /
`view_url` and don't carry `allowed_actions`. Opting them into the
convention is a per-cluster follow-on.

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
      "operation_id": "list_events",
      "href": "https://api.example/v1/sessions/session_…/events",
      "hint": "List session events (JSON polling; supports type filters and pagination)."
    },
    {
      "rel": "stream",
      "method": "GET",
      "operation_id": "stream_sse",
      "href": "https://api.example/v1/sessions/session_…/sse",
      "hint": "Subscribe to session events live over Server-Sent Events."
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

`AllowedAction` is reused across two surfaces, entity hypermedia and
error recovery, because the shape is identical; only the `rel`
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
| `events`  | Read the entity's events (JSON polling, supports filters/pagination). | GET  |
| `stream`  | Subscribe to the entity's event stream live over Server-Sent Events. | GET   |

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
state. The mapping rule lives in **one place** per resource, the
`ResourceUrlable::allowed_actions` impl, so that handlers can never
disagree about what's offered.

**Sessions** (pilot for this convention):

| Session status               | Includes `cancel` |
| ---------------------------- | ----------------- |
| `started`                    | no                |
| `active`                     | **yes**           |
| `idle`                       | no                |
| `waiting_for_tool_results`   | no                |
| `paused`                     | no                |

`self`, `events`, `update`, `delete`, and `pin`/`unpin` (chosen by
`is_pinned`) are unconditional.

**Agents**: `self`, `update`, `versions`, `delete` unconditional;
`copy` only on `active` (archived/deleted agents can't be forked).

**Harnesses**: `self`, `update`, `delete` unconditional; `copy` only
on `active`.

**Apps**: `self`, `update`, `runs`, `delete` unconditional. The
`draft` ↔ `published` lifecycle pair gates `publish` (on `draft`)
vs `unpublish` (on `published`); `archived`/`deleted` apps expose
neither.

**Skills**: `self`, `update`, `content` unconditional; `delete` only
on `active`/`disabled` (not on already-archived/deleted skills, which
are already terminal-state tombstones).

Each resolver is a pure function over `(id, status, …, api_base)`,
unit-tested alongside `session_allowed_actions` in
`crates/server/src/api/common.rs`.

## Out of scope (tracked separately)

* Rolling the convention out to Volumes, Schedules, Memory stores,
  Knowledge bases, Payment accounts/policies, API keys, and Saved
  reports, these don't currently flow through `UrlBuilder::wrap`
  with `WithUrls<T>` (most rely on the `decorate_value_links`
  middleware for `self_url`/`view_url`). Opting them in requires a
  per-handler change to the response wrapper.
* A ratchet gate for "% of entity responses carrying allowed_actions"
, designable after the remaining clusters have opted in so the
  floor isn't premature.
