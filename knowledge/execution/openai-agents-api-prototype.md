---
type: Specification
title: OpenAI Agents API Runtime Prototype
description: A bounded evaluation of the managed OpenAI Agents API as an optional Everruns runtime backend.
tags:
  - everruns
  - execution
  - openai
---

# OpenAI Agents API Runtime Prototype

## Decision

The Agents API is a conditional go for an experimental OpenAI-only backend and a
no-go for production adoption today. The managed harness can own the model and
tool loop, durable session, compaction, remote MCP calls, and optional sandbox.
That is useful for applications that choose OpenAI's orchestration contract. It
cannot replace Everruns' provider driver because it owns more than one model
generation and changes durability, approvals, tool execution, cost, and recovery
boundaries.

The [standalone prototype](../../examples/openai-agents-api/README.md) is off by
default and does not register with the worker. It maps a `RuntimeAgent`-shaped
configuration, one client function, and one HTTP MCP server into session
creation. It projects the root turn's output and lifecycle into canonical
Everruns events. Existing session SSE and UI consumers can therefore consume
the projection without a provider-specific rendering path.

## Contract mapping

Everruns' system prompt and selected model map to the managed agent's
instructions and model. Client-side tools map to function definitions. Their
pending calls map to `tool.call_requested`; the application executes each call
and returns `agent.session.input.tool_result` using the provider turn and call
identifiers. An Everruns MCP endpoint maps to an HTTP MCP transport. OpenAI calls
that endpoint directly, so its authentication and network reachability become
provider-facing configuration.

Root turn progress maps to `session.activated`, `turn.started`, streaming output
events, one terminal turn event, and `session.idled` after success. Provider tool
items map to tool timeline events. Provider IDs stay correlation metadata; local
session, turn, and message IDs remain the canonical identities. Subagent turns
must not terminate or overwrite the root turn. Unknown provider events are
ignored, while explicit lifecycle failures and a stream that closes without a
root terminal event fail closed.

## Gaps

OpenAI, not Everruns' durable worker, owns iteration scheduling, compaction, and
recovery. Production integration needs a persisted provider session ID and
event cursor, idempotent input and tool-result outboxes, restart reconciliation
through session retrieval, and a rule for duplicate or missing stream events.
The prototype demonstrates retrieval of pending actions but does not claim
exactly-once recovery across every send/commit ambiguity.

Tool policy is not equivalent. Everruns approvals, guardrails, budgets, network
policy, tool hooks, and capability attribution cannot be bypassed because a
managed harness can call a function or MCP server directly. Production work must
either enforce those policies at the application and MCP boundaries or declare
which capabilities are unsupported. Built-in tools are rejected by the
prototype rather than silently weakening their policy.

Usage projection covers reported model tokens only. OpenAI-hosted container and
tool charges need separate accounting before budget enforcement or cost
reporting can be correct. Managed compaction also changes the context evidence
available to Everruns observability. Session export, deletion, retention, and
import or fork semantics need product decisions before provider-owned state can
be treated as an Everruns session.

## Validation and adoption bar

Deterministic tests exercise the documented HTTP and event shapes without an API
key. A live provider run was not possible during this spike because no
`OPENAI_API_KEY` was available. The protocol evidence is therefore compile-free
and repeatable, but it is not a claim that preview access or billing worked for
the current account.

Production promotion requires live success with function and MCP calls, durable
restart tests, approval and guardrail enforcement, scoped MCP authentication,
cost projection, cancellation, session lifecycle controls, and an explicit
import or fork contract. Until then, keep the backend opt-in, isolated from the
normal worker, and unavailable as a default runtime.

## Official evidence

Verified on 2026-09-29:

- [Agents API overview](https://developers.openai.com/api/docs/guides/agents-api/overview)
- [Run and continue sessions](https://developers.openai.com/api/docs/guides/agents-api/sessions)
- [Events and items](https://developers.openai.com/api/docs/guides/agents-api/sessions/events)
- [Function tools](https://developers.openai.com/api/docs/guides/agents-api/tools/functions)
- [MCP connections](https://developers.openai.com/api/docs/guides/agents-api/tools/mcp)
- [Architecture](https://developers.openai.com/api/docs/guides/agents-api/architecture)
- [Observability and usage](https://developers.openai.com/api/docs/guides/agents-api/observability)
