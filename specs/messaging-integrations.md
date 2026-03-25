# Messaging Integrations

## Abstract

Messaging integrations connect agents to external messaging platforms (Slack, Discord, Teams, Telegram). A shared abstraction layer decouples platform-specific protocols from the core runtime: channel adapters translate inbound platform events into `InboundChannelEvent`, route them to sessions via `SessionRoutingStrategy`, and deliver agent output back via `ChannelDeliveryAdapter`. Multi-user threads are tracked via `ThreadContext` with per-message `ExternalActor` attribution.

## Design Decisions

- **ThreadContext is session-level, ExternalActor is message-level**: A session bound to a platform thread accumulates participants in `ThreadContext`. Each individual message carries `ExternalActor` for LLM attribution. This separation lets the agent know both "who's in this conversation" and "who said this specific thing."
- **Async agent invocation is first-class**: The `ChannelDeliveryAdapter` trait models the webhook→ack→async-response pattern. All platforms share the same lifecycle: register delivery on inbound event, deliver output when agent finishes (seconds to hours later), unregister on turn end.
- **Generic session tags**: Session routing uses `{platform}:thread:{ref}`, `{platform}:channel:{id}`, `{platform}:user:{id}` tags. The `build_session_routing_tag()` helper generates these from metadata. Slack's existing tags remain as a concrete instance of this pattern.
- **Reply mode generalized**: `ChannelReplyMode` (`all_messages` | `report_progress_only`) replaces platform-specific reply modes. Progress reporting tags use `channel:reply_mode:*` prefix. Legacy `slack:reply_mode:*` tags remain for backward compat.
- **Platform-contributed tools deferred**: The Capability trait supports `tools()` but no channel adapter contributes tools yet. See "Future: Platform Tools" below.
- **Messaging integrations live in `crates/server/`**: Unlike sandbox/execution integrations (`integrations/`), messaging integrations are deeply coupled to server internals (SessionService, MessageService, EventService, EventNotificationBroadcaster). They are organized as `crates/server/src/messaging/{platform}/` modules with shared orchestration code.

## Types

See `crates/core/src/channel.rs` for full definitions.

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
| `ChannelDeliveryAdapter` | Trait: `deliver()`, `send_ack()`, `format_progress_report()` |
| `DeliveryContext` | Auth token, channel ID, thread ref, reply mode, platform extras |
| `DeliveryResult` | Ok / TransientError / PermanentError |

### Session Routing

| Type | Purpose |
|------|---------|
| `SessionRoutingStrategy` | PerThread (default), PerChannel, PerUser |
| `ChannelReplyMode` | AllMessages (default), ReportProgressOnly |
| `build_session_routing_tag()` | Generates `{platform}:{strategy}:{ref}` session tag |

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
9. Turn ends → unregister delivery
```

## Messaging Integration Parity Requirements

Every messaging integration must ship with the following artifacts. Use Slack as the reference implementation.

| Requirement | Description |
|---|---|
| **SPEC.md** | Co-located spec (`crates/server/specs/{platform}-integration.md`): architecture, webhook flow, security review. |
| **Inbound adapter** | Parse platform webhook into `InboundChannelEvent`. Use `build_session_routing_tag()` for session lookup. Track participants via `ThreadContext`. |
| **Delivery adapter** | Implement `ChannelDeliveryAdapter` trait for outbound message delivery. Handle retry with exponential backoff. |
| **Signing/auth verification** | Platform-specific request authentication (e.g. HMAC signing secret for Slack, Ed25519 for Discord). |
| **Unit tests** | Webhook parsing, signature verification, session tag construction, delivery text extraction, bot message filtering. |
| **Integration tests** | `crates/server/tests/{platform}_integration_test.rs` — webhook→session→message flows against in-memory storage. |
| **Live API tests** | Feature-gated tests against real platform API. Doppler credentials: `TEST_{PLATFORM}_*` vars. |
| **CI: unit tests** | Tests run in the `unit-test` job. |
| **CI: change detection** | Path filter for `{platform}` files. |
| **CI: live-test job** | Dedicated `{platform}-live-test` job, conditional on change detection + `push` event. |
| **User docs** | `docs/integrations/{platform}.md` — setup guide, scopes, session strategies, reply modes. |
| **UI test case** | `test_cases/ui/{platform}_app/TC001_*.md` — manual test for app creation, webhook verification, message flow. |
| **Threat model** | Section in `specs/threat-model.md` covering platform-specific threats (signing bypass, bot loops, replay). |
| **Startup recovery** | Re-register active deliveries after server restart (query sessions with `{platform}:*` tags). |
| **DEV_MODE fallback** | Polling-based delivery when EventNotificationBroadcaster is unavailable (in-memory mode). |

## Code Organization

Messaging integrations live in the server crate, organized by platform:

```
crates/server/src/
  messaging/
    mod.rs              — shared orchestration (generic webhook routing, delivery dispatcher)
    slack/
      mod.rs            — module root, route registration
      webhook.rs        — webhook handler, signing verification
      delivery.rs       — ChannelDeliveryAdapter impl, Slack API client
      types.rs          — Slack-specific types (event envelope, file, attachment)
