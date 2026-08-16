---
type: Specification
title: "Per-operation request/response examples"
description: "Per-operation request/response examples on `#[utoipa::path]` handlers."
tags:
  - everruns
  - execution
---
# Per-operation request/response examples

Convention for **operation-level** examples on every `#[utoipa::path]`
handler so an LLM toolcaller sees concrete payloads at the operation
level, not just per-field literals. Companion to
[`api-conventions.md`](api-conventions.md) (hypermedia) and
[`api-llm-extensions.md`](api-llm-extensions.md) (cost/safety
metadata).

## What
Per-field `#[schema(example = …)]` (gate-tracked separately) tells an
LLM **what literal to put in one field**. Per-operation examples tell
an LLM:

- **How fields combine** into a valid request payload.
- **What an error response actually looks like** for that specific
  operation, complete with the right `code`/`detail` shape.

## Wire format
utoipa emits the operation-level example directly into the response
content:

```yaml
responses:
  "201":
    description: Session created successfully
    content:
      application/json:
        schema:
          $ref: "#/components/schemas/WithUrls_Session"
        example:
          self_url: https://api.example/v1/sessions/session_…
          view_url: https://app.example/sessions/session_…/chat
          id: session_…
          status: started
          …
```

## Authoring

Add `example = json!(…)` to the `responses(…)` and `request_body(…)`
entries in `#[utoipa::path]`. Keep examples **realistic**: use IDs
that match the documented prefix format, populate optional fields the
LLM is likely to set, and stick to one example per
(status × content type) pair to avoid utoipa's
non-deterministic `examples` map ordering (same caveat as
`extensions`, see `api-llm-extensions.md`).

### Single example per response

```rust
#[utoipa::path(
    post,
    path = "/v1/sessions",
    request_body(
        content = CreateSessionRequest,
        example = json!({
            "harness_name": "generic",
            "title": "Debug login issue",
            "tags": ["debugging", "urgent"]
        })
    ),
    responses(
        (
            status = 201,
            description = "Session created successfully",
            body = WithUrls<Session>,
            example = json!({
                "self_url": "https://api.example/v1/sessions/session_01933…",
                "view_url": "https://app.example/sessions/session_01933…/chat",
                "ui_link":  "https://app.example/sessions/session_01933…/chat",
                "id": "session_01933b5a00007000800000000000001",
                "status": "started",
                "organization_id": "org_00000000000000000000000000000001",
                "harness_id": "harness_01933b5a00007000800000000000001",
                "title": "Debug login issue",
                "tags": ["debugging", "urgent"],
                "created_at": "2026-05-27T15:24:00Z",
                "updated_at": "2026-05-27T15:24:00Z"
            })
        ),
        (
            status = 404,
            description = "Harness, Agent, or Model not found",
            body = ErrorResponse,
            example = json!({
                "type": "https://docs.everruns.com/errors/harness_not_found",
                "title": "Not Found",
                "status": 404,
                "detail": "Harness 'generic' not found in org org_00000000000000000000000000000001.",
                "code": "harness_not_found"
            })
        ),
    ),
    tag = "sessions"
)]
```

## When to add an example

* **Always for `POST` / `PATCH` / `PUT`** that mutate non-trivial
  state, the LLM needs to see the realistic payload shape, not
  just inferred from the schema.
* **Always on the canonical success response** (`200` / `201`).
* **On every `4xx` that the operation can specifically emit**,
  e.g. `409 Conflict` for already-archived, `429 Too Many Requests`
  with `retry_after_seconds`. Generic `500` doesn't usually need one.
* **`GET` is optional**: listing endpoints benefit most when there
  are nested objects (`messages`, `events`) so the LLM can see the
  envelope shape.

## Out of scope

* Per-field examples, tracked separately by EVE-491.
* OpenAPI `examples` (multi-example map), deferred until utoipa
  switches to a deterministic container; the single `example`
  attribute is stable.
* Coverage ratchet, designable after the rollout reaches enough
  endpoints; otherwise the floor would be premature.

## First-wave rollout

This convention lands with examples on a small first wave of
high-traffic endpoints so the convention can settle. A follow-on
issue tracks the per-endpoint sweep.
