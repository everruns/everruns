---
title: Claude Tool Search
description: Deferred tool loading for agents with many tools on supported Claude models, reducing prompt token usage. Tools are loaded on demand via Anthropic's hosted tool search.
sidebar:
  order: 91
---

| | |
|---|---|
| **ID** | `claude_tool_search` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

Enables [Anthropic's hosted tool search](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool) for agents with many tools. Instead of sending full parameter schemas for every tool upfront, only tool names and descriptions reach the model initially. The model discovers and loads full schemas on demand by searching the catalog.

This reduces prompt token usage significantly for agents with 15+ tools, without changing how tools are called or how results are returned. It is the Claude counterpart to [OpenAI Tool Search](/capabilities/openai-tool-search/); for a model-adaptive default that picks the right mechanism automatically, use [Auto Tool Search](/capabilities/auto-tool-search/).

## Tools

None, this capability configures the LLM driver, it does not provide tools.

## How It Works

1. **Threshold check**: tool search only activates when the total tool count meets or exceeds the threshold (default: 15). Below the threshold, full schemas are sent as usual.
2. **Deferred schemas**: every deferrable tool gets `defer_loading: true`, so only its name and description reach the model upfront. Anthropic defers each tool individually (there is no namespace grouping).
3. **Hosted search tool**: a `tool_search_tool_bm25_20251119` server tool is added to the request. The model issues a natural-language query against the catalog (tool names, descriptions, argument names, and argument descriptions) and Anthropic returns the 3–5 most relevant tools, expanding them into full definitions inline.
4. **Transparent execution**: the model then calls a discovered tool with a normal `tool_use`; tool calls and results work identically. The only difference is how tools are presented to the model.

Because the hosted search tool is itself never deferred, Anthropic's requirement that *at least one tool be non-deferred* is always satisfied, even when every function tool is deferrable.

### DeferrablePolicy

Each tool has a `deferrable` policy that controls whether its schema can be deferred:

| Policy | Behavior |
|---|---|
| `never` | Full schema always sent (use for high-frequency tools like `write_todos`) |
| `automatic` | Deferred when tool search is active and above threshold (default) |
| `always` | Always deferred when tool search is active, regardless of threshold |

Keeping the 3–5 most frequently used tools non-deferred (via `never`) avoids a search round-trip before the agent's first hot-path call.

### Model Support

Tool search requires model-level support. Per Anthropic, it is available on:

| Model family | Supported |
|---|---|
| Opus 4.x (`claude-opus-4*`) | Yes |
| Sonnet 4.5 / 4.6 (`claude-sonnet-4-5`, `claude-sonnet-4-6`) | Yes |
| Haiku 4.5 (`claude-haiku-4-5`) | Yes |
| Fable 5 (`claude-fable-5`) | Yes |
| Retired pre-4 Claude models | No (capability is silently ignored) |

When the capability is enabled but the model doesn't support tool search, the feature is **silently skipped, full tool schemas are sent as usual**. This standalone capability does *not* add a client-side fallback: on an unsupported model it simply does nothing (no error, no behavior change). For automatic fallback to client-side deferral, use [Auto Tool Search](/capabilities/auto-tool-search/) (or add the [Tool Search](/capabilities/tool-search/) capability explicitly).

Claude models reached through a non–first-party transport don't get hosted tool search either, because those transports don't implement the hosted format:

- **Amazon Bedrock**: this integration uses the ConverseStream API; Anthropic's server-side tool search on Bedrock is only available via the InvokeModel API.
- **OpenRouter**: its stateless OpenAI-compatible endpoint accepts but does not implement Anthropic's hosted tool search.

With `claude_tool_search` alone, those transports also send full schemas; pair with [Auto Tool Search](/capabilities/auto-tool-search/) to get client-side deferral there instead.

## Configuration

### Default (threshold: 15)

```json
{
  "capabilities": ["claude_tool_search"]
}
```

### Custom threshold

```json
{
  "capabilities": [
    {
      "capability_ref": "claude_tool_search",
      "config": { "threshold": 10 }
    }
  ]
}
```

Lower thresholds activate tool search with fewer tools. Set to `1` to always activate when the capability is present.

## Limitations

- **Claude-only**: this is an Anthropic Messages API feature; other providers (OpenAI, Gemini) ignore this capability. Use [Auto Tool Search](/capabilities/auto-tool-search/) for cross-provider agents.
- **Supported Claude models only**: pre-4 Claude models don't support hosted tool search.
- **First-party transport only**: Claude models reached via Bedrock (ConverseStream) or OpenRouter don't get hosted tool search; with this capability alone they send full schemas. Use [Auto Tool Search](/capabilities/auto-tool-search/) for client-side deferral there.
- **No standalone fallback**: on an unsupported model/transport this capability is a no-op (full schemas), not a switch to client-side deferral. Combine with `auto_tool_search`, or add `tool_search`, if you want a fallback.

## See Also

- [Anthropic Tool search tool documentation](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool), official Anthropic guide
- [Auto Tool Search](/capabilities/auto-tool-search/), model-adaptive default (recommended for multi-provider harnesses)
- [OpenAI Tool Search](/capabilities/openai-tool-search/), the equivalent for OpenAI models
- [Tool Search](/capabilities/tool-search/), the provider-agnostic client-side fallback
- [Capabilities Overview](/capabilities/)
