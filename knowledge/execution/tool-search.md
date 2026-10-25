---
type: Specification
title: "Tool Search Specification"
description: "Legacy and bounded schema disclosure, server-aware native search, and hosted tool-search mechanisms."
tags:
  - everruns
  - execution
  - tool-search
---
# Tool Search Specification

## Abstract

`tool_search` reveals tool schemas after discovery. The generic capability is
provider-agnostic. Its legacy behavior remains the default; bounded catalog
search is opt-in with `bounded: true`. Hosted OpenAI and Anthropic search are
separate provider-specific capabilities. The shared
[tool surface plan](tool-surfaces.md) determines whether operations are native,
CLI, or both; it does not grant execution authority.

## Layering

Tool search operates over operations already in the runtime registry. It does
not decide which MCP server or A2A agent to attach. Resource discovery makes
that choice. A server attached during a session becomes searchable when its
tools enter the next eligible registry view. A deferred MCP placeholder first
loads the server; the agent then searches again for an operation inside it.
See [MCP integrations](../integrations/mcp.md) and
[MCP discovery](../integrations/runtime-mcp.md).

Search metadata is not authority. The bounded path searches only the current
visible-operation set. A result resolves against the current registry, and
execution applies current schema, authorization, approval and network policy.
A prior result or transcript entry cannot authorize a removed or forbidden
operation.

## Mechanisms

The capability registry provides four IDs:

- `tool_search` uses generic client-side discovery and schema reveal.
- `openai_tool_search` uses the OpenAI Responses hosted search format on
  supported OpenAI models and transports.
- `claude_tool_search` uses Anthropic's hosted search tool on supported first-
  party Anthropic transports.
- `auto_tool_search` selects one hosted mechanism when the model profile and
  transport support it, and generic search otherwise.

