---
type: Specification
title: "Messaging Integrations"
description: "Messaging integrations."
tags:
  - everruns
  - integrations
---
# Messaging Integrations

## Abstract

Messaging integrations connect agents to external messaging platforms (Slack, Discord, Teams, Telegram). A shared abstraction layer decouples platform-specific protocols from the core runtime: channel adapters translate inbound platform events into `InboundChannelEvent`, route them to sessions via `SessionBinding`, and deliver agent output back via `ChannelDeliveryAdapter`. Multi-user threads are tracked via `ThreadContext` with per-message `ExternalActor` attribution.

## Design Decisions

- **ThreadContext is session-level, ExternalActor is message-level**: A session bound to a platform thread accumulates participants in `ThreadContext`. Each individual message carries `ExternalActor` for LLM attribution. This separation lets the agent know both "who's in this conversation" and "who said this specific thing."
- **Async agent invocation is first-class**: The `ChannelDeliveryAdapter` trait models the webhook→ack→async-response pattern. All platforms share the same lifecycle: register delivery on inbound event, deliver output when agent finishes (seconds to hours later), unregister on turn end.
- **Generic session tags**: Session routing uses `{platform}:thread:{ref}`, `{platform}:channel:{id}`, `{platform}:user:{id}` tags. The `build_session_routing_tag()` helper generates these from metadata. Slack's existing tags remain as a concrete instance of this pattern.
- **Agent-controlled communication is channel-neutral**: explicit posts let the agent choose when to communicate and how to word updates, questions, and answers. The runtime binds the destination to the input invocation and retains credential ownership in the control plane. Success confirms platform acceptance, rather than merely accepting a report payload. Whether an agent talks this way is the agent's Communication setting, not an endpoint choice; see [Explicit Communication](explicit-communication.md). See [the tool and sender contract](../../crates/contracts/src/runtime/conversation/tools.rs) and [the Slack adapter](../../crates/capabilities/src/channel_message_sender.rs).
- **Platform actions stay in capabilities**: Slack contributes native reactions, edits, user lookup, and file uploads through its hosted capability. Neutral posting shares its bound endpoint action service. See [Slack Agent Actions](slack-agent-actions.md).
- **Messaging runtime moves to core**: the channel host, reply delivery and binding store are shared by the Framework, serve and the server; see [Channels](channels.md). Server-only parts (Slack app install, interactivity, org records) stay under `crates/server/src/channels/{platform}/`, and the server's Slack delivery moves onto core reply delivery in a later rollout step.

## Types

See `crates/contracts/src/runtime/channel.rs` for full definitions.

### Thread & Participants

| Type | Purpose |
|------|---------|
| `ThreadContext` | Session-level thread state: thread_ref, platform, participants map |
| `Participant` | Actor + first_seen_at + optional role |
| `ThreadContext::track_participant()` | Idempotent participant accumulation |
| `ThreadContext::participants_summary()` | LLM-injectable "Thread participants: Alice, Bob" line |

### Inbound Events

| Type | Purpose |
|------|---------|
| `InboundChannelEvent` | Platform-agnostic inbound message (actor, text, attachments, dedup_key, thread_ref, routing metadata) |
| `InboundAttachment` | Image URL or file description |

### Outbound Delivery

| Type | Purpose |
|------|---------|
| `OutboundChannelMessage` | Text message to post back to platform thread |
| `ChannelDeliveryAdapter` | Host-owned acknowledgement, delivery, and lifecycle feedback |
| `DeliveryContext` | Auth token, channel ID, thread ref, the agent's communication mode, platform extras |
| `DeliveryResult` | Ok / TransientError / PermanentError |

### Session Routing

| Type | Purpose |
|------|---------|
| `SessionBinding` | Thread (default), Conversation, Requester, Endpoint, Ephemeral |
| `build_session_routing_tag()` | Generates `{platform}:{thread\|channel\|user}:{ref}` session tag. The segment keeps the pre-EVE-1005 word, not the binding name: renaming it orphans live sessions. |

## Adapter Lifecycle

```
1. Platform webhook → parse into InboundChannelEvent
2. build_session_routing_tag() → find or create session
3. Track participant in ThreadContext
4. Create input.message event (triggers agent workflow)
5. Register with delivery dispatcher (ChannelDeliveryAdapter)
6. Optional: send_ack() for async mode ("On it.")
7. Agent runs asynchronously...
8. Event notification → deliver(OutboundChannelMessage) → platform API
9. Turn ends (completed/failed/cancelled) → post a terminal notice if nothing
   was delivered, then unregister
```

A turn that ends without a delivered reply must still say so in the thread. A
failed turn, a cancellation, a turn that produced no text, and a reply the
platform refused are all indistinguishable from a hung agent otherwise — the
user sees only the message they sent. The notice is one status line with a link
back to the session; the failure text stays server-side because these threads
are frequently public. See `terminal_notice` in
[`crates/server/src/channels/slack/delivery/mod.rs`](../../crates/server/src/channels/slack/delivery/mod.rs).

