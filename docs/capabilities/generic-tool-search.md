---
title: Tool Search
description: Provider-agnostic deferred tool loading for agents with many tools. Hides tool parameter schemas until the model loads them on demand, reducing prompt token usage on any model.
sidebar:
  order: 91
---

| | |
|---|---|
| **ID** | `tool_search` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

Enables deferred tool loading for agents with many tools, on **any** model. Instead of sending full parameter schemas for every tool upfront, only tool names and descriptions are sent initially. The model loads full schemas on demand by calling the `tool_search` tool.

Unlike [OpenAI Tool Search](/capabilities/openai-tool-search/), which relies on OpenAI's hosted `tool_search` feature, this capability implements tool search entirely client-side. It therefore works with Anthropic, Gemini, OpenAI Completions, and any other provider — not just GPT-5.4+.

## Tools

- **`tool_search`** — search the available tools by keyword and load their full parameter schemas.

## How It Works

1. **Threshold check** — deferral only activates when the total tool count meets or exceeds the threshold (default: 15). Below it, full schemas are sent unchanged.
2. **Schema stripping** — a tool-definition hook replaces the parameter schema of every deferrable tool with a small stub (name + description survive). This runs when the runtime agent is built, so the model never receives the full schemas upfront.
3. **`tool_search` tool** — a real tool is added to the agent. When the model calls it with a query, the tool inspects its sibling tools and returns the full JSON parameter schemas of the matches.
4. **System-prompt guidance** — a short note instructs the model to call `tool_search` before using a tool whose parameters it has not yet loaded.
5. **Transparent execution** — the underlying tools stay registered and executable. Tool calls and results work identically; only how schemas reach the model changes.

### DeferrablePolicy

Each tool has a `deferrable` policy that controls whether its schema can be deferred:

| Policy | Behavior |
|---|---|
| `never` | Full schema always sent (use for high-frequency tools like `write_todos`) |
| `automatic` | Deferred when tool_search is active and above threshold (default) |
| `always` | Always deferred when tool_search is active |

The `tool_search` tool itself is never deferred.

### Model Support

None required. Because deferral and search are implemented client-side, every model works the same way. For GPT-5.4+ you may prefer [OpenAI Tool Search](/capabilities/openai-tool-search/), which uses the provider's hosted index; use this capability for all other models.

## Configuration

```json
{
  "capabilities": ["tool_search"]
}
```

The activation threshold defaults to 15 tools (`DEFAULT_TOOL_SEARCH_THRESHOLD`).

## Limitations

- **Server-executed tools** — the search reads schemas from the worker-side tool registry. This includes built-in tools and MCP server tools (MCP tools are registered as first-class registry tools). Client-side tools that are not registered worker-side are not returned by `tool_search` (their stripped definition is still sent so the model knows they exist).
- **Extra round-trip** — loading a schema costs one `tool_search` call before the first use of a deferred tool. The token savings outweigh this for agents with many tools.

## See Also

- [OpenAI Tool Search](/capabilities/openai-tool-search/) — hosted deferred loading for GPT-5.4+
- [Capabilities Overview](/capabilities/)
