---
type: Specification
title: "Framework Event Listeners and Observability"
description: "Engine-level push listeners for application-facing session events."
tags:
  - everruns
  - framework
  - observability
  - rust
---
# Framework Event Listeners and Observability

## Intent

Applications can register push listeners once on an `everruns::Engine` instead
of polling `Session::events()` for every session. The surface is Alpha. It
exposes `SessionEvent` values only and does not promote core events, host event
buses, persistence stores, or observability-provider implementations.

## Registration

`Engine::builder()` accepts `EventListener` implementations and async closures.
`Engine::new()` remains the zero-listener equivalent of
`Engine::builder().build()`. A listener belongs to the Engine deployment, not
to an Agent snapshot or one Session.

The registration covers events from sessions that the Engine creates, resumes,
or attaches. Child sessions that execute through an Engine session's runtime
use the same observer dispatcher. A parent `Session::events()` stream remains
isolated to its own session.

## Delivery contract

Each listener owns one bounded queue and one ordered drain task. The post-commit
event path performs an immediate enqueue and never waits for listener work. A
slow listener cannot delay another listener or a turn.

The listener's `EventFilter` is evaluated before enqueue. Queue overflow drops
the new delivery, increments the listener's drop counter, and emits a
rate-limited warning. `Engine::observer_stats()` exposes delivery, drop, panic,
and flush state in registration order.

Listener event and flush futures run behind a panic boundary. A panic changes
observer statistics only. It cannot fail a turn, reverse a canonical append, or
stop another listener.

The dispatcher receives the same live post-commit durable and ephemeral events
as the facade event bus. It does not read the event log. Resuming or attaching a
session therefore never replays old events into a listener.

## Shutdown

`Engine::shutdown` stops new observer intake, drains accepted queue entries in
order, and then calls each listener's defaultable `flush()` method. The caller
supplies a deadline duration. Work still running at the deadline is cancelled
and reported through `ObserverReport`.

Dropping the final dispatcher closes its queues without blocking. Applications
that require complete delivery must call `shutdown` explicitly.

## Boundaries

Framework listeners are application callbacks over reviewed `SessionEvent`
projections. Provider integrations that require lossless core events, such as
OpenTelemetry or Braintrust exporters, remain host-owned until a separate
facade integration defines their dependency and global-runtime contracts.

Listener queues are process-local and are not a durable transport. Applications
that require replay recover from the canonical event log through session
history rather than treating observer delivery as persistence.

## Source index

- `crates/everruns/src/engine.rs`
- `crates/everruns/src/observers.rs`
- `crates/everruns/src/events.rs`
- `crates/everruns/src/session.rs`
- `crates/core/src/event_listeners.rs`