```

Core abstraction types remain in `crates/core/src/channel.rs`. Platform-specific channel configs (e.g. `SlackChannelConfig`) remain in `crates/core/src/app.rs`. Each `AppChannel` holds its own `channel_type` and `channel_config`, enabling multiple channels per app (e.g. two Slack bots, or Slack + future Discord).

## Concrete Implementations

### Slack

Reference implementation. See [`crates/server/specs/slack-integration.md`](../crates/server/specs/slack-integration.md) for full details.

- Webhook: `POST /v1/apps/{app_id}/slack/events`
- Signing: HMAC-SHA256 via `signing_secret`
- Session strategies: `per_thread`, `per_channel`, `per_user`
- Reply modes: `all_messages`, `report_progress_only`
- Thread context injection via `conversations.replies` API
- Event-driven delivery via `SlackDeliveryAdapter` (implements `ChannelDeliveryAdapter`)
- Startup recovery: re-registers active sessions with `slack:*` tags
- DEV_MODE: falls back to 120s polling

### Future Platforms

| Platform | Signing | Threading Model | Notes |
|----------|---------|----------------|-------|
| Discord | Ed25519 | Channel-based threads | Bot gateway + webhook interactions |
| Microsoft Teams | HMAC-SHA256 | Reply chains | Adaptive cards for rich output |
| Telegram | Secret token header | Reply-to threading | Bot API webhook mode |

## Future: Platform Tools

Channel adapters should optionally contribute platform-specific tools via the `Capability` trait:

| Platform | Example Tools |
|----------|---------------|
| Slack | `add_reaction`, `post_to_channel`, `create_thread`, `upload_file` |
| Discord | `create_thread`, `add_reaction`, `pin_message` |
| Teams | `send_adaptive_card`, `create_tab` |

The plumbing exists (`Capability::tools()` returns `Vec<Box<dyn Tool>>`), but no adapter uses it yet. Tools would receive platform context via `ToolContext` (session access → channel config lookup). **Not implementing now** — recorded as a known gap for future work.

## Files

- `crates/core/src/channel.rs` — All types and traits defined here
- `crates/core/src/app.rs` — `SlackChannelConfig`, `SessionStrategy` (→ `SessionRoutingStrategy`), `SlackReplyMode` (→ `ChannelReplyMode`)
- `crates/core/src/progress_reporting.rs` — Generalized tag handling, backward compat
- `crates/core/src/lib.rs` — Module registration and re-exports
- `crates/server/src/messaging/` — Platform-specific webhook handlers and delivery adapters
- `crates/server/specs/slack-integration.md` — Slack-specific implementation spec
- `specs/messaging-integrations.md` — This spec
