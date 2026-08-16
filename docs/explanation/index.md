---
title: Explanation
description: Background on Everruns, why durable execution, why events are the primary store, how the agentic loop and capability layering fit together.
sidebar:
  order: 0
---

These pages discuss *why* Everruns is shaped the way it is. They don't tell you how to do anything (see [How-to guides](/how-to/)) and they aren't lookup tables (see [Reference](/api/)). They're here to give you a mental model so the other docs make sense.

Read these when:

- You're evaluating Everruns and want to understand the design.
- A reference page tells you *what* something is but you want to know *why* it exists.
- You're about to make an architectural decision and want to know which guarantees you can rely on.

## Topics

- [Core concepts](/explanation/concepts/), the entity model: harnesses, agents, sessions, capabilities, and how they compose into a runtime.
- [The agentic loop](/explanation/agentic-loop/), the reason–act cycle, execution phases, and why turns are bounded.
- [Architecture](/explanation/architecture/), control plane, workers, and the API-first design.
- [Durable execution](/explanation/durable-execution/), why agents survive crashes, and the trade-offs of a PostgreSQL-backed engine.
- [Events as the primary store](/explanation/events/), why an append-only event log is the source of truth, not a side-channel.
