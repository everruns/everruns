---
title: Tool Search
description: Deferred tool loading for agents with many tools, using the provider's hosted tool search on OpenAI and Claude models and a client-side search everywhere else.
sidebar:
  order: 89
appliesTo: [framework, platform, cloud]
---

Agents with many tools spend a large part of every prompt on tool parameter
schemas. Tool search sends only tool names and descriptions up front and loads
a tool's full schema when the model needs it. Tool calls and results work the
same way; only how schemas reach the model changes.

Four capabilities provide it:

| ID | Mechanism | Works on | On an unsupported model |
|---|---|---|---|
| `auto_tool_search` | Picks one of the three below per model | Any model | Falls back to client-side |
| `openai_tool_search` | OpenAI hosted tool search | GPT-5.4 and newer (`gpt-5.4*`, `gpt-5.5*`, `gpt-5.6*`) | Does nothing (full schemas sent) |
| `claude_tool_search` | Anthropic hosted tool search | Claude 4 and newer, first-party API | Does nothing (full schemas sent) |
| `tool_search` | Client-side `tool_search` tool | Any model | Not applicable |

Use `auto_tool_search` unless you know the model and want one mechanism
explicitly. It is what the [Generic](/built-ins/harnesses/generic/) harness
uses. Do not combine `auto_tool_search` with any of the other three on the same
agent; it already provides all three paths.

All four capabilities have category Optimization, no features, and no
dependencies.

## Auto tool search

`auto_tool_search` resolves to one of three underlying mechanisms based on the model:

