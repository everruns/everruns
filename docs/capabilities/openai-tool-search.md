---
title: OpenAI Tool Search
description: Deferred tool loading for agents with many tools, reducing prompt token usage on supported OpenAI models. Tools are loaded on demand based on semantic search.
sidebar:
  order: 90
---

| | |
|---|---|
| **ID** | `openai_tool_search` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

Enables [OpenAI's tool_search](https://platform.openai.com/docs/guides/tool-search) for agents with many tools. Instead of sending full parameter schemas for every tool upfront, only tool names and descriptions are sent initially. The model loads full schemas on-demand when it decides to call a tool.

This reduces prompt token usage significantly for agents with 15+ tools, without changing how tools are called or how results are returned.

## Tools

None, this capability configures the LLM driver, it does not provide tools.

## How It Works

1. **Threshold check**: tool_search only activates when the total tool count meets or exceeds the threshold (default: 15)
2. **Namespace grouping**: tools are grouped by their capability's category into [namespace](https://platform.openai.com/docs/api-reference/responses/create#responses-create-tools) entries, giving the model semantic structure for discovery
3. **Deferred schemas**: tools marked as deferrable have `defer_loading: true` set, meaning only name + description are sent upfront
4. **`tool_search` entry**: a `{"type": "tool_search"}` activator is appended to the tools array, enabling the model's built-in tool search index
5. **Transparent execution**: tool calls and results work identically; the only difference is how tools are presented to the model

### DeferrablePolicy

Each tool has a `deferrable` policy that controls whether its schema can be deferred:

| Policy | Behavior |
|---|---|
| `never` | Full schema always sent (use for high-frequency tools like `write_todos`) |
| `automatic` | Deferred when tool_search is active and above threshold (default) |
| `always` | Always deferred when tool_search is active, regardless of threshold |

### Model Support

Tool search requires model-level support. Currently supported:

| Model family | Supported |
|---|---|
| `gpt-5.4*` | Yes |
| `gpt-5.5*` | Yes |
| All other models | No (capability is silently ignored) |

When the capability is enabled but the model doesn't support tool_search, the feature is silently skipped, no errors, no behavior change.

## Configuration

### Default (threshold: 15)

```json
{
  "capabilities": ["openai_tool_search"]
}
```

### Custom threshold

```json
{
  "capabilities": [
    {
      "capability_ref": "openai_tool_search",
      "config": { "threshold": 10 }
    }
  ]
}
```

Lower thresholds activate tool_search with fewer tools. Set to `1` to always activate when the capability is present.

## Limitations

- **OpenAI-only**: tool_search here is an OpenAI Responses API feature; other providers ignore this capability. For Claude, use [Claude Tool Search](/capabilities/claude-tool-search/); for a provider-adaptive default, use [Auto Tool Search](/capabilities/auto-tool-search/).
- **Supported OpenAI reasoning models only**: earlier OpenAI models don't support tool_search
- **No client-side tools**: currently only applies to built-in (server-executed) tools

## See Also

- [OpenAI Tool Search documentation](https://platform.openai.com/docs/guides/tool-search), official OpenAI guide
- [OpenAI Responses API: tools parameter](https://platform.openai.com/docs/api-reference/responses/create#responses-create-tools), API reference for namespace and tool_search types
- [Claude Tool Search](/capabilities/claude-tool-search/), the equivalent for Claude models
- [Auto Tool Search](/capabilities/auto-tool-search/), model-adaptive default
- [Capabilities Overview](/capabilities/)
