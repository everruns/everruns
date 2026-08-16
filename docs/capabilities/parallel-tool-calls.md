---
title: Parallel Tool Calls
description: Controls whether the agent requests multiple tool calls per turn and runs them concurrently, prefer parallel, avoid (serialize), or leave the provider default.
sidebar:
  order: 97
---

| | |
|---|---|
| **ID** | `parallel_tool_calls` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |
| **Risk** | Low |

Controls the agent's request-level preference for parallel (multiple per turn)
tool calls, and whether the local tool scheduler runs a batch concurrently.

Most providers emit several tool calls in a single turn by default, and Everruns
runs independent tool calls concurrently. This capability makes that behavior
explicit and configurable: turn it up to actively request batching of
independent reads and searches, or turn it down to force strictly one tool call
at a time.

## Tools

None, this capability only configures the outbound LLM request and the local
tool scheduler.

## How It Works

The capability resolves a `mode` into a request-level preference that threads
through two places:

1. **The LLM request.** On providers that expose a wire control, the preference
   is sent on the request:
   - **OpenAI** (Chat Completions and Responses), and the OpenAI-compatible
     **MAI** and **Fireworks** providers, the top-level `parallel_tool_calls`
     boolean.
   - **OpenRouter**: forwarded on the Responses body; ignored by routed
     providers that do not support it.
   - **Anthropic**: `tool_choice.disable_parallel_tool_use` (sent only when the
     request carries tools).
   - **Gemini** and **Bedrock** have no equivalent request control, so nothing
     is sent. The local scheduler (below) still honors the preference.
2. **The local tool scheduler.** `avoid` forces the scheduler to run the turn's
   tool calls strictly sequentially. This applies to **every** provider, so
   `avoid` is honored even where there is no wire control.

Whether the preference is sent on the wire is gated per provider/model: a driver
that cannot express it omits the field rather than risking an API error.

## Config

```json
{
  "capabilities": [
    {
      "ref": "parallel_tool_calls",
      "config": { "mode": "prefer" }
    }
  ]
}
```

### Modes

| Mode | Provider request | Local scheduler |
|---|---|---|
| `prefer` (default) | Request parallel tool calls where supported | Concurrent (class-aware, the default) |
| `avoid` | Ask for one tool call per turn where supported | Serialized |
| `none` | Omit, provider default | Concurrent (class-aware, the default) |

When the capability is enabled without an explicit `mode`, the default is
`prefer`. `none` is equivalent to not enabling the capability; it is useful to
neutralize a preference inherited from a parent harness.

The **Generic** harness and the built-in **coding** harnesses enable this
capability with `mode: "prefer"` by default.

## Precedence

An explicit `parallel_tool_calls` field set directly on a harness, agent, or
session is a lower-level escape hatch and takes precedence over this capability.

## When To Use

- **`prefer`**: workloads that issue many independent reads or searches per
  turn benefit from batching (faster turns, fewer round-trips).
- **`avoid`**: when tool calls must be observed and applied one at a time, or
  when a model produces lower-quality parallel batches for your workload.

## Limitations

- **Provider gating.** `prefer` only changes the wire request on providers with
  a control for it (OpenAI/Anthropic families). Elsewhere providers already
  parallelize by default, so `prefer` is a no-op on the wire.
- **Durable mode.** A harness/agent-level `mode` other than the default
  (`prefer`) is applied with full fidelity in the in-process runtime; in durable
  worker mode, harness/agent capability config falls back to the default, set
  the mode at the session level, or use the explicit `parallel_tool_calls`
  field, to override durably. (This matches other config-bearing capabilities.)

## See Also

- [Capabilities](/features/capabilities/), the extension model this capability plugs into
- [Agentic Loop](/explanation/agentic-loop/), how the runtime schedules a batch of tool calls
