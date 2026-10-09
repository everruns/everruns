---
type: Specification
title: "Channels"
description: "One channel implementation for the Framework, serve and the server: a channel is a definition plus a platform driver, run by one channel host in core, with hosts plugging in only storage, secrets and HTTP."
tags:
  - everruns
  - integrations
  - channels
  - framework
  - serve
  - server
---
# Channels

## Abstract

A channel is a door through which people reach an agent: a Slack app, a
webhook, an AG-UI page, an A2A endpoint, a voice line. Channels work the way
agents do. An agent is a definition run by one engine against storage the host
plugs in; a channel is a **definition** plus a **platform driver**, run by one
**channel host** in `everruns-core`. The Framework, `everruns-serve` and the
platform server differ only in where the definition comes from and which
storage, secrets and HTTP stack they plug in.

## Why

Before this design there were three channel stories. `everruns-contracts`
had a neutral model (`InboundChannelEvent`, `ChannelDeliveryAdapter`,
`SessionBinding`) that only the server's Slack implemented; the logic that turns
a session's events into replies (streaming, status, approval prompts, failure
notices, recovery) lived in the server's Slack dispatcher; serve had its own
smaller `Channel` trait and a second Slack parser that posted only the final
text; the Framework had nothing. Every fix landed in one place only, and AG-UI,
A2A and voice each became "built in, not a channel" exceptions.

## Model

| Layer | Owns | Lives in |
|---|---|---|
| Wire types | `InboundChannelEvent`, `ExternalActor`, `SessionBinding`, `ChannelReplyMode`, `ChannelDeliveryAdapter` (post, stream, status), `DeliveryTarget` | `everruns-contracts` (`runtime::channel`) |
| Driver | One platform: parse and verify a request into an inbound message or a direct answer; deliver replies | core (`webhook`), `everruns-integrations` one feature per platform |
| Reply delivery | One turn's events in, platform calls out: automatic vs tool-only replies, per-message streams with retraction, status and title, approval prompts, the notice for a turn that produced nothing | core `channel::delivery` |
| Channel host | Request in, response out: driver, duplicate filter, thread to session binding, sender attribution, start or steer the turn, run reply delivery, recover pending deliveries after restart, proactive posts | core `channel::host` |
| Host plug-ins | Session port (create, send, events), channel store (bindings, seen keys, pending deliveries), secrets, HTTP mounting | Framework, serve, server |

Everything not platform-specific runs in core, behind the `channels` feature.
Hosts never re-implement it.

### Channel kinds

- **Messaging**: a request arrives (usually a webhook), the request is
  acknowledged at once, and replies are delivered later through the driver.
  Slack, webhook, scheduled posts with a delivery target, later Telegram,
  Discord, Teams. Driven by reply delivery.
- **Streaming**: the request carries the response (AG-UI, A2A, voice, the
  `api` channel of the [Agent Execution API](agent-execution-api.md)). They
  share the host's binding, store and session port and answer from the
  session's own stream. They join the host after messaging channels; until
  then they keep their existing routes.

### Session port

The host never owns sessions. A host implements three operations: create a
session for a channel's agent, send an input message (which starts a turn or
steers the running one), and stream the session's events (live from now, or
after a durable sequence for recovery). The Framework implements it over
`Engine`/`Session`, serve over its host, the server over its services. Same
shape as the voice loop's `VoiceSessionPort`.

### Binding and duplicates

`SessionBinding` decides which session a message lands in: one per thread
(default for messaging), per conversation, per requester, one shared per
channel, or a new one per message. The binding key and the platform's dedup key
are stored in the channel store, so a platform retry (Slack resends on a slow
ack) never starts a second turn.

### Reply delivery

One `TurnDelivery` per turn, keyed by the input message id. It reads the
canonical event stream: it does not depend on `data.accumulated`, which the
Framework drops from deltas. Rules carried over from the Slack dispatcher:

- Automatic mode posts each completed assistant message; tool-only mode posts
  only what `channel_post_message` sends and observes its receipts.
- Streaming is per output message, flushed on a cadence; a guardrail
  replacement rewrites the open stream; every opened stream is stopped.
- Status and title are advisory: a failure is logged, never fails the turn.
- A turn that ends having delivered nothing posts exactly one short notice,
  with no error detail, plus an optional session link from the host.
- A cancellation ends every delivery on the session once the turn boundary was
  seen.

### Recovery

A pending delivery (channel, session, input message id, delivery target, last
sequence) is saved when a turn starts and cleared when it ends. On start the
host replays each pending delivery from its sequence. The target never holds
credentials: the driver re-derives them.

## Host surfaces

- **Framework** (`everruns` feature `channels`): `Channel` values added to
  `Channels::new(&engine)`; `handle(name, request)` from any HTTP stack, and
  `start(...)` for a proactive post. No HTTP listener of its own.
- **serve**: `#[channel]` returns the same `Channel`; serve mounts
  `POST /v1/channels/{name}` and lists channels in the manifest. Channel and
  agent names share the `/v1/channels/{name}` namespace.
- **server**: an `agent_channels` row maps onto the same definition; the server
  keeps management only (UI, Slack app install, rate limits, billing) and its
  Postgres store and wake-up.

## Decisions

- **No new crate.** Wire types in contracts, runtime in core behind a feature,
  platform drivers in `everruns-integrations`.
- **No HTTP framework in core.** Requests and responses are plain values;
  hosts adapt them.
- **Auth stays with the host for now.** Drivers verify platform signatures;
  caller auth (keys, OIDC) plugs in when the channel auth verifier moves to
  core with the Agent Execution API.
- **No backward compatibility for serve or the Framework.** serve's earlier
  `Channel` trait is replaced, not wrapped.

## Rollout

1. Core host, reply delivery, memory store, webhook driver; Framework
   `Channels`.
2. serve on the core host.
3. Slack driver in `everruns-integrations`, used by serve.
4. Server Slack delivery on core reply delivery and the shared Slack client.
5. Streaming kinds (AG-UI, A2A, voice, `api`) as host channels.

## Source index

- Wire types: [`crates/contracts/src/runtime/channel.rs`](../../crates/contracts/src/runtime/channel.rs)
- Core host: [`crates/core/src/channel/`](../../crates/core/src/channel/)
- Framework: [`crates/everruns/src/channels.rs`](../../crates/everruns/src/channels.rs)
- Earlier model and Slack rules: [Messaging Integrations](messaging-integrations.md), [Slack Bot Integration](slack-integration.md)
