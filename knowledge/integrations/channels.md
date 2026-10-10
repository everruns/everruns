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
| Wire types and driver contract | `InboundChannelEvent`, `ExternalActor`, `SessionBinding`, `ChannelDeliveryAdapter` (post, stream, status), `DeliveryTarget`, `ChannelDriver` with its request, response and error | `everruns-contracts` (`runtime::channel`) |
| Driver | One platform: parse and verify a request into an inbound message or a direct answer; deliver replies | `everruns-integrations`, one feature per platform (`webhook-channel` first) |
| Reply delivery | One turn's events in, platform calls out: automatic vs tool-only replies, per-message streams with retraction, status and title, approval prompts, the notice for a turn that produced nothing | core `channel_runtime` (`TurnDelivery`) |
| Channel host | Request in, response out: driver, duplicate filter, thread to session binding, sender attribution, start or steer the turn, run reply delivery, recover pending deliveries after restart, proactive posts | core `channel_runtime` (`ChannelHost`) |
| Host plug-ins | Session port (create, send, events), channel store (bindings, seen keys, pending deliveries), secrets, HTTP mounting | Framework, serve, server |

Everything not platform-specific runs in core, behind the `channels` feature.
Hosts never re-implement it.

### Channel kinds

- **Messaging**: a request arrives (usually a webhook), the request is
  acknowledged at once, and replies are delivered later through the driver.
  Slack, webhook, scheduled posts with a delivery target, later Telegram,
  Discord, Teams. Driven by reply delivery.
- **Streaming**: the request carries the response (AG-UI, A2A, voice, the
  `api` channel of the [Agent Execution API](agent-execution-api.md)). A
  streaming channel has no driver: the host binds its conversation to a
  session (`ChannelHost::conversation`) through the same store and port, and
  `stream_turn` sends the input and returns that turn's events, ending at the
  turn's terminal event. The surface encodes them in its own protocol (AG-UI
  SSE, A2A task events). Voice speaks from the session's events through one
  shared mapper (`voice::AgentOutputMapper`).

### Session port

The host never owns sessions. A host implements three operations: create a
session for a channel's agent, send an input message (which starts a turn or
steers the running one), and stream the session's events (live from now, or
after a durable sequence for recovery). Send and events also name the
channel, so a host that restarted can reopen a session it no longer holds
with that channel's agent. The Framework implements it over
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

- No mode setting: the events say how the agent talks
  ([Explicit Communication](explicit-communication.md)). Completed assistant
  messages are posted unless they are an explicit agent's working notes;
  `conversation.message` events are posted unless their sender already
  delivered them; a successful `no_reply` suppresses the end-of-turn notice.
- Streaming is per output message, flushed on a cadence; a guardrail
  replacement rewrites the open stream; every opened stream is stopped.
- Status and title are advisory: a failure is logged, never fails the turn.
- A turn that ends having delivered nothing posts exactly one short notice,
  with no error detail, plus an optional session link from the host.
- A cancellation ends every delivery on the session once the turn boundary was
  seen.
- A message is delivered once: a delta or completion for a message already
  completed, closed or replaced is dropped, since live deltas and durable
  events travel separately.
- Optional adapter surfaces, each a capability probe that defaults to off:
  streaming, status and title, approval prompts for a `request_approval`
  pause, and one task-progress message edited on flush and pushed a last time
  at turn end. Prompts and progress count as delivered.

### Recovery

A pending delivery (channel, session, input message id, delivery target, last
sequence) is saved when a turn starts and cleared when it ends. On start the
host replays each pending delivery from its sequence. The target never holds
credentials: the driver re-derives them.

## Host surfaces

- **Framework** (`everruns` feature `channels`): `Channel` values added to
  `Channels::new(&engine)`; `handle(name, request)` from any HTTP stack, and
  `start(...)` for a proactive post. No HTTP listener of its own.
- **serve**: `#[channel]` returns a driver or a `Channel`, with
  `agent = "…"` to pick the agent that answers; serve mounts
  `POST /v1/channels/{name}`, lists channels and their secrets in the
  manifest, keeps channel state in its SQLite store, and recovers cut-off
  replies at boot. A schedule posts with
  `start_session(..).deliver_to(slack::channel("C…"))`, which goes through
  the host's `send`. Channel and agent names share the `/v1/channels/{name}`
  namespace, so a clash is a discovery error.
