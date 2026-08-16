---
title: Tool Search
description: Provider-agnostic deferred tool loading for agents with many tools. Hides tool parameter schemas until the model loads them on demand, reducing prompt token usage on any model.
sidebar:
  order: 92
---

| | |
|---|---|
| **ID** | `tool_search` |
| **Category** | Optimization |
| **Features** | None |
| **Dependencies** | None |

Enables deferred tool loading for agents with many tools, on **any** model. Instead of sending full parameter schemas for every tool upfront, only tool names and descriptions are sent initially. The model loads full schemas on demand by calling the `tool_search` tool.

Unlike the hosted [OpenAI Tool Search](/capabilities/openai-tool-search/) and [Claude Tool Search](/capabilities/claude-tool-search/), which rely on the provider's server-side `tool_search` feature, this capability implements tool search entirely client-side. It therefore works with Gemini, OpenAI Completions, models reached through gateways that don't implement hosted search, and any other provider, not just GPT-5.4+ or Claude 4. For a default that automatically picks hosted search where available and this client-side path everywhere else, use [Auto Tool Search](/capabilities/auto-tool-search/).

## Tools

- **`tool_search`**: search the available tools by keyword and load their full parameter schemas.

## How It Works

The diagram below traces one deferred tool through a full round-trip, from schema stripping at context-assembly time, through the `tool_search` call, to calling the real tool with its restored parameters.

