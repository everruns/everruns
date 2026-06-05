---
title: Auto Tool Search
description: Model-adaptive deferred tool loading. Uses OpenAI's hosted tool search where available and a provider-agnostic client-side fallback everywhere else, reducing prompt token usage for agents with many tools.
sidebar:
  order: 89
---

| | |
|---|---|
| **ID** | `auto_tool_search` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

Enables deferred tool loading and automatically picks the best mechanism for the agent's model. On agents with many tools, full parameter schemas are not sent upfront — only names and descriptions — and schemas are loaded on demand. This reduces prompt token usage for agents with 15+ tools, regardless of provider.

This is the recommended default for harnesses that may run on different models. It is what the [Generic](/built-ins/harnesses/generic/) harness uses.

## How It Works

`auto_tool_search` resolves to one of two underlying mechanisms based on the model:

- **Models with native tool search** (OpenAI GPT-5.4 and newer) → the hosted mechanism described in [OpenAI Tool Search](/capabilities/openai-tool-search/): namespaces + `defer_loading` + a `{"type": "tool_search"}` activator. No extra tool is added; the provider handles search server-side.
- **All other models** (Anthropic, Gemini, OpenAI Completions, …) → the client-side mechanism described in [Tool Search](/capabilities/generic-tool-search/): schemas are stripped to stubs and a `tool_search` tool loads them back on demand.

The choice is made when the agent's capabilities are assembled, once the model is known. You don't have to know in advance which provider an agent will use.

## Tools

One — the client-side `tool_search` tool, used only on models without native support. On models with native tool search, no client-side tool is added and the provider's hosted search is used instead.

## Configuration

### Default (threshold: 15)

```json
{
  "capabilities": ["auto_tool_search"]
}
```

### Custom threshold

```json
{
  "capabilities": [
    {
      "capability_ref": "auto_tool_search",
      "config": { "threshold": 10 }
    }
  ]
}
```

The threshold (minimum tool count before deferral activates) applies to both mechanisms. Set to `1` to always activate when the capability is present.

## When to Use a Specific Capability Instead

Prefer the single-mechanism capabilities when you know the model and want explicit behavior:

- [OpenAI Tool Search](/capabilities/openai-tool-search/) (`openai_tool_search`) — hosted only; silently disabled on unsupported models (full schemas sent, no fallback).
- [Tool Search](/capabilities/generic-tool-search/) (`tool_search`) — client-side only; works on any model including OpenAI.

Do not combine `auto_tool_search` with either of the above on the same agent — it already provides both paths.

## See Also

- [OpenAI Tool Search](/capabilities/openai-tool-search/) — the hosted mechanism
- [Tool Search](/capabilities/generic-tool-search/) — the client-side mechanism
- [Capabilities Overview](/capabilities/)