- **server**: an `agent_channels` row maps onto the same definition, resolved
  per request (see [Server on the host](#server-on-the-host)). The server keeps
  management (UI, Slack app install, keys, rate limits, budgets, audit) and its
  Postgres store, port and wake-up.

## Server on the host

The server's channels are database rows with per-row secrets, owners and
tenants, and its intake grew per platform: Slack, webhook, `api`, the frozen
`api_endpoint`, AG-UI, public chat, FCP and A2A each find or create sessions
on their own (session tags, oldest match wins, no unique key, so two first
messages can race), dedup their own way or not at all, and recover by scanning.
The target is one intake: every server channel goes through `ChannelHost`, and
what stays server-specific is what is genuinely tenant or protocol state.

### Host changes (core)

- **Resolved channels.** The host stops assuming a static map. A host passes a
  `ResolvedChannel` (config plus driver, or config plus stream kind) per call;
  the builder's static map becomes one `ChannelResolver`, the server's resolver
  reads `agent_channels`. Recovery resolves each pending delivery's channel
  through the resolver and leaves it when the channel is only unavailable;
  it drops it only when the resolver says the channel is gone.
- **Host scope.** `ChannelConfig` carries an opaque, typed scope the host's
  port downcasts (the server's ingress context: org, agent, harness, owner,
  virtual user, channel row id). `NewChannelSession` and `send` receive the
  config, so the port creates sessions and attributes messages with the
  channel's tenant and owner instead of reading them back from tags.
- **Atomic binding.** `bind` becomes first-writer-wins and returns the bound
  session; a host whose create lost the race discards its session through
  the port. Stores key bindings by the channel's stable id, never its
  display name, so keys cannot cross tenants.
- **Message metadata from the driver.** `InboundChannelEvent` carries the
  platform's message metadata (Slack `slack_ts`, `slack_thread_ts`) onto the
  input message, so dedup, history and approvals read it from one place.
- **Intake hooks.** A driver may answer three optional questions the Slack
  intake needs and others can use: `admit` (respond or stay silent, with the
  bound session's history: Slack's relevance policy), `backfill` (history to
  seed a session created mid-thread), and control events (`Inbound::Control`:
  cancel the bound turn, rename it, update its context).
- **Decisions.** `Inbound::Decision` carries an approval or a declined tool
  call from a platform button; the host checks the decider against the
  session's policy through the port and resumes the turn. Slack's buttons are
  the first user; the same shape serves Teams or Telegram later.
- **Streams end at a park.** `stream_turn` and a new events-only `follow`
  (resume after answering a question, no new message) end at a terminal event
  or at a park (`tool.call_requested` waiting on a person or the client),
  which AG-UI and A2A need. Events stay JSON envelopes; core gives one typed
  parse for surfaces that translate typed events.
- **Leased recovery.** Pending deliveries carry a lease, so several server
  instances recover each delivery once.

### Server plug-ins

- **Store.** Three tables: `channel_bindings` (unique channel and key,
  session), `channel_seen` (channel and dedup key, pruned after a day) and
  `channel_pending_deliveries` (with lease). Existing tag bindings are
  backfilled once, in the same migration, so no conversation loses its
  session.
- **Port.** Create is `SessionService::create_from_app` with the scope's
  tenant, owner, source and channel row; send is `MessageService::create`,
  whose `delivery` already says started or steered, with the sender mapped to
  a principal and session participant; events are `EventDelivery` live plus
  `EventService::list_advanced` replay, the pattern `session_event_sse`
  already uses.
- **Resolver.** Public id to row, liveness, decrypted config, driver built per
  row (signing secret and bot token from the row). Caller auth, rate limits
  and the not-found shape stay in the HTTP layer in front of the host.
- **Reply delivery.** Slack's own dispatcher (PostgreSQL poll, live-delta
  merge, restart scan) is replaced by the host's delivery over the port.

### Per channel

| Channel | On the host as | Stays server-specific |
|---|---|---|
| Slack | Driver: the shared Slack driver gains multi-tenant config, pane events (control), `file_share`, display names, `admit`, `backfill`, decisions. Dedup key is channel and message ts, which also folds the mention and message pair Slack sends for one post. Thread keys are channel-qualified; the backfill rewrites the old ones | App install and manifest provisioning, onboarding evidence |
| webhook | Driver with token check and message template; shared or per-message binding; no reply | Trigger webhooks (`agent_triggers`) |
| `api` | Streaming: `conversation` for a requester binding, explicit sessions otherwise; `stream_turn` for send and SSE | Agent keys, visibility projection |
| `api_endpoint` | Same intake as `api` until it is removed | Its frozen wire shape |
| AG-UI, public chat | Streaming: `conversation` keyed by thread and end user (visitor hash for public chat), `stream_turn` and `follow` | Frontend tools, interrupts, snapshot, expiry, Turnstile |
| FCP | Streaming: `conversation` keyed by its cookie token, `stream_turn` | Markdown reply shape |
| A2A | Streaming: `conversation` for shared mode, one-off sessions otherwise; `stream_turn` and `follow` | Tasks, push configs, PACT, templates |
| Poppy | Port only (send knows steer from start) | Its conversation table: the binding is a protocol object with owner, context and close |
| voice | Port only (session creation) | Calls, leases, the voice loop |


## Decisions

- **No new crate.** Wire types and the driver contract in contracts, runtime
  in core behind a feature, platform drivers in `everruns-integrations`, which
  depends on contracts only.
- **No transport in core.** Requests and responses are plain values that hosts
  adapt; drivers own their HTTP clients, so core keeps its kernel dependency
  guard.
- **Auth stays with the host for now.** Drivers verify platform signatures;
  caller auth (keys, OIDC) plugs in when the channel auth verifier moves to
  core with the Agent Execution API.
- **No backward compatibility for serve or the Framework.** serve's earlier
  `Channel` trait is replaced, not wrapped.

## Rollout

1. Done: core host, reply delivery, memory store; webhook driver in
   `everruns-integrations`; Framework `Channels`.
2. Done: serve on the core host, with the Slack driver in
   `everruns-integrations` (feature `slack-channel`).
3. Server Slack delivery on core reply delivery: done, the missing core
   pieces (late-delta guard, approval prompts, task progress, per-turn
   surface switch) and the server dispatcher driving one `TurnDelivery` per
   turn from both the PostgreSQL poll and live deltas. The Slack Web API
   envelope (`slack_channel::web_api`) is shared by the serve driver and the
   server.
4. Streaming kinds: done for serve (AG-UI threads and A2A contexts bind
   through the host and its store) and the shared voice output mapper
   (Framework and server). The Framework's `AgUiThreads` keeps its own
   `ThreadStore`: it reopens a thread's session after a restart and scopes
   thread ids per caller, which the channel store does not model.
5. Server on the host (see [Server on the host](#server-on-the-host)), one
   PR each: host changes; server store, port and resolver; webhook and
   `api`/`api_endpoint`; AG-UI, public chat and FCP; A2A; Slack intake and
   delivery; Poppy and voice on the port. Each PR deletes the server code it
   replaces.

## Source index

- Wire types: [`crates/contracts/src/runtime/channel.rs`](../../crates/contracts/src/runtime/channel.rs)
- Driver contract: [`crates/contracts/src/runtime/channel_driver.rs`](../../crates/contracts/src/runtime/channel_driver.rs)
- Core host: [`crates/core/src/channel_runtime/`](../../crates/core/src/channel_runtime/)
- Webhook driver: [`crates/integrations/src/webhook_channel/`](../../crates/integrations/src/webhook_channel/)
- Slack driver: [`crates/integrations/src/slack_channel/`](../../crates/integrations/src/slack_channel/)
- serve: [`crates/serve/src/channels.rs`](../../crates/serve/src/channels.rs)
- Framework: [`crates/everruns/src/channels.rs`](../../crates/everruns/src/channels.rs)
- Earlier model and Slack rules: [Messaging Integrations](messaging-integrations.md), [Slack Bot Integration](slack-integration.md)
