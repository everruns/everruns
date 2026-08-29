---
title: Auto Tool Search
description: Model-adaptive deferred tool loading. Uses the provider's hosted tool search where available (OpenAI or Claude) and a provider-agnostic client-side fallback everywhere else, reducing prompt token usage for agents with many tools.
sidebar:
  order: 89
---

| | |
|---|---|
| **ID** | `auto_tool_search` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

Enables deferred tool loading and automatically picks the best mechanism for the agent's model. On agents with many tools, full parameter schemas are not sent upfront, only names and descriptions, and schemas are loaded on demand. This reduces prompt token usage for agents with 15+ tools, regardless of provider.

This is the recommended default for harnesses that may run on different models. It is what the [Generic](/built-ins/harnesses/generic/) harness uses.

## How It Works

`auto_tool_search` resolves to one of three underlying mechanisms based on the model:

- **Models with native OpenAI tool search** (GPT-5.4 and newer) → the hosted mechanism described in [OpenAI Tool Search](/capabilities/openai-tool-search/): namespaces + `defer_loading` + a `{"type": "tool_search"}` activator. No extra tool is added; the provider handles search server-side.
- **Models with native Claude tool search** (Opus 4, Sonnet 4.5, Haiku 4.5, and Fable 5 and newer) → the hosted mechanism described in [Claude Tool Search](/capabilities/claude-tool-search/): per-tool `defer_loading` + a `tool_search_tool_bm25_20251119` server tool. No extra tool is added; the provider handles search server-side.
- **All other models** (Gemini, OpenAI Completions, Claude/GPT reached via a gateway that doesn't implement the hosted format, …) → the client-side mechanism described in [Tool Search](/capabilities/tool-search/): schemas are stripped to stubs and a `tool_search` tool loads them back on demand.

The choice is made when the agent's capabilities are assembled, once the model is known. You don't have to know in advance which provider an agent will use.

The dispatch looks at the **model id** (matched against the first-party OpenAI/Anthropic profiles), not the transport. In practice that handles the common gateway cases: a Claude model served via Amazon Bedrock or OpenRouter carries a distinct id (`anthropic.claude-…`, `anthropic/claude-…`) that doesn't match the bare first-party profile, so `auto_tool_search` resolves to the client-side mechanism there.

> **Edge case:** if a masked transport presents a *bare* first-party id that does resolve (e.g. a `gpt-5.4` served through an OpenAI-compatible gateway), `auto_tool_search` picks the hosted capability, but the driver then suppresses the hosted wire format for that transport, so full schemas are sent with **no** client-side fallback (a missed optimization, not a failure). If you run a first-party model id through such a gateway, add the [Tool Search](/capabilities/tool-search/) capability explicitly to force client-side deferral.

## Tools

One, the client-side `tool_search` tool, used only on models without native support. On models with native tool search, no client-side tool is added and the provider's hosted search is used instead.

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

- [OpenAI Tool Search](/capabilities/openai-tool-search/) (`openai_tool_search`), hosted, OpenAI only; silently disabled on unsupported models (full schemas sent, no fallback).
- [Claude Tool Search](/capabilities/claude-tool-search/) (`claude_tool_search`), hosted, Claude only; silently disabled on unsupported models (full schemas sent, no fallback).
- [Tool Search](/capabilities/tool-search/) (`tool_search`), client-side only; works on any model including OpenAI and Claude.

Do not combine `auto_tool_search` with any of the above on the same agent, it already provides all three paths.

## See Also

- [OpenAI Tool Search](/capabilities/openai-tool-search/), the hosted mechanism for OpenAI
- [Claude Tool Search](/capabilities/claude-tool-search/), the hosted mechanism for Claude
- [Tool Search](/capabilities/tool-search/), the client-side mechanism
- [Capabilities Overview](/capabilities/)
