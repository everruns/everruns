---
type: Specification
title: "Slack Integration"
description: "Slack bot channel: one Slack App per Everruns App, signing-secret trust, tag-based session routing, and event-driven delivery."
tags:
  - everruns
  - integrations
  - slack
  - messaging
---
# Slack Integration

## Abstract

Slack deploys an agent as a bot. An Everruns [App](apps.md) binds a harness and
an optional agent to a Slack workspace; inbound Slack events reach a per-app
webhook, land in a session chosen by tag, and the agent's output is posted back
asynchronously. Slack is the reference implementation of the
[messaging integrations](messaging-integrations.md) channel abstraction:
`InboundChannelEvent` for parsing, `build_session_routing_tag()` for routing,
`ThreadContext` for participants, and `SlackDeliveryAdapter` for delivery.

## Design Decisions

- **One Slack App per Everruns App.** Slack bots are user-facing: each has a
  name, avatar, and scope set that belongs to a use case, not to the platform.
  This is the opposite of the GitHub integration, where a single global App is
  correct because it carries no identity in the conversation. The cost is a
  per-app installation ceremony, which is why the manifest endpoint exists.
- **Per-app manifest generation over manual setup.** `GET /v1/apps/{app_id}/slack/manifest`
  returns YAML with the correct scopes and bot user plus a "create from
  manifest" URL, so the scope list cannot drift from what the code needs.
  `event_subscriptions` is deliberately omitted: it requires a live webhook URL,
  which does not exist until the app is published.
- **The webhook is app-scoped and unauthenticated.** Slack has no Everruns API
  key, so trust comes from HMAC-SHA256 verification against the app's own
  signing secret, and the app is looked up across organizations
  (`get_by_public_id_unscoped`) because the request carries no auth context.
  Forgery and replay are the load-bearing risks; see TM-SLACK-001..003 in the
  [threat model](../security/threat-model.md).
- **Session identity comes from Slack's own thread structure.** Routing tags
  (`slack:thread:{ts}`, `slack:channel:{id}`, `slack:user:{id}`) let the session
  strategy be a configuration choice rather than a code path, and make a session
  findable from an inbound event alone with no extra mapping table.
- **Delivery is event-driven, not deadline-driven.** Slack requires an ack in
  under three seconds, while an agent turn can run for hours. The webhook acks
  immediately and registers with `SlackDeliveryDispatcher`, which subscribes to
  the event notification broadcaster (PostgreSQL `NOTIFY`) and posts
  `output.message.completed` text as it arrives, filtered by `input_message_id`
  so concurrent turns do not cross, and unregisters on `turn.completed` or
  `turn.failed`. Polling remains only as a `DEV_MODE` fallback where there is no
  PostgreSQL to listen on. Transient post failures retry with exponential
  backoff; non-retryable ones (invalid token, unknown channel) fail immediately
  rather than burning attempts.
- **Delivery survives restart.** In-memory registrations would strand every
  in-flight turn on deploy, so `SlackDeliveryDispatcher::recover()` rebuilds
  them at startup from active sessions carrying `slack:*` tags.
- **Deduplication is database-level, not in-memory.** Slack sends both
  `app_mention` and `message` for a single @mention, and any instance may
  receive either. The check is a containment query on the already-persisted
  input events keyed by `slack_ts`, so it holds across instances and restarts.
- **Thread history is injected, not inferred.** When the bot is first mentioned
  mid-thread, prior messages are fetched via `conversations.replies` and written
  as `input.message` events without triggering a turn, with bot messages taking
  the assistant role and human messages carrying `ExternalActor` attribution.
  Failure is non-fatal: an agent with no history is better than no reply.
- **Reply mode is a product decision about noise.** `all_messages` posts every
  completed assistant message. `report_progress_only` posts a deterministic
  acknowledgement, then only what the agent explicitly reports through the
  `report_progress` tool, which is the right shape for long handoff-style work
  where intermediate assistant chatter is not wanted in a channel.
- **User identity is channel-agnostic.** Slack user IDs are resolved to display
  names and stored as `ExternalActor`, the same struct any future Discord or
  Teams adapter populates, and the prefixing that tells the model who is
  speaking lives in `ReasonAtom` rather than in the Slack handler. Name
  resolution is cached in process, with permanent API errors cached as absent
  and transient errors left uncached so they retry.
- **Attachments become content parts, never dropped messages.** Images become
  image parts; other files and legacy attachments become text descriptions. A
  message carrying only attachments still reaches the agent.

## Where the Details Live

| Detail | Source of truth |
|---|---|
| Channel config fields, session strategies, reply modes | `SlackChannelConfig` in `crates/platform/src/app.rs` |
| Webhook handling, manifest generation, signature verification, identity resolution | `crates/server/src/api/slack_events.rs` |
| Delivery adapter, dispatcher, retry, startup recovery | `crates/server/src/slack_delivery.rs` |
| Channel abstraction types | `crates/core/src/channel.rs`, see [Messaging Integrations](messaging-integrations.md) |
| `ExternalActor` | `crates/core/src/message.rs` |
| Endpoint paths and payloads | route table in `crates/server/src/api/slack_events.rs` and the OpenAPI export |
| Tests, including live-API coverage and its Doppler variables | `crates/server/tests/slack_integration_test.rs` |
| Setup instructions for users | [`docs/integrations/slack.md`](../../docs/integrations/slack.md) |
| Apps UI surface | `apps/ui/src/app/(main)/apps/` |

## Known Gaps

- Agents cannot pull file content on demand. A Slack file-fetch tool
  (`url_private` plus bot token) would remove the current
  image-or-description-only limit on attachments.

## See also

- [Messaging Integrations](messaging-integrations.md), the abstraction Slack implements
- [Apps](apps.md), the entity a Slack channel binds to
- [Threat Model](../security/threat-model.md), TM-SLACK-001 through TM-SLACK-003