![Tool Search deferred-loading flow: the Agent Runtime strips parameter schemas to stubs before sending tools to the model; the model calls tool_search, which ranks visible tools in the registry and returns full schemas while recording a session-scoped reveal; on the next iteration the hook restores the revealed tool's registered schema so the model can call it with full parameters.](../images/capabilities/tool-search-flow.svg)

1. **Threshold check**: deferral only activates when the total tool count meets or exceeds the threshold (default: 15). Below it, full schemas are sent unchanged.
2. **Schema stripping**: a tool-definition hook replaces the parameter schema of every deferrable tool with a minimal open-object stub (name + description survive). The shared prompt carries the search instruction once instead of repeating it in every stub. This runs when the runtime agent is built, so the model never receives the full schemas upfront.
3. **`tool_search` tool**: a real tool is added to the agent. When the model calls it with a query, the tool inspects its sibling tools and returns the full JSON parameter schemas of the matches.
4. **Progressive disclosure**: `tool_search` also records the matched tools as *revealed*. The hook re-runs on every reasoning iteration, so on the next step the revealed tools are advertised with their full, authoritative schema on the *registered* definition. This is what lets a structured tool caller actually pass arguments to a previously deferred tool, rather than only reading its schema as text.
5. **System-prompt guidance**: a short note instructs the model to call `tool_search` before using a tool whose parameters it has not yet loaded.
6. **Transparent execution**: the underlying tools stay registered and executable. Tool calls and results work identically; only how schemas reach the model changes.

### DeferrablePolicy

Each tool has a `deferrable` policy that controls whether its schema can be deferred:

| Policy | Behavior |
|---|---|
| `never` | Full schema always sent (use for high-frequency tools like `write_todos`) |
| `automatic` | Deferred when tool_search is active and above threshold (default) |
| `always` | Always deferred when tool_search is active |

The `tool_search` tool itself is never deferred.

### Never-defer allowlist

`DeferrablePolicy::Never` is set by the tool's *owner*. An embedder that composes tools it does not own (for example file/shell tools from another crate) can instead keep specific tools fully loaded by name:

- Programmatically: `ToolSearchCapability::with_never_defer(["read_file", "bash", ...])`.
- By configuration: a `never_defer` array (merged with any programmatic list).

Allowlisted tools behave exactly like `DeferrablePolicy::Never` tools, their full schema is always sent, so the agent is never forced through a `tool_search` round-trip before its first read/edit/shell call.

### Search ranking and result bounding

Because there is no hosted semantic index, `tool_search` ranks matches client-side with a deliberately simple, predictable scheme:

- **Field-weighted keyword overlap**: each whitespace-separated query term scores **3** if it appears in a tool's *name* and **1** if it only appears in the *description*. A name hit is a far stronger signal of intent than an incidental word in prose, so it dominates.
- **Exact-name bonus**: a query that is exactly a tool name gets a large bonus (**+100**), so "load this specific tool" always ranks that tool first. The deferred stub tells the model to query the exact tool name, so this is the common path. Wrapping punctuation is stripped first, so a quoted or backticked name (`"read_file"`, `` `read_file` ``) still matches.
- **Top-band cutoff**: only results scoring at least **half the top score** are returned, trimming weak tail matches so a loose query does not drag in loosely related tools.
- **Result cap**: at most **8** tools are returned per call. Every returned tool is also *revealed* (its full schema is un-deferred for the rest of the session), so the cap bounds both the response payload and how much of the catalogue a single search can permanently un-defer.
- **Visible-tool scoping**: the search only considers tools visible in the current turn (the turn-scoped allowlist), so it never reveals a tool the agent could not otherwise call.
- **No-match fallback**: if nothing matches, the tool returns the catalogue of available tool *names* (not schemas) so the model can refine its query instead of dead-ending. An empty query lists tools so the model can browse.

The session reveal set that drives progressive disclosure is itself bounded: it is keyed per session and evicts the oldest sessions past a fixed cap, so reveals never leak across sessions or grow without limit (an evicted session simply re-runs `tool_search`).

### Model Support

None required. Because deferral and search are implemented client-side, every model works the same way. For GPT-5.4+ you may prefer [OpenAI Tool Search](/capabilities/openai-tool-search/), which uses the provider's hosted index; use this capability for all other models.

## Configuration

```json
{
  "capabilities": ["tool_search"]
}
```

The activation threshold defaults to 15 tools (`DEFAULT_TOOL_SEARCH_THRESHOLD`). Both the threshold and a never-defer allowlist can be set via capability config:

```json
{
  "capabilities": {
    "tool_search": {
      "threshold": 20,
      "never_defer": ["read_file", "write_file", "edit_file", "list_directory", "grep_files", "bash"]
    }
  }
}
```

## Benchmarks

Deferral only touches how tool *parameter schemas* reach the model, names and descriptions still go out in full, so the savings scale with how many tools an agent carries and how rich their schemas are.

Measured on a representative 19-tool generic-agent surface (file, shell, web-fetch, session, storage, todo, time, scheduling, and subagent tools, plus `tool_search` itself), comparing the serialized tool list the driver sends to the model **with and without** deferral on the first turn:

| Metric | Full schemas | Deferred (first turn) | Saving |
|---|---|---|---|
| Tool list sent to model | ~9.1 KB (~2,270 tokens) | ~3.0 KB (~740 tokens) | **67% smaller** |
| Parameter-schema bytes only | ~7.2 KB | ~1.0 KB | **86% smaller** |

Token figures use the ~4-chars-per-token rule of thumb for JSON. 18 of the 19 tools were deferred (`tool_search` keeps its schema). Net savings grow with tool count: an agent with dozens of MCP tools defers proportionally more.

These numbers come from the `benchmark_prompt_size_reduction` test in `crates/builtins/src/tool_search.rs`, which also guards the reduction against regressions. Reproduce them with:

```bash
cargo test -p everruns-builtins --lib benchmark_prompt_size_reduction -- --nocapture
```

The trade-off is one extra `tool_search` round-trip per deferred tool before its first use; for many-tool agents the upfront token savings dominate.

## Limitations

- **Server-executed tools**: the search reads schemas from the worker-side tool registry. This includes built-in tools and MCP server tools (MCP tools are registered as first-class registry tools). Client-side tools that are not registered worker-side are not returned by `tool_search` (their stripped definition is still sent so the model knows they exist).
- **Extra round-trip**: loading a schema costs one `tool_search` call before the first use of a deferred tool. The token savings outweigh this for agents with many tools.

## See Also

- [Auto Tool Search](/capabilities/auto-tool-search/), model-adaptive default (hosted where available, this client-side path elsewhere)
- [OpenAI Tool Search](/capabilities/openai-tool-search/), hosted deferred loading for GPT-5.4+
- [Claude Tool Search](/capabilities/claude-tool-search/), hosted deferred loading for Claude 4+
- [Capabilities Overview](/capabilities/)