Hosted modes keep their provider wire behavior. The bounded catalog settings
below apply to generic `tool_search`; they do not change hosted search. A
first-party model ID through a gateway does not establish that the gateway
implements the hosted format. Driver profile masking remains authoritative.
See [OpenAI's guide](https://developers.openai.com/api/docs/guides/tools-tool-search/)
and [Anthropic's tool-use guide](https://platform.claude.com/docs/en/agents-and-tools/tool-use/tool-search-tool).

## Generic search behavior

Without `bounded: true`, the capability retains the legacy generic behavior:
when the configured threshold is reached, eligible schemas are deferred while
tool names and descriptions remain visible; a keyword search returns full
schemas for a small ranked set. The default threshold is 15 tools. Eager,
client-side and explicitly never-defer tools keep their schemas. The legacy
matcher and its catalog presentation are unchanged by the bounded index.

With bounded mode enabled, a registry-revision `ToolSearchIndex` uses BM25-style
lexical ranking over operation names and descriptions, schema property names
and descriptions, and MCP server names and configured descriptions. The index
stores metadata, not duplicate schemas. Server context is scored once per
server, so one large source does not get a vote for every tool it owns. CLI
`tools search` uses the same server-aware index; generic native search uses it
when bounded mode is enabled. Name, description and schema fields use separate
IDF and length normalization so verbose schemas and shared envelope terms cannot
dilute distinctive names. Complete names written with spaces or hyphens resolve
like underscore spellings; canonical MCP namespace separators remain intact.

The DB-backed host preserves attached or preset server descriptions in listed
tools and deferred placeholders, including across the worker transport. A
worker receives retrieval metadata alongside the definitions; it still resolves
execution through current server configuration and authorization.

Search applies the current visible-operation allowlist before ranking. A
server description may help find common operations such as `search`, `get`, or
`list`, while server-qualified tool identities keep same-named operations
separate. A match on server metadata alone returns a server refinement, not
arbitrary operation schemas. The index is invalidated when the runtime registry changes. A
selected name is resolved against that registry before schema disclosure or
execution.

## Bounded mode

`bounded` defaults to `false`. When enabled, omitted settings use:

| Setting | Default | Accepted range | Meaning |
|---|---:|---:|---|
| `max_active_tools` | 32 | 1–128 | Maximum optional schemas revealed in the current native tool view |
| `max_active_bytes` | 65,536 | 1 KiB–1 MiB | Serialized-byte budget for optional active schemas |
| `max_result_bytes` | 32,768 | 1 KiB–64 KiB | Serialized-byte budget for one `tool_search` result |

A bounded search returns at most eight matching schemas, regardless of the
configured byte limit. Eager tools, client-side tools, owner-marked
`DeferrablePolicy::Never` tools, and names in `never_defer` remain in the native
view in addition to the optional schema budgets. Provider compatibility holds
may temporarily retain a removed definition on the wire during a removal-only
warm-prefix transition, but the held wire list must still fit the combined
native tool budget (eager definitions plus the configured optional count and
bytes). If a lower live ceiling no longer admits the held list, the runtime
releases those removals. A held definition is not an execution grant. There is no feature-specific
bound on cumulative schema/help history in the conversation. Inline removal
does not reclaim earlier prompt tokens.

Schemas that do not fit are omitted and counted, with guidance to refine the
query. A source-only query returns a server refinement rather than arbitrary
operation schemas. If no operation matches, the result offers a bounded list
of authorized server sources rather than returning the whole tool inventory. Results carry
the full registered schema and reveal selected operations for the next model
step. Bounded mode requires `reveal: "redefine"`; `reveal: "in_result"` is
rejected because typed calls need the registered schema to become active.

Example:

```json
{
  "ref": "tool_search",
  "config": {
    "bounded": true,
    "max_active_tools": 32,
    "max_active_bytes": 65536,
    "max_result_bytes": 32768
  }
}
```

The native active-schema budgets apply only to optional disclosed schemas; they
do not cap the eager native set. In hybrid mode, eligible operations are also
available through `tools`, which can search, show help and invoke them by
canonical JSON arguments. See [Native and CLI Tool Surfaces](tool-surfaces.md)
and [Tools in Shell](tools-in-shell.md).

## Progressive disclosure and replay

By default, search records selected tool names in transcript-derived disclosure
state. On the next reasoning pass, the definition hook restores their complete
authoritative schemas on registered native tools. Bounded mode enforces its
active count and byte budget when projecting that view. The transcript's
`ToolChanges` record carries the additions/removals used by supported drivers;
replay after worker restart reconstructs the same disclosure state.

The disclosure reader accepts both structured search results and the single
JSON text value reconstructed by DB event storage. Plain text, malformed JSON
and non-object values carry no disclosure state. This keeps the hosted and
in-memory histories equivalent without changing the storage contract for
ordinary tool output.

Provider behavior differs. Some qualified drivers can apply inline tool
changes; fallback drivers may rewrite their full tool list on additions or
schema redefinitions. Removal-only changes can be held while a warm fallback
prefix is in use. No setting guarantees a provider cache hit. Session
compaction and context-window limits are separate from the active optional
schema budget.

The legacy `reveal: "in_result"` experiment leaves the registered stub in place
and asks the model to call with the schema shown in the search result. It is
not permitted in bounded mode; see
[`evals/tool-search-reveal`](../../evals/tool-search-reveal/README.md) for its
model-specific qualification.

## Relationship to Tools in Shell

`tools_in_shell` defaults to `mode: "cli"`, which supersedes tool-search
capabilities and exposes eligible operations through the `tools` command. In
`mode: "hybrid"`, it does not supersede `tool_search`: eligible operations are
available through both routes. Hybrid mode alone leaves native schemas eager;
bounded `tool_search` must be explicitly enabled with `bounded: true` to defer
optional schemas under its configured limits. Owner-required eager schemas
remain visible. The same operation registry and current policy back each route.
See [Tools in Shell](tools-in-shell.md).

## Implementation references

- Generic and bounded search: `crates/core/src/builtins/tool_search.rs`
- Shared search index: `crates/contracts/src/runtime/tool_catalog.rs`
- Surface routes: `crates/contracts/src/runtime/tool_surface.rs`
- Tool disclosure replay: `crates/core/src/engine/execution/reason/tool_changes.rs`
- MCP pagination and bounds: `crates/core/src/mcp/discovery.rs`
- MCP cache: `crates/core/src/host/mcp_cache.rs`
- Canonical evaluation: [`evals/mcp-scale`](../../evals/mcp-scale/README.md)
- Related: [Large MCP Catalogs](large-mcp-catalogs.md),
  [Capabilities](capabilities.md), and [Cache Stable Tool Changes](cache-stable-tool-changes.md)
