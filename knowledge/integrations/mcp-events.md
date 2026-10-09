---
type: Specification
title: "MCP Events: session webhooks out, agent triggers in"
description: "/mcp clients subscribe to session webhooks (outbound); agents subscribe to events on their MCP servers as mcp_event triggers (inbound). Spec revision pinned, delivery, verification and access rules."
tags:
  - everruns
  - mcp
  - integrations
---
# MCP Events: session webhooks out, agent triggers in

## Why

An agent run started from ChatGPT or another MCP client can take minutes, and
can stop to ask the user something. Without events the client has to poll
`session_get_status`. MCP Events lets the client subscribe once and be told
when a session finishes, fails, or needs the user. That is the outbound half of
EVE-1121.

The inbound half turns it around: an agent subscribes to events on one of its
own MCP servers (a tracker announcing new issues, say) and wakes on each one,
as an `mcp_event` [agent trigger](../runtime-resources/agent-triggers.md). See
[Inbound: MCP event triggers](#inbound-mcp-event-triggers).

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

## Inbound: MCP event triggers

An `mcp_event` trigger names one of the agent's MCP server attachments (agent
or harness layer), an event from that server's `events/list`, and the
subscription arguments. Everything after verification is the shared trigger
event pipeline: filter, dedupe, session routing, delivery log.

- **The agent subscribes as itself.** `events/*` calls use the attachment's own
  transport and credential, the same ones its tool calls use: headers and API
  key, or for a catalog preset with `actsAs: service`, the agent identity's
  OAuth grant. An `actsAs: user` attachment is refused, since a trigger has no
  user to act as.
- **One secret per subscription, generated by Everruns.** Create, enable, and
  any change to server, event or arguments subscribe with a fresh `whsec_`
  secret; the previous subscription is unsubscribed first. The secret is stored
  encrypted beside the subscription state, never in the trigger config.
- **The callback is the trigger's ingress**,
  `/v1/e/{ingress_id}/mcp-events`. The Standard Webhooks signature, with the
  timestamp held to five minutes either way, is checked before the body is
  parsed. The verification challenge is answered only when signed. `webhook-id`
  is the pipeline's event id, so a replay is a recorded duplicate. The event
  `name` must be the subscribed one; `data` is the template's `payload`, and
  `{{mcp.server}}`, `{{mcp.event}}`, `{{mcp.event_id}}` describe the delivery.
- **410 ends it.** A delivery for a trigger that is unknown, disabled, deleted,
  flagged off, or for a superseded subscription id answers `410 Gone`. That is
  also the backstop when `events/unsubscribe` cannot reach the server.
- **A create either subscribes or leaves nothing.** A subscribe the server
  rejects deletes the new trigger and returns the server's error; a failed
  re-subscribe on update restores the previous config, disabled.
- **Refresh is a periodic sweep.** A background task on each API replica
  re-subscribes, with the same secret and the last delivered `cursor`, any
  subscription within 15 minutes of `refreshBefore`, every 60 seconds
  (`MCP_EVENT_TRIGGER_REFRESH_INTERVAL_SECONDS`). State lives in the database
  and `events/subscribe` is idempotent, so replicas racing on a row do no harm.
  A failed refresh is retried after five minutes. A durable workflow was not
  needed: a missed tick only delays the refresh to the next one.
- **Gated by `mcp_events`.** Creating or re-subscribing needs the org's
  effective flag; with it off, deliveries answer `410` and refreshes skip.

## Security

TM-MCP-010 in the [threat model](../security/threat-model.md): callback SSRF,
cross-tenant or stale-access delivery, and payload injection. TM-TRIGGER-005
covers the inbound callback: forged, stale, replayed or oversized deliveries.

## Code

`crates/server/src/domains/mcp_servers/events.rs` (catalog, subscribe, verification,
delivery, listener), `crates/server/src/api/mcp_endpoint/events.rs`
(JSON-RPC), `crates/server/migrations/147_mcp_event_subscriptions.sql`, and
the `mcp_events` flag in `crates/server/src/records/feature_flags.rs`. Tests in
`crates/server/tests/domain/mcp_events_test.rs`.

Inbound: `crates/server/src/domains/agent_triggers/mcp_event.rs` (config,
subscribe, refresh, unsubscribe, receive), `crates/server/src/api/mcp_event_webhooks.rs`
(callback), `crates/server/src/services/standard_webhooks.rs` (signing shared
by both halves), `crates/server/migrations/157_agent_trigger_mcp_subscriptions.sql`.
Tests in `crates/server/tests/domain/mcp_event_triggers_test.rs`, against an
in-process fake MCP server.
