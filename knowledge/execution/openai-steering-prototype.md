---
type: Specification
title: OpenAI Steering Prototype
description: A bounded experiment for durable user updates on an explicitly owned OpenAI WebSocket connection.
tags:
  - everruns
  - execution
  - openai
---

# OpenAI Steering Prototype

## Why revisit WebSockets

Mid-turn user corrections are a concrete benefit unavailable through the current
HTTP reason activity. The normal worker consumes persisted user-message wakes at
activity boundaries (see [the worker boundary](../../crates/worker/src/unified_worker.rs)
and [durable runner](../../crates/worker/src/durable_runner.rs)). A socket cannot
follow arbitrary activity placement across stateless workers.

The [runnable prototype](../../examples/openai-steering/README.md) therefore has
an explicit, single-host owner process and durable inbox. It does not register a
production provider, alter default models, consume production session messages,
or replace the normal HTTP path. This boundary allows testing the protocol and
failure semantics before changing the distributed runtime contract.

## Ownership and delivery contract

A single owner holds an OS lock for the journal's lifetime and owns one socket,
one default lane, and one response chain. Independent producer processes submit
user updates and completed tool results to SQLite; they never acquire a socket.
Producer retries use stable local message IDs. A paused owner retains its lock,
so another process cannot take over while the old connection remains alive.
This is a same-host worker experiment, not a distributed SQLite deployment.

The owner durably records each submission before sending it. Only one steering
submission awaits commitment at a time; further user updates queue locally.
Acceptance is not application. The successor's creation commits accepted input;
the original may end incomplete because it was steered or complete normally.
Neither event alone completes the logical work while steering is outstanding.
Terminal responses and their usage remain keyed by response ID, so interrupted
parents and successors contribute once each. Raw provider events and final
outputs remain in the private journal for inspection.

Client-owned tools and approvals keep accepted steering queued. The owner waits
for saved results and sends one explicit continuation with the original settings,
without repeating accepted input. Tools execute outside the owner: steering does
not cancel or rerun them. The prototype supports synchronous function/custom
results and explicit MCP approval decisions. Unsupported required input remains
visible and blocked; it must not be guessed or auto-approved.

## Recovery and its irreducible ambiguity

Stored response IDs allow HTTP retrieval after owner failure. A known successor
can be reconciled and its terminal usage recovered before continuing on a new
socket. A supplied successor must belong to the recorded session and parent;
its retrieved user-input history must prove the unresolved update appears exactly
once. A failure event preserves the rejected update and error for a user's next
decision. It does not silently discard or automatically retry invalid input.

There is no steering client idempotency key and no documented lookup of an
unknown successor by parent. A send without acknowledgement, or acceptance
without a durably observed successor, is therefore ambiguous after a disconnect.
Absence from the parent response is not evidence of rejection: the accepted
queue is connection-local. The prototype preserves these intentions as uncertain,
blocks replay, and exposes the need for reconciliation. It deliberately cannot
promise automatic progress through that crash window. This trades availability
for avoiding silent loss or duplicate application. Provider support for successor
discovery or durable steering receipts would materially improve recovery.

## Production adoption bar

A production path needs a separately fenced distributed owner service (or a
long-lived leased activity), durable inbox/outbox operations over the existing
control-plane store, authenticated session-scoped routing from message ingress,
and result/event projection back into the runtime. The existing boundary consumer
must exclude inputs committed by steering, otherwise the next reason activity
would replay them. An owner replacement must reconcile before claiming work;
expiring a lease alone does not prevent the old socket from sending.

Also required: multi-host failure tests, integration with existing tool execution
and approval policy, billing projection across successor responses, bounded event
retention, and a user-visible unresolved-delivery state. These are promotion
requirements, not capabilities claimed by the isolated prototype.

## Evidence and references

[Executable tests](../../examples/openai-steering/test_steering.py) cover event
sequences, saved tool/approval continuation, producer deduplication, ownership,
usage aggregation, and recovery. The example includes an optional billable
[live smoke check](../../examples/openai-steering/live_smoke.py).

Official API contracts verified on 2026-09-05:

- [GPT-6 Astra](https://developers.openai.com/api/docs/guides/latest-model?model=gpt-6-astra)
- [Steering](https://developers.openai.com/api/docs/guides/steering)
- [WebSocket events](https://developers.openai.com/api/reference/resources/responses/websocket-events)
- [WebSocket recovery](https://developers.openai.com/api/docs/guides/websocket-mode#reconnect-and-recover)

The current WebSocket guide supports concurrent named lanes, superseding the
historical single-in-flight-per-connection limitation. This experiment intentionally
keeps a single lane; multiplexing does not solve ownership or ambiguous delivery.
