---
title: Core concepts
description: Understand the entity model behind Everruns, harnesses, agents, sessions, capabilities, and how they merge into a runtime.
sidebar:
  order: 1
---

Everruns has five entities that you'll meet over and over: **harness**, **agent**, **session**, **capability**, and **event**. This page explains how they relate and why each exists separately.

For field-by-field schemas, see the [API reference](/api/) and the [Concepts cheat-sheet](/getting-started/concepts/).

## Configuration vs. runtime

The single most useful distinction in Everruns:

| Layer | Entities | Lifetime |
|---|---|---|
| **Configuration** | Harness, Agent, Capability | Long-lived. You author these. |
| **Runtime** | Session, Turn, RuntimeAgent | Created per conversation. The server owns these. |
| **Data** | Event, Message | Append-only log produced during runtime. |

Your application creates **configuration**, starts **runtime**, and consumes **data**. Reference pages that mix these layers, and there are some, read as if everything was one bag of "stuff to set". The three roles never collapse into each other.

## Why three configuration layers (harness, agent, session)?

You could imagine flattening everything into one "agent config" object. Everruns deliberately doesn't.

- A **harness** answers *"what environment am I running in?"*, model defaults, baseline tools, network access. The same harness is reused across many agents.
- An **agent** answers *"what role am I playing?"*, system prompt, domain capabilities, the agent's voice.
- A **session** answers *"what's true for this one conversation?"*, extra tools the user just unlocked, an overridden model, a tighter network policy.

When a session starts, all three layers merge into a single `RuntimeAgent` via an associative fold of overlays. Earlier layers form the base; later layers override or add. System prompts concatenate; network policies can only narrow (allow lists intersect, blocklists union); capabilities are deduplicated by ID.

The motivation: operators control harnesses, app authors control agents, end users (or the runtime) control sessions. Each layer has a different blast radius, and the merge rules reflect that.

## Why capabilities are first-class

Tools could just be free-floating function declarations attached to an agent. Capabilities exist because three concerns travel together:

1. The **tool definition** (name, schema, handler).
2. The **system prompt addition** that teaches the model when to use it.
3. The **session state** the tool needs (mount points in the filesystem, secrets, dependencies on other capabilities).

A capability bundles those three. Enabling `web_fetch` on an agent gives you not just the tool but also the prompt fragment explaining how to use it. Enabling `bashkit_shell` automatically pulls in `session_file_system` because of the declared dependency. Capability ordering is meaningful, earlier capabilities' prompt fragments appear first in the merged system prompt.

MCP servers and skills are also capabilities. They participate in the same merge, dependency resolution, and tool-name prefixing. This keeps the model surface uniform regardless of where a tool came from.

## Sessions are stateful; turns are bounded

A session is a long-lived conversation: it owns an isolated virtual filesystem, a key/value store, and the full event log. Sessions don't terminate, they go `idle` waiting for the next input.

Inside a session, work happens in **turns**. A turn is one cycle of *reason* (call the LLM) and *act* (execute tools), repeated until the model produces a final answer. Turns are bounded, by default capped at 10 iterations, to make runaway loops impossible.

The reason–act loop is described in more detail in [The agentic loop](/explanation/agentic-loop/).

## Messages are derived, events are stored

There is no `messages` table in Everruns. The primary store is the **event log**: an immutable, append-only sequence of records per session. Messages are *reconstructed* from those events when needed.

This sounds backwards until you remember what the application actually wants:

- The UI wants a stream of *deltas* and *tool calls* and *state transitions*, not just finished messages.
- Observability wants a trace of every LLM call and every tool invocation.
- Replay and durability want a deterministic record of what happened.

A message log can't carry that. An event log can, and you can derive a message log from it cheaply. So events come first.

See [Events as the primary store](/explanation/events/) for the consequences.

## Apps connect agents to the outside world

A bare agent has no way to receive messages from users. An **App** wires a harness + agent pair to an inbound channel (such as Slack, webhook, or AG-UI) and exposes a publish/unpublish lifecycle. The App owns the inbound auth, the session-routing strategy (per-thread, per-channel, per-user), and the channel-specific config. For proactive scheduled work, use [agent triggers](/features/agent-triggers/) instead of the deprecated App schedule channel.

This is why apps are a separate concept from agents: the same agent can be deployed to multiple channels, and you want to publish and revoke those deployments independently.
