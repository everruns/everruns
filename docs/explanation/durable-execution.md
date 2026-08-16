---
title: Durable execution
description: Why Everruns persists every step of agent execution to PostgreSQL, what guarantees that provides, and the trade-offs of avoiding Temporal-style infrastructure.
sidebar:
  order: 4
---

The word "durable" in "durable agentic harness engine" means a specific thing: **every step of an agent's execution survives a process restart**. This page explains why that matters, how it works, and what it costs.

## The problem

An agent turn looks simple from the outside, send a message, get a streamed response. Internally it's a chain of network calls that each take seconds:

1. Call the LLM provider.
2. Parse tool calls, dispatch them.
3. Wait for each tool to return.
4. Call the LLM again with the results.
5. ...repeat...

Anywhere in that chain, the worker process can crash, the container can be reaped, the network can blip. A naïve implementation loses all the work done so far and forces the user to retry. For a 30-second turn that already burned tokens, this is unacceptable.

## The mechanism

Everruns runs every step as a **durable task**. Each task:

- Has a typed input and output.
- Persists its result to PostgreSQL before acknowledging completion.
- Has retry and timeout policies.
- Heartbeats while running, so the control plane can detect a crashed worker.

A turn is a small state machine over those tasks. The state lives in `durable_workflow_events`, an append-only event log just for the workflow engine. To replay a workflow, you load its events and feed them back into the state machine, same input, same decisions, same output.

When a worker crashes mid-turn, the control plane sees the missed heartbeats, marks the in-flight task as failed, and re-queues it. Another worker picks it up. The application sees a momentary stall in the SSE stream, then it continues.

## Why a custom engine

The obvious alternative is Temporal (or Cadence, or Restate). Everruns deliberately built its own minimal durable engine, `everruns-durable`, instead. The reasoning:

1. **Single dependency.** PostgreSQL is the only stateful infrastructure. Operators don't need to run a second cluster with its own ops story.
2. **Co-located with the rest of the platform.** Workflow events and session events live in the same database, in the same transaction when needed. There's no eventual consistency between "what happened" and "what was reported."
3. **Tight scope.** Everruns runs agentic workflows specifically, limited fan-out, short-to-medium duration, well-understood failure modes. We don't need the full Temporal feature set, and the operational surface area of a tightly-scoped engine is much smaller.

The trade-off: no multi-region replication beyond what PostgreSQL itself offers, no language-agnostic SDK (the engine is Rust-only inside the platform), no visual workflow designer. For agent execution these are not missed.

## Guarantees

What durable execution gives you:

- **No work lost on crash.** If a worker dies, another worker resumes from the last persisted step. Tokens already paid for are not paid for again.
- **Exactly-once tool execution.** Tool calls are persisted by their result, not their attempt. A tool that completed but failed to ack will not be re-run.
- **Deterministic replay.** Reloading a session reproduces the same message sequence, which makes traces and exports authoritative.

What it doesn't give you:

- **Idempotence of side effects.** If your tool POSTs to an external API, the external API will see one call per *successful* execution but a retried-task scenario can still cause duplicates if a tool completes externally and crashes before persisting. Tools that have external side effects must include their own idempotency keys.
- **Real-time latency guarantees.** Persisting every step adds tens of milliseconds per task. For agent workloads (already dominated by LLM latency) this is invisible; for hot-loop workloads it would be costly.

## When the database becomes the bottleneck

Everruns is designed to run on a single PostgreSQL primary. Read replicas help for reporting; write-heavy session loads are handled by partitioning the durable workflow tables by workflow ID hash and by keeping event payloads compact.

For deployments that outgrow a single primary, the migration path is to a sharded PostgreSQL setup keyed by organization, but in practice, LLM provider rate limits cap throughput long before the database does.

## Further reading

- [The agentic loop](/explanation/agentic-loop/), what each step inside a turn looks like.
- [Architecture](/explanation/architecture/), how the control plane and workers interact.
