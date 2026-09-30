---
type: Specification
title: "MCP Events: outbound session webhooks"
description: "/mcp clients such as ChatGPT subscribe to session completion, failure and input-required webhooks; spec revision pinned, delivery, and access rules."
tags:
  - everruns
  - mcp
  - integrations
---
# MCP Events: outbound session webhooks

## Why

An agent run started from ChatGPT or another MCP client can take minutes, and
can stop to ask the user something. Without events the client has to poll
`session_get_status`. MCP Events lets the client subscribe once and be told
when a session finishes, fails, or needs the user. This is the outbound half of
EVE-1121. Inbound MCP Events (Everruns subscribing to other servers) is a
separate trigger source and is not covered here.

## Spec revision (pinned 2026-09-30)

Implemented against the MCP Events draft on protocol `2026-07-28`, as used by
OpenAI's MCP Events guide for ChatGPT. The draft is not final, so the surface
is behind the experimental `mcp_events` flag and org opt-in. Points we rely on:

- `capabilities.events: {}` in `server/discover` advertises support.
- `events/list` returns each event's `name`, `description`, `delivery`
  (`["webhook"]` here), `inputSchema` (subscription arguments) and
  `payloadSchema`.
- `events/subscribe` takes `name`, `arguments`, `delivery: {mode: "webhook",
  url, secret}`, optional `cursor` and `ttlMs`, and returns `id`,
  `refreshBefore`, `cursor` and `truncated`. It is idempotent on
  (principal, url, name, canonical arguments). The secret is `whsec_` plus
  base64 of 24 to 64 bytes.
- Before activation the server POSTs `{"type": "verification", "challenge"}`
  to the callback; the receiver echoes the challenge. Failure is JSON-RPC
  error `-32015` with `data.reason`.
- Deliveries are `{eventId, name, timestamp, data, cursor}`, at most 256 KiB,
  signed per Standard Webhooks (`webhook-id`, `webhook-timestamp`,
  `webhook-signature: v1,<base64 HMAC-SHA256>`), plus
  `X-MCP-Subscription-Id`. `410` ends the subscription; `413` is not retried.
- `events/unsubscribe` takes `name`, `arguments` and `delivery.url`.

When the draft changes, update this section and the code together.

## Events

| Event | Fires on | Payload beyond session, agent, title, link |
|---|---|---|
| `session.completed` | `turn.completed` | none |
| `session.failed` | `turn.failed` | `error_code` |
| `session.input_required` | `ask_user` requested, or `request_approval` awaiting | `kind` (`question`/`approval`), `tool_call_id` |

Subscription arguments filter by `agent_id` or `session_id`. `link` points at
the session in the Everruns UI; with the [MCP Apps](../ui/mcp-apps.md) views
the client can also open the session in place.

## Decisions

- **Webhook delivery only.** `/mcp` is stateless; there is no stream to carry
  in-band events. No replay either: a subscribe carrying a `cursor` gets
  `truncated: true` so the client knows it may have missed events.
- **No transcript text in payloads.** Identifiers and state only. The client
  reads content with `session_get_status` under its own token. A webhook
  therefore cannot become a channel for model output into another system's
  model, and a failed turn's error message (which can carry provider detail)
  stays out; only its code is sent.
- **Access is checked at delivery, not only at subscribe.** A subscription is
  bound to the MCP token's user. Anonymous MCP (no-auth local mode) cannot
  subscribe. Each delivery re-checks that the org still has the flag, that the
  user is still a member with session view, and that the session is not a
  private Platform Chat session. A former member's subscriptions are deleted.
- **Callbacks are verified and egress-safe.** HTTPS only, resolved and pinned
  against private ranges, no redirects, through the host egress service. The
  signing secret is stored encrypted.
- **Retries.** Three retries with backoff, same `webhook-id`, fresh timestamp
  and signature each attempt. Delivery runs off the event path.
- **Subscriptions expire.** Default 24 hours, `ttlMs` clamps to one minute to
  seven days; the client refreshes by subscribing again before
  `refreshBefore`.

## Security

TM-MCP-010 in the [threat model](../security/threat-model.md): callback SSRF,
cross-tenant or stale-access delivery, and payload injection.

## Code

`crates/server/src/services/mcp_events.rs` (catalog, subscribe, verification,
delivery, listener), `crates/server/src/api/mcp_endpoint/events.rs`
(JSON-RPC), `crates/server/migrations/147_mcp_event_subscriptions.sql`, and
the `mcp_events` flag in `crates/platform/src/feature_flags.rs`. Tests in
`crates/server/tests/domain/mcp_events_test.rs`.