Where a platform streams, the stream is per *output message*, not per turn: a turn
that produces three messages with tool calls between them is three streams, so the
reader sees three replies rather than one concatenated blob. Every terminal state
closes any stream still open — an unstopped stream is a message left spinning in
the client forever, which is worse than the silence above. Flush cadence belongs
next to its constant with the measurement that chose it, because the documented
rate-limit tier is a floor rather than the real ceiling.

Cancellation is session-scoped, not turn-scoped: both cancel paths mint a fresh
`input_message_id` for the synthetic `turn.cancelled` event, so a delivery that
matches it per-turn never unregisters.

## Messaging Integration Parity Requirements

Every messaging integration must ship with the following artifacts. Use Slack as the reference implementation.

| Requirement | Description |
|---|---|
| **Spec** | Knowledge concept (`knowledge/integrations/{platform}-integration.md`): architecture, webhook flow, security review. |
| **Inbound adapter** | Parse platform webhook into `InboundChannelEvent`. Use `build_session_routing_tag()` for session lookup. Track participants via `ThreadContext`. |
| **Delivery adapter** | Implement `ChannelDeliveryAdapter` trait for outbound message delivery. Handle retry with exponential backoff. Every outbound message goes through the trait, and transient-vs-permanent decision lives in the adapter alone — a dispatcher that also classifies lets the two lists drift apart. |
| **Signing/auth verification** | Platform-specific request authentication (e.g. HMAC signing secret for Slack, Ed25519 for Discord). |
| **Unit tests** | Webhook parsing, signature verification, session tag construction, delivery text extraction, bot message filtering. |
| **Integration tests** | `crates/server/tests/{platform}_integration_test.rs`, webhook→session→message flows against a per-test PostgreSQL database (`TestServer`). |
| **Live API tests** | Feature-gated tests against real platform API. Doppler credentials: `TEST_{PLATFORM}_*` vars. |
| **CI: unit tests** | Tests run in the `unit-test` job. |
| **CI: change detection** | Path filter for `{platform}` files. |
| **CI: live-test job** | Dedicated `{platform}-live-test` job, conditional on change detection + `push` event. |
| **User docs** | `docs/integrations/{platform}.md`, setup guide, scopes, session strategies, how the agent's Communication setting affects replies. |
| **Threat model** | Section in `knowledge/security/threat-model.md` covering platform-specific threats (signing bypass, bot loops, replay). |
| **Thread backfill** | When a new session joins an existing thread, backfill its history by following the platform's pagination cursor to the end — a single page is a silent truncation. Cap what is injected, and say so in the injected context when the cap bites, so the agent can tell a short thread from the tail of a long one. Backfill only where "new session" and "thread the agent has not seen" mean the same thing (for Slack, `per_thread` alone). |
| **Rich message rendering** | Agent output is Markdown. Post it through whatever rich-text primitive the platform offers (Slack: a `markdown` block) rather than the plain-text field, whose dialect is invariably smaller — tables, headings and fenced code are exactly what degrades. Keep the plain field populated as the notification fallback. Split past the platform's per-block size limit on a boundary that does not break a code fence, while bounding total outbound text and any repeated fence metadata before constructing payloads. |
| **Message correlation** | Stamp the session and input message id onto every posted message using the platform's metadata facility, so a platform message maps back to the run that produced it without tag-string heuristics. |
| **Thread context** | Persist a `ThreadContext` per session (participants, and where the user is looking when the platform reports it) and surface it as *conversation context*, never as system prompt — participant names and platform view reports are external user-controlled strings. Accumulate across messages and survive a restart. A platform signal that changes often (Slack: `app_context_changed`) updates the record rather than minting an event per change. Store it under the reserved session KV key `channel:thread_context`, which `session_storage` withholds from the agent-facing `kv_store` tool so a session actor cannot forge its own context. |
| **Inbound control signals** | A platform stop/cancel control is not a message: keep its blast radius fixed at cancel-only, resolve the session through the same endpoint-scoped lookup inbound messages use (so another endpoint's thread resolves nothing), and route it through the shared cancel path that checks terminal state first — a stop for a finished turn is a no-op, not an error. Let the terminal-state notice be the user's confirmation rather than posting a second one. |
| **Explicit posting** | Bind to the trusted input conversation. Return platform acceptance and an editable message reference; observe the receipt without forwarding it again. Treat posting as an at-most-once effect under durable Act. |
| **Working feedback** | Show pane status through the existing lifecycle adapter; explicit agents decide their own acknowledgements. |
| **Terminal-state notice** | A turn ending without a delivered reply posts exactly one status line with a session link (see Adapter Lifecycle). |
| **Streaming (optional)** | Implement `ChannelStreamDelivery` and return it from `ChannelDeliveryAdapter::streaming()`. A platform without progressive delivery returns `None` and keeps discrete posting — the capability is probed, not required. One stream per output message, closed on every terminal state. |
| **Startup recovery** | Re-register active deliveries after server restart (query sessions with `{platform}:*` tags). |
| **Wakeup without a listener** | The same dispatcher polls its active sessions when no PostgreSQL event listener runs (NATS delivery, or no listener URL). There is no separate deadline-bound fallback: one delivery path keeps terminal notices, streaming, and approvals on every backend. Where deltas are ephemeral and never reach PostgreSQL (NATS), each session with a registered delivery also subscribes to the event bus for its deltas only; a failed subscription degrades to completed-message posting. |

## Code Organization

Messaging integrations live in the server crate, one module tree per platform
under `crates/server/src/channels/`. Slack is the only messaging platform
implemented today, and everything that talks to Slack lives in its tree. The
protocol channels sit beside it the same way: `a2a/`, `ag_ui/`, `fcp.rs`,
`public_chat.rs` and `voice/` hold each channel's inbound handlers and wire
handling.

```
crates/server/src/channels/slack/
  delivery/        — ChannelDeliveryAdapter impl, dispatcher, streaming, recovery
  events/          — inbound HTTP: Events API webhook, interactivity, manifest,
                     signing verification, route registration
  install.rs       — one-click OAuth install and connection routes
  actions/         — agent-invoked Slack actions behind the worker seam
  approvals.rs     — approval cards and button decisions
  task_progress.rs — live task summary for a thread
  provisioning/    — app creation, branding, token rotation
  api.rs, api_error.rs — shared Slack Web API calls and error classification
```

The HTTP handlers sit beside the delivery code rather than under `api/` so the
integration reads from one tree; `app_builder` mounts their routers. Slack-shaped
domain records stay in `records/`, domain rules in `domains/agent_channels/`, and
the org connection store in `storage/`. A second platform gets its own
`channels/{platform}/` sibling; shared orchestration moves to `channels/mod.rs`
only once two platforms need it.

Core abstraction types remain in `crates/contracts/src/runtime/channel.rs`. Platform-specific channel configs (e.g. `SlackChannelConfig`) remain in `crates/server/src/domains/apps/record.rs`. Each `AgentChannel` holds transport type and configuration, enabling multiple independent endpoints per agent.

## Concrete Implementations

### Slack

Reference implementation. See [`knowledge/integrations/slack-integration.md`](slack-integration.md) for full details.

- Webhook: `POST /v1/channels/{channel_id}/slack/events` (with a permanent App-shaped alias)
- Signing: HMAC-SHA256 via `signing_secret`
- Session strategies: `per_thread`, `per_channel`, `per_user`
- Reply behavior: automatic assistant forwarding or agent-controlled communication, configured through [the Slack endpoint](../../crates/server/src/domains/apps/record.rs)
- Thread context injection via paginated `conversations.replies` (`per_thread` only, capped with a truncation notice)
- Event-driven delivery via `SlackDeliveryAdapter` (implements `ChannelDeliveryAdapter`)
- Replies rendered as bounded `markdown` blocks, split past Slack's per-block limit, stamped with session/message `metadata`
- Thread context (participants + current view) persisted per session, rendered by the `channel_context` capability
- Startup recovery: re-registers active sessions with `slack:*` tags
- No PostgreSQL event listener (NATS deployments): the dispatcher polls active sessions, see [`wake.rs`](../../crates/server/src/channels/slack/delivery/wake.rs); token deltas come from the event bus, see [`live_deltas.rs`](../../crates/server/src/channels/slack/delivery/live_deltas.rs)

### Future Platforms

| Platform | Signing | Threading Model | Notes |
|----------|---------|----------------|-------|
| Discord | Ed25519 | Channel-based threads | Bot gateway + webhook interactions |
| Microsoft Teams | HMAC-SHA256 | Reply chains | Adaptive cards for rich output |
| Telegram | Secret token header | Reply-to threading | Bot API webhook mode |

## Platform Tools

Channel adapters should optionally contribute platform-specific tools via the `Capability` trait:

| Platform | Example Tools |
|----------|---------------|
| Slack | `add_reaction`, `post_to_channel`, `create_thread`, `upload_file` |
| Discord | `create_thread`, `add_reaction`, `pin_message` |
| Teams | `send_adaptive_card`, `create_tab` |

Slack implements platform actions through its native capability and endpoint action service. The neutral posting tool uses that same service; platform-specific affordances remain capability tools. See [Slack Agent Actions](slack-agent-actions.md).

## Files

- `crates/contracts/src/runtime/channel.rs`, All types and traits defined here
- `crates/server/src/domains/apps/record.rs`, `SlackChannelConfig`, `session_strategy: SessionBinding`
- `crates/contracts/src/runtime/conversation/tools.rs`, `send_message` / `no_reply`, the sender contract, and explicit-mode composition
- `crates/core/src/lib.rs`, Module registration and re-exports
- `crates/server/src/messaging/`, Platform-specific webhook handlers and delivery adapters
- `knowledge/integrations/slack-integration.md`, Slack-specific implementation spec
- `knowledge/integrations/messaging-integrations.md`, This spec
