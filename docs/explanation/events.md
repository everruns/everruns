---
title: Events as the primary store
description: Why Everruns uses an append-only event log as the source of truth for sessions instead of a conventional message table.
sidebar:
  order: 5
---

Most chat systems store messages in a `messages` table and emit events as a side-channel. Everruns inverts this: the **event log is the source of truth**, and messages are derived from events. This page explains why.

## What an event is

An **event** is an immutable record of something that happened in a session. Examples:

- A user submitted a message.
- The LLM produced a token of streaming output.
- A tool was called.
- A tool completed.
- A turn started, completed, failed, or was cancelled.

Every event has a session-local monotonic sequence number, a stable ID (UUID v7), a type in dot notation (`turn.completed`, `output.message.delta`), and a typed payload. Events are appended to the log and never modified or deleted.

The full catalog is in the [Event Reference](/event-reference/).

## Why not a `messages` table?

A conventional design has:

- A `messages` table for the conversation.
- A separate change-feed (Kafka, NATS, a `pg_notify` channel) for real-time updates.
- A traces table for observability.

This gives you three sources of truth that can disagree. When the SSE stream and the messages table disagree, which one is right? When a message is edited mid-stream, what does the change-feed say? When a tool call is observed but no resulting message is persisted, did it happen?

Everruns avoids the question by making the event log authoritative for all three concerns:

- **Conversation history** is reconstructed from events. A `Message` is a projection of a contiguous range of `output.*` and `input.*` events.
- **The real-time stream** is just a tail of the same log. SSE clients receive events as they're appended; resumption with `since_id` is a database read against the same table.
- **Tracing and replay** consume the log directly. Every LLM call, tool execution, and state transition is there.

One log, one ordering, one source of truth.

## Consequences

This design has visible consequences in the API:

- **You can't UPDATE a message.** What you can do is append a new event that supersedes part of the projection. Messages can't be edited because they don't exist as rows.
- **Ordering is by sequence number, not timestamp.** Two events with the same wall-clock time still have a strict order. Timestamps are informational.
- **Resumption is cheap.** `since_id` is a primary-key scan. You can disconnect and reconnect to an SSE stream millions of events later and get exactly the missing tail.
- **Audit is free.** Everything that happened in a session is in one table, in order, immutably.

## Compatibility guarantees

Because events are the API contract, Everruns treats them like a public protocol. The compatibility rules are:

| Change | Allowed |
|---|---|
| Add a new event type | yes |
| Add an optional field to an existing event | yes |
| Add a new enum value to an existing field | yes |
| Remove a field | no, breaking change |
| Change the type of a field | no, breaking change |
| Reuse a sequence number | no, sequence numbers are atomic per session |

Consumers must follow the dual: ignore unknown fields, ignore unknown event types, and treat optional fields as optional. The SDK clients do this automatically.

## What about volume?

A long agent session can produce thousands of events, streaming deltas alone can be hundreds per turn. Two things keep this manageable:

- **Delta events are batched** at ~100ms. The model produces tokens faster than that, but consumers don't need every token as a separate event.
- **Sessions are bounded by intent, not duration.** Even busy sessions rarely exceed five-figure event counts. PostgreSQL handles that easily.

For very long-running sessions, [context compaction](/advanced/compaction/) reduces the *prompt* size but does not modify the event log. The full history remains available via [Infinity Context](/capabilities/infinity-context/) and the event API.

## Further reading

- [Event Reference](/event-reference/), every event type and payload.
- [The agentic loop](/explanation/agentic-loop/), what produces events during a turn.
- [How-to: stream events](/how-to/stream-events/), practical patterns.
