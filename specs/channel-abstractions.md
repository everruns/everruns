# Multi-Platform Channel Abstractions

## Abstract

Channel abstractions decouple platform-specific protocols (Slack, Discord, Teams, Telegram) from the core runtime. A channel adapter translates inbound platform events into `InboundChannelEvent`, routes them to sessions via `SessionRoutingStrategy`, and delivers agent output back via `ChannelDeliveryAdapter`. Multi-user threads are tracked via `ThreadContext` with per-message `ExternalActor` attribution.

## Design Decisions

- **ThreadContext is session-level, ExternalActor is message-level**: A session bound to a platform thread accumulates participants in `ThreadContext`. Each individual message carries `ExternalActor` for LLM attribution. This separation lets the agent know both "who's in this conversation" and "who said this specific thing."
- **Async agent invocation is first-class**: The `ChannelDeliveryAdapter` trait models the webhook→ack→async-response pattern. All platforms share the same lifecycle: register delivery on inbound event, deliver output when agent finishes (seconds to hours later), unregister on turn end.
- **Generic session tags**: Session routing uses `{platform}:thread:{ref}`, `{platform}:channel:{id}`, `{platform}:user:{id}` tags. The `build_session_routing_tag()` helper generates these from metadata. Slack's existing tags remain as a concrete instance of this pattern.
- **Reply mode generalized**: `ChannelReplyMode` (`all_messages` | `report_progress_only`) replaces platform-specific reply modes. Progress reporting tags use `channel:reply_mode:*` prefix. Legacy `slack:reply_mode:*` tags remain for backward compat.
- **Platform-contributed tools deferred**: The Capability trait supports `tools()` but no channel adapter contributes tools yet. See "Future: Platform Tools" below.

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

## Migration Path (Slack)

The existing Slack implementation (`slack_events.rs`, `slack_delivery.rs`) continues to work as-is. Migration is incremental:

1. `SlackDeliveryDispatcher` can adopt `ChannelDeliveryAdapter` internally
2. `sync_slack_reply_mode_tags()` now also sets generic `channel:*` tags
3. `session_uses_report_progress()` checks both tag prefixes
4. New platforms implement `ChannelDeliveryAdapter` directly

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
- `crates/core/src/progress_reporting.rs` — Generalized tag handling, backward compat
- `crates/core/src/lib.rs` — Module registration and re-exports
- `specs/channel-abstractions.md` — This spec