- **Models with native OpenAI tool search** (GPT-5.4 and newer) → the hosted mechanism described in [OpenAI Tool Search](#hosted-openai): namespaces + `defer_loading` + a `{"type": "tool_search"}` activator. No extra tool is added; the provider handles search server-side.
- **Models with native Claude tool search** (Opus 4, Sonnet 4.6, Haiku 4.5, and Fable 5 and newer) → the hosted mechanism described in [Claude Tool Search](#hosted-claude): per-tool `defer_loading` + a `tool_search_tool_bm25_20251119` server tool. No extra tool is added; the provider handles search server-side.
- **All other models** (Gemini, OpenAI Completions, Claude/GPT reached via a gateway that doesn't implement the hosted format, …) → the client-side mechanism described in [Tool Search](#client-side): schemas are stripped to stubs and a `tool_search` tool loads them back on demand.

The choice is made when the agent's capabilities are assembled, once the model is known. You don't have to know in advance which provider an agent will use.

The dispatch looks at the **model id** (matched against the first-party OpenAI/Anthropic profiles), not the transport. In practice that handles the common gateway cases: a Claude model served via Amazon Bedrock or OpenRouter carries a distinct id (`anthropic.claude-…`, `anthropic/claude-…`) that doesn't match the bare first-party profile, so `auto_tool_search` resolves to the client-side mechanism there.

> **Edge case:** if a masked transport presents a *bare* first-party id that does resolve (e.g. a `gpt-5.4` served through an OpenAI-compatible gateway), `auto_tool_search` picks the hosted capability, but the driver then suppresses the hosted wire format for that transport, so full schemas are sent with **no** client-side fallback (a missed optimization, not a failure). If you run a first-party model id through such a gateway, add the [Tool Search](#client-side) capability explicitly to force client-side deferral.

### Tools

One, the client-side `tool_search` tool, added only on models without native
support. On models with native tool search, no tool is added and the provider's
hosted search is used instead.

### Configuration

```json
{
  "capabilities": ["auto_tool_search"]
}
```

The default threshold is 15 tools. To change it:

```json
{
  "capabilities": [
    {
      "ref": "auto_tool_search",
      "config": { "threshold": 10 }
    }
  ]
}
```

The threshold (minimum tool count before deferral activates) applies to every
mechanism. Set it to `1` to always activate when the capability is present.

## Deferrable policy

Each tool has a `deferrable` policy that controls whether its schema can be deferred:

| Policy | Behavior |
|---|---|
| `never` | Full schema always sent (use for high-frequency tools like `write_todos`) |
| `automatic` | Deferred when tool_search is active and above threshold (default) |
| `always` | Always deferred when tool_search is active |

## Hosted: OpenAI

`openai_tool_search` enables [OpenAI's tool_search](https://platform.openai.com/docs/guides/tool-search).
It configures the LLM driver and adds no tool.

1. **Threshold check**: tool_search only activates when the total tool count meets or exceeds the threshold (default: 15)
2. **Namespace grouping**: tools are grouped by their capability's category into [namespace](https://platform.openai.com/docs/api-reference/responses/create#responses-create-tools) entries, giving the model semantic structure for discovery. Wire namespace `name` values are normalized to provider-safe identifiers (for example `File Operations` → `File_Operations`); descriptions keep the human-readable category label
3. **Deferred schemas**: tools marked as deferrable have `defer_loading: true` set, meaning only name + description are sent upfront
4. **`tool_search` entry**: a `{"type": "tool_search"}` activator is appended to the tools array, enabling the model's built-in tool search index
5. **Transparent execution**: tool calls and results work identically; the only difference is how tools are presented to the model

### Model support

Tool search requires model-level support. Currently supported:

| Model family | Supported |
|---|---|
| `gpt-5.6*` | Yes |
| `gpt-5.5*` | Yes |
| `gpt-5.4*` | Yes |
| All other models | No (capability is silently ignored) |

When the capability is enabled but the model doesn't support tool_search, the feature is silently skipped, no errors, no behavior change.

Configure it like `auto_tool_search`, with `"ref": "openai_tool_search"` and an
optional `threshold`.

### Limitations

- **OpenAI only**: tool_search is an OpenAI Responses API feature; other providers ignore this capability.
- **Supported OpenAI models only**: earlier OpenAI models don't support tool_search.
- **No client-side tools**: currently only applies to built-in (server-executed) tools.

See the [OpenAI Responses API tools parameter](https://platform.openai.com/docs/api-reference/responses/create#responses-create-tools) for the namespace and tool_search types.

## Hosted: Claude

`claude_tool_search` enables [Anthropic's hosted tool search](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool).
It configures the LLM driver and adds no tool.

1. **Threshold check**: tool search only activates when the total tool count meets or exceeds the threshold (default: 15). Below the threshold, full schemas are sent as usual.
2. **Deferred schemas**: every deferrable tool gets `defer_loading: true`, so only its name and description reach the model upfront. Anthropic defers each tool individually (there is no namespace grouping).
3. **Hosted search tool**: a `tool_search_tool_bm25_20251119` server tool is added to the request. The model issues a natural-language query against the catalog (tool names, descriptions, argument names, and argument descriptions) and Anthropic returns the 3–5 most relevant tools, expanding them into full definitions inline.
4. **Transparent execution**: the model then calls a discovered tool with a normal `tool_use`; tool calls and results work identically. The only difference is how tools are presented to the model.

Because the hosted search tool is itself never deferred, Anthropic's requirement that *at least one tool be non-deferred* is always satisfied, even when every function tool is deferrable.

Keeping the 3 to 5 most frequently used tools non-deferred (policy `never`)
avoids a search round-trip before the agent's first hot-path call.

### Model support

Tool search requires model-level support. Per Anthropic, it is available on:

| Model family | Supported |
|---|---|
| Opus 5.5 / 5 (`claude-opus-5-5`, `claude-opus-5`) | Yes |
| Opus 4.x (`claude-opus-4*`) | Yes |
| Sonnet 5.5 / 5 (`claude-sonnet-5-5`, `claude-sonnet-5`) | Yes |
| Sonnet 4.6 (`claude-sonnet-4-6`) | Yes |
| Haiku 4.5 (`claude-haiku-4-5`) | Yes |
| Fable 5.1 / 5 (`claude-fable-5-1`, `claude-fable-5`) | Yes |
| Retired pre-4 Claude models | No (capability is silently ignored) |

When the capability is enabled but the model doesn't support tool search, the feature is **silently skipped, full tool schemas are sent as usual**. This standalone capability does *not* add a client-side fallback: on an unsupported model it simply does nothing (no error, no behavior change). For automatic fallback to client-side deferral, use [Auto Tool Search](#auto-tool-search) (or add the [Tool Search](#client-side) capability explicitly).

Claude models reached through a non–first-party transport don't get hosted tool search either, because those transports don't implement the hosted format:

- **Amazon Bedrock**: this integration uses the ConverseStream API; Anthropic's server-side tool search on Bedrock is only available via the InvokeModel API.
- **OpenRouter**: its stateless OpenAI-compatible endpoint accepts but does not implement Anthropic's hosted tool search.

With `claude_tool_search` alone, those transports also send full schemas; pair with [Auto Tool Search](#auto-tool-search) to get client-side deferral there instead.

Configure it like `auto_tool_search`, with `"ref": "claude_tool_search"` and an
optional `threshold`.

## Client-side

`tool_search` implements tool search entirely client-side, so it works with
Gemini, OpenAI Completions, models reached through gateways that don't
implement hosted search, and any other provider.

### Tools

- **`tool_search`**: search the available tools by keyword and load their full parameter schemas. It is never deferred itself.

### How it works

The diagram below traces one deferred tool through a full round-trip, from schema stripping at context-assembly time, through the `tool_search` call, to calling the real tool with its restored parameters.

![Tool Search deferred-loading flow: the Agent Runtime strips parameter schemas to stubs before sending tools to the model; the model calls tool_search, which ranks visible tools in the registry and returns full schemas while recording a session-scoped reveal; on the next iteration the hook restores the revealed tool's registered schema so the model can call it with full parameters.](../images/capabilities/tool-search-flow.svg)

1. **Threshold check**: deferral only activates when the total tool count meets or exceeds the threshold (default: 15). Below it, full schemas are sent unchanged.
2. **Schema stripping**: a tool-definition hook replaces the parameter schema of every deferrable tool with a minimal open-object stub (name + description survive). The shared prompt carries the search instruction once instead of repeating it in every stub. This runs when the runtime agent is built, so the model never receives the full schemas upfront.
3. **`tool_search` tool**: a real tool is added to the agent. When the model calls it with a query, the tool inspects its sibling tools and returns the full JSON parameter schemas of the matches.
4. **Progressive disclosure**: `tool_search` also records the matched tools as *revealed*. The hook re-runs on every reasoning iteration, so on the next step the revealed tools are advertised with their full, authoritative schema on the *registered* definition. This is what lets a structured tool caller actually pass arguments to a previously deferred tool, rather than only reading its schema as text.
5. **System-prompt guidance**: a short note instructs the model to call `tool_search` before using a tool whose parameters it has not yet loaded.
6. **Transparent execution**: the underlying tools stay registered and executable. Tool calls and results work identically; only how schemas reach the model changes.

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

### Configuration

```json
{
  "capabilities": ["tool_search"]
}
```

The activation threshold defaults to 15 tools (`DEFAULT_TOOL_SEARCH_THRESHOLD`). Both the threshold and a never-defer allowlist can be set via capability config:

```json
{
  "capabilities": [
    {
      "ref": "tool_search",
      "config": {
        "threshold": 20,
        "never_defer": ["read_file", "write_file", "edit_file", "list_directory", "grep_files", "bash"]
      }
    }
  ]
}
```

### Benchmarks

Deferral only touches how tool *parameter schemas* reach the model, names and descriptions still go out in full, so the savings scale with how many tools an agent carries and how rich their schemas are.

Measured on a representative 19-tool generic-agent surface (file, shell, web-fetch, session, storage, todo, time, scheduling, and subagent tools, plus `tool_search` itself), comparing the serialized tool list the driver sends to the model **with and without** deferral on the first turn:

| Metric | Full schemas | Deferred (first turn) | Saving |
|---|---|---|---|
| Tool list sent to model | ~9.1 KB (~2,270 tokens) | ~3.0 KB (~740 tokens) | **67% smaller** |
| Parameter-schema bytes only | ~7.2 KB | ~1.0 KB | **86% smaller** |

Token figures use the ~4-chars-per-token rule of thumb for JSON. 18 of the 19 tools were deferred (`tool_search` keeps its schema). Net savings grow with tool count: an agent with dozens of MCP tools defers proportionally more.

These numbers come from the `benchmark_prompt_size_reduction` test in `crates/core/src/builtins/tool_search.rs`, which also guards the reduction against regressions. Reproduce them with:

```bash
cargo test -p everruns-builtins --lib benchmark_prompt_size_reduction -- --nocapture
```

The trade-off is one extra `tool_search` round-trip per deferred tool before its first use; for many-tool agents the upfront token savings dominate.

### Limitations

- **Server-executed tools**: the search reads schemas from the worker-side tool registry. This includes built-in tools and MCP server tools (MCP tools are registered as first-class registry tools). Client-side tools that are not registered worker-side are not returned by `tool_search` (their stripped definition is still sent so the model knows they exist).
- **Extra round-trip**: loading a schema costs one `tool_search` call before the first use of a deferred tool. The token savings outweigh this for agents with many tools.

## See also

- [OpenAI Tool Search documentation](https://platform.openai.com/docs/guides/tool-search)
- [Anthropic tool search tool documentation](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool)
- [Capabilities Overview](/capabilities/)
